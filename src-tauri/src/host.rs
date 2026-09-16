use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;

use crate::runtime::{
    AvailableUpdate, CheckResult, LAUNCHER_VERSION, PreparedRuntime, RuntimeManager,
    UpdateSourcePolicy,
};
use crate::{dialogs, paths::AppPaths, process::HarnessProcess};

const WINDOW_LABEL: &str = "dsh";
const EXIT_GAP: &str = "当前 Harness 未提供完整待完成工作、停止接纳和收尾完成证据。运行已保留；如需停止，请从“帮助 → 软件更新 → 高级选项”选择“停止后台服务”。强制停止可能中断任务，不代表正常收尾。";

#[derive(Clone, Default)]
struct HostState {
    process: Arc<Mutex<Option<HarnessProcess>>>,
    busy: Arc<AtomicBool>,
    exiting: Arc<AtomicBool>,
    check: Arc<Mutex<CheckResult>>,
    feedback: Arc<Mutex<String>>,
    status_items: Arc<Mutex<Vec<MenuItem<tauri::Wry>>>>,
    version_menus: Arc<Mutex<Vec<Submenu<tauri::Wry>>>>,
    selections: Arc<Mutex<HashMap<String, AvailableUpdate>>>,
}

#[allow(clippy::missing_panics_doc)]
pub fn run() {
    tauri::Builder::default()
        .plugin(single_instance(|app, _, _| show_window(app)))
        .setup(setup)
        .on_menu_event(|app, event| dispatch(app, event.id().as_ref()))
        .build(tauri::generate_context!())
        .expect("failed to build DSH Desktop")
        .run(handle_run_event);
}

fn manager() -> Result<RuntimeManager> {
    RuntimeManager::new(AppPaths::discover()?)
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let state = HostState::default();
    app.manage(state.clone());
    let window_menu = build_menu(app.handle(), &state)?;
    app.set_menu(window_menu)?;
    let tray_menu = build_menu(app.handle(), &state)?;
    let mut tray = TrayIconBuilder::with_id("dsh-desktop")
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    state.busy.store(true, Ordering::SeqCst);
    let handle = app.handle().clone();
    let initial = state.clone();
    thread::spawn(move || {
        let progress = dialogs::Progress::show(initial.feedback.clone());
        let result = (|| {
            let manager = manager()?;
            let runtime = manager.prepare_current(&|message| feedback(&initial, message))?;
            start_and_present(&handle, &initial, &runtime)
        })();
        drop(progress);
        complete(&initial, result, true);
        check_updates(&handle, true);
    });
    let scheduler = app.handle().clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_hours(6));
            check_updates(&scheduler, true);
        }
    });
    monitor(app.handle().clone(), state);
    Ok(())
}

fn build_menu(app: &AppHandle, state: &HostState) -> tauri::Result<Menu<tauri::Wry>> {
    use tauri::menu::PredefinedMenuItem;
    let item = |id, text| MenuItem::with_id(app, id, text, true, None::<&str>);
    let separator = || PredefinedMenuItem::separator(app);
    let status = MenuItem::with_id(app, "status", "正在启动…", false, None::<&str>)?;
    state.status_items.lock().unwrap().push(status.clone());
    let versions = Submenu::with_id_and_items(app, "versions", "指定版本", true, &[])?;
    state.version_menus.lock().unwrap().push(versions.clone());
    let sources = Submenu::with_id_and_items(
        app,
        "sources",
        "下载来源",
        true,
        &[
            &item("source-auto", "自动选择（推荐）")?,
            &item("source-npm", "官方软件源")?,
            &item("source-local", "本机已下载的版本")?,
            &item("source-oss", "旧版安装缓存")?,
        ],
    )?;
    let advanced = Submenu::with_id_and_items(
        app,
        "advanced",
        "高级选项",
        true,
        &[
            &versions,
            &sources,
            &separator()?,
            &item("info", "诊断信息…")?,
            &item("recover", "恢复应用…")?,
            &item("force-stop", "停止后台服务…")?,
        ],
    )?;
    let updates = Submenu::with_id_and_items(
        app,
        "updates",
        "软件更新",
        true,
        &[
            &status,
            &separator()?,
            &item("download", "下载更新")?,
            &item("activate", "安装已下载的更新…")?,
            &separator()?,
            &advanced,
        ],
    )?;
    let file = Submenu::with_id_and_items(
        app,
        "file",
        "文件(&F)",
        true,
        &[
            &item("hide", "关闭窗口")?,
            &separator()?,
            &item("exit", "退出")?,
        ],
    )?;
    let view = Submenu::with_id_and_items(
        app,
        "view",
        "视图(&V)",
        true,
        &[
            &item("show", "显示主窗口")?,
            &MenuItem::with_id(app, "fullscreen", "全屏", true, Some("F11"))?,
        ],
    )?;
    let help = Submenu::with_id_and_items(
        app,
        "help",
        "帮助(&H)",
        true,
        &[
            &item("check", "检查更新…")?,
            &updates,
            &separator()?,
            &item("about", "关于 DSH Desktop…")?,
        ],
    )?;
    Menu::with_items(app, &[&file, &view, &help])
}

#[allow(clippy::too_many_lines)]
fn dispatch(app: &AppHandle, id: &str) {
    match id {
        "show" => show_window(app),
        "hide" => {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                let _ = window.hide();
            }
        }
        "fullscreen" => {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL)
                && let Ok(fullscreen) = window.is_fullscreen()
            {
                let _ = window.set_fullscreen(!fullscreen);
            }
        }
        "info" | "about" => {
            let state = app.state::<HostState>().inner().clone();
            let diagnostic = id == "info";
            thread::spawn(move || match information(&state, diagnostic) {
                Ok(text) => dialogs::info(
                    if diagnostic {
                        "诊断信息"
                    } else {
                        "关于 DSH Desktop"
                    },
                    text,
                ),
                Err(error) => dialogs::error("DSH Desktop", format!("无法读取版本：{error:#}")),
            });
        }
        "check" => check_updates(app, false),
        "download" => operation(app, |_, state| {
            let _progress = dialogs::Progress::show(state.feedback.clone());
            let target = state
                .check
                .lock()
                .map_err(|_| anyhow::anyhow!("目标状态不可读"))?
                .available
                .clone()
                .context("请先检查并选择具体目标")?;
            let copy = manager()?.stage_target(&target, &|message| feedback(state, message))?;
            feedback(
                state,
                &format!("Harness {} 已准备，等待确认切换", copy.version_label()),
            );
            Ok(())
        }),
        "activate" => operation(app, |app, state| {
            let manager = manager()?;
            let confirmed = manager.read_staged()?.context("没有已准备目标，请先下载")?;
            if !dialogs::confirm(
                "安装更新",
                format!(
                    "安装 DeepSeek Harness {}？\n更新需要重新启动应用。",
                    confirmed.version_label()
                ),
            ) {
                return Ok(());
            }
            require_no_run(state)?;
            let prepared = manager.activate(&confirmed)?;
            start_and_present(app, state, &prepared)
        }),
        "recover" => operation(app, |app, state| {
            {
                let mut slot = state
                    .process
                    .lock()
                    .map_err(|_| anyhow::anyhow!("运行状态不可读"))?;
                if let Some(process) = slot.as_mut() {
                    if !process.tree_ended()? {
                        return present(app, &process.runtime, process.url.clone());
                    }
                    *slot = None;
                }
            }
            let manager = manager()?;
            let runtime = if manager.read_current()?.is_some() {
                manager.repair_current(&|message| feedback(state, message))?
            } else {
                manager.prepare_current(&|message| feedback(state, message))?
            };
            start_and_present(app, state, &runtime)
        }),
        "exit" => operation(app, |app, state| {
            if require_no_run(state).is_err() {
                if !dialogs::confirm(
                    "退出 DSH Desktop",
                    "无法确认所有任务已经完成。\n\n仍要退出吗？强制退出可能中断任务，未保存的内容可能丢失。",
                ) {
                    return Ok(());
                }
                let mut slot = state
                    .process
                    .lock()
                    .map_err(|_| anyhow::anyhow!("暂时无法停止应用，请稍后重试"))?;
                if let Some(process) = slot.as_mut() {
                    process.kill()?;
                }
                *slot = None;
            }
            state.exiting.store(true, Ordering::SeqCst);
            app.exit(0);
            Ok(())
        }),
        "force-stop" | "force-exit" => {
            let exit = id == "force-exit";
            operation(app, move |app, state| {
                if !dialogs::confirm(
                    "强制停止",
                    "强制结束本轮全部受管进程？任务可能中断，收尾结果未知。",
                ) {
                    return Ok(());
                }
                let mut slot = state
                    .process
                    .lock()
                    .map_err(|_| anyhow::anyhow!("运行状态不可读"))?;
                if let Some(process) = slot.as_mut() {
                    process.kill()?;
                }
                *slot = None;
                drop(slot);
                if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                    window.hide()?;
                }
                feedback(
                    state,
                    "受管进程树已结束；正常收尾未知，可同版恢复或确认切换",
                );
                if exit {
                    state.exiting.store(true, Ordering::SeqCst);
                    app.exit(0);
                }
                Ok(())
            });
        }
        id if id.starts_with("target-") => {
            let id = id.to_owned();
            operation(app, move |_, state| {
                let target = state
                    .selections
                    .lock()
                    .map_err(|_| anyhow::anyhow!("版本列表不可读"))?
                    .get(&id)
                    .cloned()
                    .context("版本列表已变化，请重新检查")?;
                let manager = manager()?;
                let mut settings = manager.read_settings()?;
                settings.version = Some(target.version.clone());
                manager.write_settings(&settings)?;
                state
                    .check
                    .lock()
                    .map_err(|_| anyhow::anyhow!("目标状态不可读"))?
                    .available = Some(target.clone());
                feedback(
                    state,
                    &format!("已选择 Harness {}；尚未下载或切换", target.version),
                );
                Ok(())
            });
        }
        id if id.starts_with("source-") => {
            let source = match id {
                "source-npm" => UpdateSourcePolicy::Npm,
                "source-local" => UpdateSourcePolicy::Local,
                "source-oss" => UpdateSourcePolicy::Oss,
                _ => UpdateSourcePolicy::Auto,
            };
            operation(app, move |_, state| {
                let manager = manager()?;
                let mut settings = manager.read_settings()?;
                settings.source = source;
                manager.write_settings(&settings)?;
                *state
                    .check
                    .lock()
                    .map_err(|_| anyhow::anyhow!("检查状态不可读"))? = CheckResult::default();
                feedback(state, "来源已保存；固定版本偏好保持，请检查更新");
                Ok(())
            });
        }
        _ => {}
    }
}

fn reserve(state: &HostState) -> bool {
    state
        .busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}
fn operation(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle, &HostState) -> Result<()> + Send + 'static,
) {
    let state = app.state::<HostState>().inner().clone();
    if !reserve(&state) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        let result = action(&app, &state);
        complete(&state, result, true);
    });
}
fn complete(state: &HostState, result: Result<()>, notify: bool) {
    state.busy.store(false, Ordering::SeqCst);
    if let Err(error) = result {
        feedback(state, &format!("操作未完成：{error:#}"));
        if notify {
            dialogs::error("DSH Desktop", format!("{error:#}"));
        }
    }
}
fn feedback(state: &HostState, text: &str) {
    if let Ok(mut value) = state.feedback.lock() {
        *value = text.into();
    }
    if let Ok(items) = state.status_items.lock() {
        for item in items.iter() {
            let _ = item.set_text(text);
        }
    }
}

fn check_updates(app: &AppHandle, automatic: bool) {
    let state = app.state::<HostState>().inner().clone();
    if !reserve(&state) {
        return;
    }
    thread::spawn(move || {
        let result = (|| {
            feedback(&state, "正在检查官方版本…");
            let manager = manager()?;
            let check = manager.check_for_update()?;
            render_versions(&state, &check)?;
            let summary = check.summary();
            *state
                .check
                .lock()
                .map_err(|_| anyhow::anyhow!("检查状态不可读"))? = check.clone();
            feedback(&state, &summary);
            if automatic {
                if let Some(target) = check.available {
                    let copy =
                        manager.stage_target(&target, &|message| feedback(&state, message))?;
                    feedback(
                        &state,
                        &format!(
                            "Harness {} 已准备，等待确认；{}",
                            copy.version_label(),
                            summary
                        ),
                    );
                }
            } else {
                dialogs::info("检查更新", summary);
            }
            Ok(())
        })();
        complete(&state, result, !automatic);
    });
}

fn render_versions(state: &HostState, check: &CheckResult) -> Result<()> {
    let menus = state
        .version_menus
        .lock()
        .map_err(|_| anyhow::anyhow!("版本菜单不可读"))?;
    let mut selections = state
        .selections
        .lock()
        .map_err(|_| anyhow::anyhow!("版本列表不可读"))?;
    selections.clear();
    for menu in menus.iter() {
        for old in menu.items()? {
            menu.remove(&old)?;
        }
        for (index, target) in check.versions.iter().enumerate() {
            let id = format!("target-{index}");
            let item = MenuItem::with_id(
                menu.app_handle(),
                &id,
                format!("Harness {} · {}", target.version, target.source.label()),
                true,
                None::<&str>,
            )?;
            menu.append(&item)?;
            selections.insert(id, target.clone());
        }
    }
    Ok(())
}

fn information(state: &HostState, diagnostic: bool) -> Result<String> {
    if !diagnostic {
        let running = state
            .process
            .lock()
            .map_err(|_| anyhow::anyhow!("暂时无法读取版本"))?
            .as_ref()
            .map_or_else(
                || "未运行".into(),
                |process| process.runtime.copy.version_label().to_owned(),
            );
        return Ok(format!(
            "DSH Desktop {LAUNCHER_VERSION}\n\nDeepSeek Harness：{running}\n\nDeepSeek Harness 的非官方桌面客户端。\n检查新版本，请选择“帮助 → 检查更新”。"
        ));
    }
    let manager = manager()?;
    let settings = manager.read_settings()?;
    let running = state
        .process
        .lock()
        .map_err(|_| anyhow::anyhow!("运行状态不可读"))?
        .as_ref()
        .map_or_else(
            || "未运行".into(),
            |process| {
                let evidence = process.status().map_or_else(
                    |_| "退出证据未知".into(),
                    |status| {
                        format!(
                            "接纳新工作：{}；待完成工作：{}；收尾完成：{}\n观测到运行中的 agent：{}（不代表全部待完成工作）",
                            known_boolean(status.accepting_new_work),
                            status.pending_work.map_or_else(|| "未知".into(), |value| value.to_string()),
                            known_boolean(status.cleanup_complete),
                            status.observed_running_agents.map_or_else(|| "未知".into(), |value| value.to_string())
                        )
                    },
                );
                format!(
                    "{} · {}\n{}",
                    process.runtime.copy.version_label(),
                    process.runtime.copy.source.label(),
                    evidence
                )
            },
        );
    let staged = manager.read_staged()?.map_or_else(
        || "无".into(),
        |copy| format!("{} · {}", copy.version_label(), copy.source.label()),
    );
    let current = manager
        .read_current()?
        .map_or_else(|| "无".into(), |copy| copy.version_label().to_owned());
    let message = state
        .feedback
        .lock()
        .map_err(|_| anyhow::anyhow!("状态不可读"))?
        .clone();
    Ok(format!(
        "DSH Desktop（非官方宿主）\nLauncher {LAUNCHER_VERSION}\n本轮 Harness：{running}\n当前启用目标：{current}\n已准备、未确认：{staged}\n选择偏好：{} · {}\n\n{message}",
        settings.source.label(),
        settings.version.as_deref().unwrap_or("最新可用版本")
    ))
}

fn known_boolean(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "是",
        Some(false) => "否",
        None => "未知",
    }
}

fn require_no_run(state: &HostState) -> Result<()> {
    let mut slot = state
        .process
        .lock()
        .map_err(|_| anyhow::anyhow!("运行状态未知，不能继续"))?;
    if let Some(process) = slot.as_mut() {
        ensure!(process.tree_ended()?, "{EXIT_GAP}");
        *slot = None;
    }
    Ok(())
}
fn start_and_present(app: &AppHandle, state: &HostState, runtime: &PreparedRuntime) -> Result<()> {
    require_no_run(state)?;
    feedback(
        state,
        &format!("正在启动 Harness {}…", runtime.copy.version_label()),
    );
    let process = HarnessProcess::start(runtime).inspect_err(|_| {
        if let Ok(manager) = manager() {
            let _ = manager.record_forward_repair(
                &runtime.copy.release,
                "start",
                "当前目标启动失败，请同版恢复；用户数据保持",
            );
        }
    })?;
    let url = process.url.clone();
    *state
        .process
        .lock()
        .map_err(|_| anyhow::anyhow!("无法保存运行状态"))? = Some(process);
    present(app, runtime, url).context("Harness 正在运行，但窗口呈现失败；请重新打开窗口")?;
    manager()?.clear_repair()?;
    feedback(
        state,
        &format!(
            "Harness {} 已就绪，窗口已打开",
            runtime.copy.version_label()
        ),
    );
    Ok(())
}
fn present(app: &AppHandle, runtime: &PreparedRuntime, url: url::Url) -> Result<()> {
    let title = format!("DSH Desktop · Harness {}", runtime.copy.version_label());
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<()> {
            if let Some(window) = handle.get_webview_window(WINDOW_LABEL) {
                window.navigate(url)?;
                window.set_title(&title)?;
                window.show()?;
                window.set_focus()?;
            } else {
                WebviewWindowBuilder::new(&handle, WINDOW_LABEL, WebviewUrl::External(url))
                    .title(title)
                    .inner_size(1280.0, 800.0)
                    .visible(true)
                    .build()?;
            }
            Ok(())
        })();
        let _ = tx.send(result);
    })?;
    rx.recv_timeout(Duration::from_secs(15))
        .context("等待窗口呈现超时")?
}
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
fn monitor(app: AppHandle, state: HostState) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(750));
            if !reserve(&state) {
                continue;
            }
            let ended = (|| -> Result<bool> {
                let mut slot = state
                    .process
                    .lock()
                    .map_err(|_| anyhow::anyhow!("运行状态未知"))?;
                if let Some(process) = slot.as_mut() {
                    if process.tree_ended()? {
                        *slot = None;
                        return Ok(true);
                    }
                    if process.try_wait()?.is_some() {
                        if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                            let _ = window.hide();
                        }
                        feedback(
                            &state,
                            "根进程已退出，受管子进程仍存在；等待结束或明确强制停止",
                        );
                    }
                }
                Ok(false)
            })();
            if matches!(ended, Ok(true)) {
                if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                    let _ = window.hide();
                    let _ = window.set_title("DSH Desktop · Harness 未运行");
                }
                feedback(&state, "受管进程树已结束；收尾未知，可选择同版恢复");
            } else if let Err(error) = ended {
                feedback(&state, &format!("进程树状态未知：{error}"));
            }
            state.busy.store(false, Ordering::SeqCst);
        }
    });
}
fn handle_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested {
            api, code: None, ..
        } if !app.state::<HostState>().exiting.load(Ordering::SeqCst) => {
            api.prevent_exit();
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == WINDOW_LABEL => {
            api.prevent_close();
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                let _ = window.hide();
            }
            let state = app.state::<HostState>().inner().clone();
            feedback(&state, "窗口已隐藏，运行保留；完整工作与收尾证据未知");
        }
        _ => {}
    }
}
