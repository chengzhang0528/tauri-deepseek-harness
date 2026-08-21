use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;
use url::Url;

use crate::dialogs;
use crate::paths::AppPaths;
use crate::process::HarnessProcess;
use crate::runtime::{PreparedRuntime, RuntimeManager, StagedRelease};

const WINDOW_LABEL: &str = "dsh";

#[derive(Clone, Default)]
struct HostState {
    process: Arc<Mutex<Option<HarnessProcess>>>,
    shutting_down: Arc<AtomicBool>,
    updating: Arc<AtomicBool>,
    update: Arc<Mutex<UpdateState>>,
    update_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePhase {
    Check,
    Stage,
    Restart,
    Failed,
}

#[derive(Debug, Clone)]
struct UpdateState {
    phase: UpdatePhase,
    available: Option<String>,
    staged: Option<StagedRelease>,
    busy: bool,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            phase: UpdatePhase::Check,
            available: None,
            staged: None,
            busy: false,
        }
    }
}

#[allow(clippy::missing_panics_doc)]
pub fn run() {
    tauri::Builder::default()
        .plugin(single_instance(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(setup)
        .build(tauri::generate_context!())
        .expect("failed to build DSH Desktop")
        .run(handle_run_event);
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let check_item = MenuItem::with_id(app, "check", "Check for updates", true, None::<&str>)?;
    let exit_item = MenuItem::with_id(app, "exit", "Exit DSH Desktop", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&check_item, &exit_item])?;

    TrayIconBuilder::with_id("dsh-desktop")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event
                && let Some(window) = tray.app_handle().get_webview_window(WINDOW_LABEL)
            {
                let _ = window.show();
                let _ = window.set_focus();
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "check" => update_action(app.clone()),
            "exit" => graceful_exit(app.clone()),
            _ => {}
        })
        .build(app)?;

    let state = HostState::default();
    if let Ok(mut item) = state.update_item.lock() {
        *item = Some(check_item.clone());
    }
    app.manage(state.clone());
    let handle = app.handle().clone();
    thread::spawn(move || bootstrap_runtime(handle, state));
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn bootstrap_runtime(app: AppHandle, state: HostState) {
    let result = (|| -> Result<(PreparedRuntime, Option<StagedRelease>)> {
        let paths = AppPaths::discover()?;
        let manager = RuntimeManager::new(paths)?;
        if let Some(repair) = manager.read_repair()? {
            dialogs::error(
                "DSH Desktop",
                format!(
                    "上次运行时更新需要修复（{} / {}）：{}",
                    repair.release, repair.phase, repair.error
                ),
            );
        }
        let staged = manager.read_staged().ok().flatten();
        let prepared = manager.prepare_current()?;
        Ok((prepared, staged))
    })();
    match result {
        Ok((prepared, staged)) => match HarnessProcess::start(&prepared.root) {
            Ok(process) => {
                let url = process.url.clone();
                if let Ok(mut slot) = state.process.lock() {
                    *slot = Some(process);
                }
                if let Some(staged) = staged {
                    if let Ok(mut update) = state.update.lock() {
                        update.available = Some(staged.release.clone());
                        update.staged = Some(staged);
                        update.phase = UpdatePhase::Restart;
                        update.busy = false;
                    }
                    set_update_text(&state, "Restart to activate update");
                }
                monitor_process(app.clone(), state.clone());
                start_update_scheduler(state.clone());
                let main_app = app.clone();
                if let Err(error) = app.run_on_main_thread(move || {
                    if let Err(error) = open_dsh_window(&main_app, url) {
                        dialogs::error("DSH Desktop", format!("无法打开 dsh 页面：{error:#}"));
                    }
                }) {
                    dialogs::error("DSH Desktop", format!("无法切换到 dsh 页面：{error:#}"));
                }
            }
            Err(error) => dialogs::error("DSH Desktop", format!("dsh 启动失败：{error:#}")),
        },
        Err(error) => dialogs::error(
            "DSH Desktop",
            crate::bootstrap::runtime_preparation_message(&error),
        ),
    }
}

fn open_dsh_window(app: &AppHandle, url: Url) -> Result<()> {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window
            .navigate(url)
            .context("cannot navigate existing dsh WebView")?;
        window.show().context("cannot show dsh WebView")?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::External(url))
        .title("DSH Desktop")
        .inner_size(1280.0, 800.0)
        .visible(true)
        .build()
        .context("cannot create dsh WebView")?;
    Ok(())
}

fn update_action(app: AppHandle) {
    let state = app.state::<HostState>().inner().clone();
    let phase = state
        .update
        .lock()
        .ok()
        .map_or(UpdatePhase::Check, |state| state.phase);
    match phase {
        UpdatePhase::Check | UpdatePhase::Failed => run_update_check(state, false),
        UpdatePhase::Stage => stage_update(state, true),
        UpdatePhase::Restart => activate_update(app, state),
    }
}

fn start_update_scheduler(state: HostState) {
    thread::spawn(move || {
        run_update_check(state.clone(), true);
        loop {
            thread::sleep(Duration::from_hours(6));
            run_update_check(state.clone(), true);
        }
    });
}

fn run_update_check(state: HostState, automatic: bool) {
    if !begin_update_work(&state, UpdatePhase::Check) {
        return;
    }
    thread::spawn(move || {
        let result = (|| -> Result<Option<String>> {
            let paths = AppPaths::discover()?;
            RuntimeManager::new(paths)?.check_for_update()
        })();
        match result {
            Ok(Some(version)) => {
                if let Ok(mut update) = state.update.lock() {
                    update.available = Some(version.clone());
                    update.phase = UpdatePhase::Stage;
                    update.busy = false;
                }
                set_update_text(&state, "Download and stage update");
                if automatic {
                    stage_update(state, false);
                } else {
                    dialogs::info("DSH Desktop", format!("发现可用运行时更新：{version}"));
                }
            }
            Ok(None) => {
                if let Ok(mut update) = state.update.lock() {
                    update.available = None;
                    update.staged = None;
                    update.phase = UpdatePhase::Check;
                    update.busy = false;
                }
                set_update_text(&state, "Check for updates");
                if !automatic {
                    dialogs::info("DSH Desktop", "当前已是最新运行时");
                }
            }
            Err(error) => {
                if let Ok(mut update) = state.update.lock() {
                    update.phase = UpdatePhase::Failed;
                    update.busy = false;
                }
                set_update_text(&state, "Retry update check");
                if !automatic {
                    dialogs::error("DSH Desktop", format!("检查更新失败：{error:#}"));
                }
            }
        }
    });
}

fn stage_update(state: HostState, notify: bool) {
    if !begin_update_work(&state, UpdatePhase::Stage) {
        return;
    }
    thread::spawn(move || {
        let result = (|| -> Result<Option<StagedRelease>> {
            let paths = AppPaths::discover()?;
            RuntimeManager::new(paths)?.stage_update()
        })();
        match result {
            Ok(Some(staged)) => {
                let version = staged.release.clone();
                if let Ok(mut update) = state.update.lock() {
                    update.staged = Some(staged);
                    update.available = Some(version.clone());
                    update.phase = UpdatePhase::Restart;
                    update.busy = false;
                }
                set_update_text(&state, "Restart to activate update");
                if notify {
                    dialogs::info(
                        "DSH Desktop",
                        format!("运行时 {version} 已下载并通过 doctor，可从托盘重启激活。"),
                    );
                }
            }
            Ok(None) => {
                if let Ok(mut update) = state.update.lock() {
                    update.phase = UpdatePhase::Check;
                    update.busy = false;
                }
                set_update_text(&state, "Check for updates");
            }
            Err(error) => {
                if let Ok(mut update) = state.update.lock() {
                    update.phase = UpdatePhase::Failed;
                    update.busy = false;
                }
                set_update_text(&state, "Retry update check");
                dialogs::error("DSH Desktop", format!("暂存运行时更新失败：{error:#}"));
            }
        }
    });
}

fn activate_update(app: AppHandle, state: HostState) {
    if !dialogs::confirm(
        "DSH Desktop",
        "运行时更新已准备好。现在停止 dsh、重启并激活吗？",
    ) {
        return;
    }
    if !begin_update_work(&state, UpdatePhase::Restart) {
        return;
    }
    state.updating.store(true, Ordering::SeqCst);
    thread::spawn(move || {
        let paths = match AppPaths::discover() {
            Ok(paths) => paths,
            Err(error) => {
                finish_update_failure(&state, format!("无法定位运行时目录：{error:#}"));
                return;
            }
        };
        let manager = match RuntimeManager::new(paths) {
            Ok(manager) => manager,
            Err(error) => {
                finish_update_failure(&state, format!("无法创建运行时管理器：{error:#}"));
                return;
            }
        };
        if let Err(error) = drain_process(&state) {
            finish_update_failure(&state, format!("激活前无法完成退出 drain：{error:#}"));
            return;
        }
        let prepared = match manager.activate_staged() {
            Ok(prepared) => prepared,
            Err(error) => {
                let release = state
                    .update
                    .lock()
                    .ok()
                    .and_then(|update| update.staged.as_ref().map(|staged| staged.release.clone()))
                    .unwrap_or_else(|| "unknown".into());
                let _ = manager.record_forward_repair(release, "activate", format!("{error:#}"));
                finish_update_failure(&state, format!("激活运行时失败：{error:#}"));
                return;
            }
        };
        let activated_release = prepared
            .root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
            .to_owned();
        match HarnessProcess::start(&prepared.root) {
            Ok(process) => {
                let url = process.url.clone();
                if let Ok(mut slot) = state.process.lock() {
                    *slot = Some(process);
                }
                state.updating.store(false, Ordering::SeqCst);
                if let Ok(mut update) = state.update.lock() {
                    update.phase = UpdatePhase::Check;
                    update.available = None;
                    update.staged = None;
                    update.busy = false;
                }
                set_update_text(&state, "Check for updates");
                monitor_process(app.clone(), state.clone());
                let target = app.clone();
                if let Err(error) = app.run_on_main_thread(move || {
                    if let Err(error) = open_dsh_window(&target, url) {
                        dialogs::error(
                            "DSH Desktop",
                            format!("无法打开更新后的 dsh 页面：{error:#}"),
                        );
                    }
                }) {
                    dialogs::error(
                        "DSH Desktop",
                        format!("无法切换到更新后的 dsh 页面：{error:#}"),
                    );
                }
            }
            Err(error) => {
                let _ =
                    manager.record_forward_repair(activated_release, "start", format!("{error:#}"));
                finish_update_failure(&state, format!("更新后的 dsh 启动失败：{error:#}"));
            }
        }
    });
}

fn begin_update_work(state: &HostState, phase: UpdatePhase) -> bool {
    let Ok(mut update) = state.update.lock() else {
        return false;
    };
    if update.busy {
        return false;
    }
    update.busy = true;
    update.phase = phase;
    drop(update);
    set_update_text(
        state,
        match phase {
            UpdatePhase::Check | UpdatePhase::Failed => "Checking for updates",
            UpdatePhase::Stage => "Downloading and staging update",
            UpdatePhase::Restart => "Activating update",
        },
    );
    true
}

fn set_update_text(state: &HostState, text: &str) {
    let enabled = state.update.lock().ok().is_none_or(|update| !update.busy);
    if let Ok(item) = state.update_item.lock()
        && let Some(item) = item.as_ref()
    {
        let _ = item.set_text(text);
        let _ = item.set_enabled(enabled);
    }
}

fn finish_update_failure(state: &HostState, description: String) {
    state.updating.store(false, Ordering::SeqCst);
    if let Ok(mut update) = state.update.lock() {
        update.phase = UpdatePhase::Failed;
        update.busy = false;
    }
    set_update_text(state, "Retry update check");
    dialogs::error("DSH Desktop", description);
}

fn drain_process(state: &HostState) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut drain_started = false;
    let mut exit_sent = false;
    loop {
        let mut finished = true;
        if let Ok(mut slot) = state.process.lock()
            && let Some(process) = slot.as_mut()
        {
            finished = process.try_wait()?.is_some();
            if !finished {
                if !drain_started {
                    process.begin_drain()?;
                    drain_started = true;
                } else if !exit_sent {
                    let status = process.status()?;
                    ensure!(
                        !status.accepting_new_work,
                        "desktop bridge did not enter drain mode"
                    );
                    if status.active_work == 0 {
                        process.app_exit()?;
                        exit_sent = true;
                    }
                }
            }
        }
        if finished {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("dsh process did not exit before the drain deadline");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn monitor_process(app: AppHandle, state: HostState) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(500));
            let result = state
                .process
                .lock()
                .ok()
                .and_then(|mut slot| slot.as_mut().map(HarnessProcess::try_wait));
            match result {
                Some(Ok(Some(status))) => {
                    if !state.shutting_down.load(Ordering::SeqCst)
                        && !state.updating.load(Ordering::SeqCst)
                    {
                        if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                            let _ = window.hide();
                        }
                        dialogs::error(
                            "DSH Desktop",
                            format!("dsh 运行时已退出（{status}）。请退出后重新启动 DSH Desktop。"),
                        );
                    }
                    break;
                }
                Some(Err(error)) => {
                    if !state.shutting_down.load(Ordering::SeqCst) {
                        dialogs::error("DSH Desktop", format!("无法监控 dsh 运行时：{error:#}"));
                    }
                    break;
                }
                Some(Ok(None)) | None => {}
            }
        }
    });
}

fn graceful_exit(app: AppHandle) {
    let state = app.state::<HostState>().inner().clone();
    if !dialogs::confirm("DSH Desktop", "确认退出？活动任务将先完成 drain。") {
        return;
    }
    drain_and_exit(app, state);
}

fn request_window_close(app: AppHandle, state: HostState) {
    thread::spawn(move || {
        let active = state
            .process
            .lock()
            .ok()
            .and_then(|mut slot| slot.as_mut().and_then(|process| process.status().ok()))
            .map_or(0, |status| status.active_work);
        if active == 0 {
            drain_and_exit(app, state);
        }
    });
}

fn drain_and_exit(app: AppHandle, state: HostState) {
    if state
        .shutting_down
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut drain_started = false;
        let mut exit_sent = false;
        let mut failure: Option<anyhow::Error> = None;
        loop {
            let mut finished = true;
            if let Ok(mut slot) = state.process.lock()
                && let Some(process) = slot.as_mut()
            {
                finished = process.try_wait().ok().flatten().is_some();
                if !finished {
                    if !drain_started {
                        drain_started = true;
                        if let Err(error) = process.begin_drain() {
                            failure = Some(error);
                        }
                    } else if !exit_sent {
                        match process.status() {
                            Ok(status) if status.accepting_new_work => {
                                failure = Some(anyhow::anyhow!(
                                    "desktop bridge did not enter drain mode"
                                ));
                            }
                            Ok(status) if status.active_work == 0 => {
                                if let Err(error) = process.app_exit() {
                                    failure = Some(error);
                                } else {
                                    exit_sent = true;
                                }
                            }
                            Ok(_) => {}
                            Err(error) => failure = Some(error),
                        }
                    }
                }
            }
            if finished || failure.is_some() || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }
        if finished_without_failure(&state, failure.is_none()) {
            app.exit(0);
            return;
        }
        let description = failure.map_or_else(
            || "活动任务仍在运行，是否强制退出？".to_owned(),
            |error| format!("退出 drain 失败：{error:#}\n是否强制退出？"),
        );
        if dialogs::confirm("DSH Desktop", description) {
            if let Ok(mut slot) = state.process.lock()
                && let Some(process) = slot.as_mut()
            {
                let _ = process.kill();
            }
            app.exit(0);
        } else {
            state.shutting_down.store(false, Ordering::SeqCst);
        }
    });
}

fn finished_without_failure(state: &HostState, no_failure: bool) -> bool {
    if !no_failure {
        return false;
    }
    state.process.lock().ok().is_none_or(|mut slot| {
        slot.as_mut()
            .and_then(|process| process.try_wait().ok().flatten())
            .is_some()
    })
}

fn handle_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested {
            api, code: None, ..
        } if !app
            .state::<HostState>()
            .shutting_down
            .load(Ordering::SeqCst) =>
        {
            api.prevent_exit();
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                let _ = window.hide();
            }
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
            request_window_close(app.clone(), app.state::<HostState>().inner().clone());
        }
        _ => {}
    }
}
