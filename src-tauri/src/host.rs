use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;
use url::Url;

use crate::dialogs;
use crate::paths::AppPaths;
use crate::process::{BridgeStatus, HarnessProcess};
use crate::runtime::{PreparedRuntime, RuntimeManager};

const WINDOW_LABEL: &str = "dsh";

#[derive(Clone, Default)]
struct HostState {
    process: Arc<Mutex<Option<HarnessProcess>>>,
    shutting_down: Arc<AtomicBool>,
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
            "check" => check_update(),
            "exit" => graceful_exit(app.clone()),
            _ => {}
        })
        .build(app)?;

    let state = HostState::default();
    app.manage(state.clone());
    let handle = app.handle().clone();
    thread::spawn(move || bootstrap_runtime(handle, state));
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn bootstrap_runtime(app: AppHandle, state: HostState) {
    let result = (|| -> Result<PreparedRuntime> {
        let paths = AppPaths::discover()?;
        RuntimeManager::new(paths)?.prepare_current()
    })();
    match result {
        Ok(prepared) => match HarnessProcess::start(&prepared.root) {
            Ok(process) => {
                let url = process.url.clone();
                if let Ok(mut slot) = state.process.lock() {
                    *slot = Some(process);
                }
                monitor_process(app.clone(), state.clone());
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
        Err(error) => dialogs::error("DSH Desktop", format!("运行时准备失败：{error:#}")),
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

fn check_update() {
    thread::spawn(|| {
        let result = (|| -> Result<Option<String>> {
            let paths = AppPaths::discover()?;
            RuntimeManager::new(paths)?.check_for_update()
        })();
        match result {
            Ok(Some(version)) => {
                dialogs::info("DSH Desktop", format!("发现可用运行时更新：{version}"));
            }
            Ok(None) => dialogs::info("DSH Desktop", "当前已是最新运行时"),
            Err(error) => dialogs::error("DSH Desktop", format!("检查更新失败：{error:#}")),
        }
    });
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
                    if !state.shutting_down.load(Ordering::SeqCst) {
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
                    let status = if !drain_started {
                        drain_started = true;
                        process.begin_drain()
                    } else if exit_sent {
                        Ok(BridgeStatus {
                            accepting_new_work: false,
                            active_work: 1,
                        })
                    } else {
                        process.status()
                    };
                    match status {
                        Ok(status) if status.accepting_new_work => {
                            failure =
                                Some(anyhow::anyhow!("desktop bridge did not enter drain mode"));
                        }
                        Ok(status) if status.active_work == 0 && !exit_sent => {
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
