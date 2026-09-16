use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;

use crate::runtime::{LAUNCHER_VERSION, PreparedRuntime, RuntimeManager, UpdateSourcePolicy};
use crate::update_ui::{UpdateOffer, UpdateView};
use crate::{dialogs, paths::AppPaths, process::HarnessProcess};

const WINDOW_LABEL: &str = "dsh";
const EXIT_GAP: &str = "应用仍在运行。如需强制停止，请选择“帮助 → 故障处理 → 强制停止应用”。这可能中断任务并丢失未保存内容。";

#[derive(Clone, Default)]
struct HostState {
    process: Arc<Mutex<Option<HarnessProcess>>>,
    busy: Arc<AtomicBool>,
    exiting: Arc<AtomicBool>,
    feedback: Arc<Mutex<String>>,
    action_items: Arc<Mutex<Vec<MenuItem<tauri::Wry>>>>,
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
    enable_actions(&state, false);
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
    let item = |id: &str, text| {
        let shortcut = match id {
            "check" => Some("Ctrl+Shift+U"),
            "update-settings" => Some("Ctrl+Comma"),
            _ => None,
        };
        let item = MenuItem::with_id(app, id, text, true, shortcut)?;
        if matches!(
            id,
            "check" | "update-settings" | "recover" | "force-stop" | "exit"
        ) {
            state.action_items.lock().unwrap().push(item.clone());
        }
        Ok::<_, tauri::Error>(item)
    };
    let separator = || PredefinedMenuItem::separator(app);
    let troubleshooting = Submenu::with_id_and_items(
        app,
        "troubleshooting",
        "故障处理",
        true,
        &[
            &item("info", "诊断信息…")?,
            &item("recover", "启动或修复当前版本…")?,
            &separator()?,
            &item("force-stop", "强制停止应用…")?,
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
            &item("update-settings", "更新设置…")?,
            &separator()?,
            &troubleshooting,
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
        "update-settings" => operation(app, |_, state| update_settings(state)),
        "recover" => operation(app, |app, state| {
            {
                let mut slot = state
                    .process
                    .lock()
                    .map_err(|_| anyhow::anyhow!("运行状态不可读"))?;
                if let Some(process) = slot.as_mut() {
                    if !process.tree_ended()? {
                        dialogs::info(
                            "应用正在运行",
                            "当前版本无需修复。若窗口未显示，请选择“视图 → 显示主窗口”。",
                        );
                        return present(app, &process.runtime, process.url.clone());
                    }
                    *slot = None;
                }
            }
            if !dialogs::action(
                "启动或修复当前版本",
                "将启动当前版本；仅在程序文件缺失时重新下载同一版本。保留配置和会话。",
                "启动当前版本",
            ) {
                return Ok(());
            }
            let _progress =
                dialogs::Progress::with_heading(state.feedback.clone(), "正在启动当前版本");
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
                if !dialogs::action(
                    "退出 DSH Desktop",
                    "无法确认所有任务已经完成。\n\n仍要退出吗？强制退出可能中断任务，未保存的内容可能丢失。",
                    "强制退出",
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
                if !dialogs::action(
                    "强制停止应用",
                    "将结束应用及其后台任务，窗口也会关闭。正在进行的任务可能中断，未保存内容可能丢失。",
                    "强制停止",
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
    enable_actions(&state, false);
    let app = app.clone();
    thread::spawn(move || {
        let result = action(&app, &state);
        complete(&state, result, true);
    });
}
fn complete(state: &HostState, result: Result<()>, notify: bool) {
    state.busy.store(false, Ordering::SeqCst);
    enable_actions(state, true);
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
}

fn enable_actions(state: &HostState, enabled: bool) {
    let running = state.process.lock().map_or(true, |slot| slot.is_some());
    if let Ok(items) = state.action_items.lock() {
        for item in items.iter() {
            let applicable = match item.id().as_ref() {
                "recover" => !running,
                "force-stop" => running,
                _ => true,
            };
            let _ = item.set_enabled(enabled && applicable);
        }
    }
}

fn check_updates(app: &AppHandle, automatic: bool) {
    let state = app.state::<HostState>().inner().clone();
    if !reserve(&state) {
        return;
    }
    enable_actions(&state, false);
    let app = app.clone();
    thread::spawn(move || {
        let result = if automatic {
            (|| {
                let manager = manager()?;
                let check = manager.check_for_update()?;
                feedback(&state, &check.summary());
                if let Some(target) = check.available {
                    let copy =
                        manager.stage_target(&target, &|message| feedback(&state, message))?;
                    feedback(
                        &state,
                        &format!(
                            "{} 已下载；选择“帮助 → 检查更新”查看并安装。",
                            copy.version_label()
                        ),
                    );
                }
                Ok(())
            })()
        } else {
            show_updates(&app, &state)
        };
        complete(&state, result, !automatic);
    });
}

fn running_version(state: &HostState) -> Result<String> {
    Ok(state
        .process
        .lock()
        .map_err(|_| anyhow::anyhow!("暂时无法读取版本"))?
        .as_ref()
        .map_or_else(
            || "未运行".into(),
            |process| process.runtime.copy.version_label().to_owned(),
        ))
}

fn show_updates(app: &AppHandle, state: &HostState) -> Result<()> {
    let manager = manager()?;
    loop {
        feedback(state, "正在检查更新，请稍候…");
        let check = {
            let _progress = dialogs::Progress::with_heading(state.feedback.clone(), "正在检查更新");
            manager.check_for_update()?
        };
        let view = UpdateView::new(
            &running_version(state)?,
            manager.read_current()?.as_ref(),
            manager.read_staged()?.as_ref(),
            &check,
            &manager.read_settings()?,
        );
        feedback(state, view.heading);
        if let Some(offer) = view.offer {
            if dialogs::action(view.heading, &view.description, offer.label()) {
                update_now(app, state, offer)?;
            }
        } else if !check.issues.is_empty() {
            if dialogs::action(view.heading, &view.description, "重新检查") {
                continue;
            }
        } else {
            dialogs::notice(view.heading, &view.description);
        }
        return Ok(());
    }
}

/// The visible action owns the exact target across download and restart confirmation.
fn update_now(app: &AppHandle, state: &HostState, offer: UpdateOffer) -> Result<()> {
    let manager = manager()?;
    let confirmed = match offer {
        UpdateOffer::Install(copy) => copy,
        UpdateOffer::Download(target) => {
            feedback(state, &format!("正在下载 Harness {}…", target.version));
            let _progress = dialogs::Progress::with_heading(state.feedback.clone(), "正在下载更新");
            manager.stage_target(&target, &|message| feedback(state, message))?
        }
    };
    manager.prepare_for_start(&confirmed)?;
    let running = {
        let mut slot = state
            .process
            .lock()
            .map_err(|_| anyhow::anyhow!("暂时无法确认应用状态，请稍后重试"))?;
        match slot.as_mut() {
            Some(process) => !process.tree_ended()?,
            None => false,
        }
    };
    let description = if running {
        format!(
            "将更新到 DeepSeek Harness {}。\n\n无法确认所有任务已经完成。强制重启可能中断任务或丢失未保存内容。请先完成正在进行的工作。\n\n取消会保留当前运行和已下载更新。",
            confirmed.version_label()
        )
    } else {
        format!(
            "将安装并打开 DeepSeek Harness {}。\n\n取消会保留已下载更新。",
            confirmed.version_label()
        )
    };
    let label = if running {
        "强制重启并更新"
    } else {
        "安装并打开"
    };
    if !dialogs::action("更新已下载", &description, label) {
        feedback(
            state,
            &format!(
                "{} 已下载；稍后从“帮助 → 检查更新”继续安装。",
                confirmed.version_label()
            ),
        );
        return Ok(());
    }
    feedback(state, "正在重启并安装更新…");
    let progress = dialogs::Progress::with_heading(state.feedback.clone(), "正在安装更新");
    {
        let mut slot = state
            .process
            .lock()
            .map_err(|_| anyhow::anyhow!("暂时无法停止应用，请稍后重试"))?;
        if let Some(process) = slot.as_mut() {
            process.kill()?;
        }
        *slot = None;
    }
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.hide()?;
    }
    let prepared = manager.activate(&confirmed)?;
    start_and_present(app, state, &prepared)?;
    drop(progress);
    dialogs::notice(
        "更新完成",
        &format!("当前运行：DeepSeek Harness {}。", confirmed.version_label()),
    );
    Ok(())
}

fn update_settings(state: &HostState) -> Result<()> {
    let manager = manager()?;
    loop {
        let mut settings = manager.read_settings()?;
        let description = format!(
            "更新来源：{}\n版本选择：{}\n\n设置只影响以后检查的范围，保存不会下载或重启。",
            settings.source.label(),
            settings.version.as_deref().unwrap_or("自动选择最新版本")
        );
        match dialogs::commands(
            "更新设置",
            &description,
            &["更新来源…".into(), "目标版本…".into()],
        ) {
            Some(0) => {
                let sources = [
                    UpdateSourcePolicy::Auto,
                    UpdateSourcePolicy::Npm,
                    UpdateSourcePolicy::Local,
                    UpdateSourcePolicy::Oss,
                ];
                let labels = [
                    "自动选择（推荐，包含官方软件源）",
                    "官方软件源（联网）",
                    "仅本机已下载的版本",
                    "仅旧版安装缓存（不联网）",
                ]
                .map(str::to_owned);
                let selected = sources
                    .iter()
                    .position(|source| *source == settings.source)
                    .unwrap_or(0);
                if let Some(index) = dialogs::select(
                    "更新来源",
                    "选择检查更新时使用的来源。目标版本设置会保持不变。",
                    &labels,
                    selected,
                ) {
                    settings.source = sources[index];
                    manager.write_settings(&settings)?;
                }
            }
            Some(1) => select_version(&manager, state, &mut settings)?,
            _ => return Ok(()),
        }
    }
}

fn select_version(
    manager: &RuntimeManager,
    state: &HostState,
    settings: &mut crate::runtime::RuntimeUpdateSettings,
) -> Result<()> {
    feedback(state, "正在获取可选版本…");
    let check = {
        let _progress = dialogs::Progress::with_heading(state.feedback.clone(), "正在获取版本");
        manager.check_for_update()?
    };
    let current = manager.read_current()?;
    let mut versions = Vec::new();
    if let Some(version) = &settings.version {
        versions.push(version.clone());
    }
    for target in check.versions {
        if current
            .as_ref()
            .and_then(|copy| copy.harness_version.as_ref())
            .is_some_and(|version| {
                crate::official::compare_versions(&target.version, version).is_lt()
            })
        {
            continue;
        }
        if !versions.contains(&target.version) {
            versions.push(target.version);
        }
    }
    let mut labels = vec!["自动选择最新版本（推荐）".to_owned()];
    labels.extend(versions.iter().map(|version| format!("固定为 {version}")));
    let selected = settings
        .version
        .as_ref()
        .and_then(|version| versions.iter().position(|value| value == version))
        .map_or(0, |index| index + 1);
    let mut description =
        "固定版本后将不再提示更高版本；选择自动可恢复检查新版本。保存不会安装或重启。".to_owned();
    if !check.issues.is_empty() {
        let _ = write!(
            description,
            "\n\n版本列表可能不完整：{}",
            check.issues.join("；")
        );
    }
    if let Some(index) = dialogs::select("目标版本", &description, &labels, selected) {
        settings.version = index.checked_sub(1).map(|index| versions[index].clone());
        manager.write_settings(settings)?;
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
                enable_actions(&state, true);
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
