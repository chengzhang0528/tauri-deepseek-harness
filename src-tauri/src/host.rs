use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;
use url::Url;

use crate::dialogs;
use crate::paths::AppPaths;
use crate::process::HarnessProcess;
use crate::runtime::{
    AvailableUpdate, PreparedRuntime, RuntimeManager, RuntimeSource, StagedRelease,
    UpdateSourcePolicy,
};

const WINDOW_LABEL: &str = "dsh";

type SourceMenuItems = Vec<(UpdateSourcePolicy, CheckMenuItem<tauri::Wry>)>;

fn source_label(source: RuntimeSource) -> &'static str {
    match source {
        RuntimeSource::Local => "本地",
        RuntimeSource::Oss => "OSS",
        RuntimeSource::Npm => "npm",
    }
}

fn source_policy_label(source: UpdateSourcePolicy) -> &'static str {
    match source {
        UpdateSourcePolicy::Auto => "自动",
        UpdateSourcePolicy::Local => "本地",
        UpdateSourcePolicy::Oss => "OSS",
        UpdateSourcePolicy::Npm => "npm",
    }
}

fn source_policy_id(source: UpdateSourcePolicy) -> &'static str {
    match source {
        UpdateSourcePolicy::Auto => "runtime-source-auto",
        UpdateSourcePolicy::Local => "runtime-source-local",
        UpdateSourcePolicy::Oss => "runtime-source-oss",
        UpdateSourcePolicy::Npm => "runtime-source-npm",
    }
}

fn source_policy_from_id(id: &str) -> Option<UpdateSourcePolicy> {
    [
        UpdateSourcePolicy::Auto,
        UpdateSourcePolicy::Local,
        UpdateSourcePolicy::Oss,
        UpdateSourcePolicy::Npm,
    ]
    .into_iter()
    .find(|source| source_policy_id(*source) == id)
}

#[derive(Clone, Default)]
struct HostState {
    process: Arc<Mutex<Option<HarnessProcess>>>,
    shutting_down: Arc<AtomicBool>,
    updating: Arc<AtomicBool>,
    update: Arc<Mutex<UpdateState>>,
    update_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    source_items: Arc<Mutex<SourceMenuItems>>,
    version_menu: Arc<Mutex<Option<Submenu<tauri::Wry>>>>,
    version_items: Arc<Mutex<Vec<MenuItem<tauri::Wry>>>>,
    version_selections: Arc<Mutex<HashMap<String, (RuntimeSource, String)>>>,
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
    available: Option<AvailableUpdate>,
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
    let source_policies = [
        UpdateSourcePolicy::Auto,
        UpdateSourcePolicy::Local,
        UpdateSourcePolicy::Oss,
        UpdateSourcePolicy::Npm,
    ];
    let source_items = source_policies
        .into_iter()
        .map(|source| {
            CheckMenuItem::with_id(
                app,
                source_policy_id(source),
                source_policy_label(source),
                true,
                source == UpdateSourcePolicy::Auto,
                None::<&str>,
            )
            .map(|item| (source, item))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source_refs = source_items
        .iter()
        .map(|(_, item)| item as &dyn tauri::menu::IsMenuItem<_>)
        .collect::<Vec<_>>();
    let source_menu = Submenu::with_id_and_items(
        app,
        "runtime-source-menu",
        "Runtime source",
        true,
        &source_refs,
    )?;
    let version_placeholder = MenuItem::with_id(
        app,
        "runtime-version-loading",
        "正在检查可用版本…",
        false,
        None::<&str>,
    )?;
    let version_menu = Submenu::with_id_and_items(
        app,
        "runtime-version-menu",
        "Runtime version",
        true,
        &[&version_placeholder],
    )?;
    let check_item = MenuItem::with_id(app, "check", "Check for updates", true, None::<&str>)?;
    let exit_item = MenuItem::with_id(app, "exit", "Exit DSH Desktop", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&source_menu, &version_menu, &check_item, &exit_item])?;

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
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_owned();
            match id.as_str() {
                "check" => update_action(app.clone()),
                "exit" => graceful_exit(app.clone()),
                _ => {
                    if let Some(source) = source_policy_from_id(&id) {
                        select_runtime_source(app, source);
                    } else if id.starts_with("runtime-version-") {
                        select_runtime_version(app, &id);
                    }
                }
            }
        })
        .build(app)?;

    let state = HostState::default();
    if let Ok(mut item) = state.update_item.lock() {
        *item = Some(check_item.clone());
    }
    if let Ok(mut items) = state.source_items.lock() {
        *items = source_items;
    }
    if let Ok(mut menu_slot) = state.version_menu.lock() {
        *menu_slot = Some(version_menu);
    }
    if let Ok(mut items) = state.version_items.lock() {
        *items = vec![version_placeholder];
    }
    app.manage(state.clone());
    let handle = app.handle().clone();
    refresh_runtime_versions(state.clone());
    thread::spawn(move || bootstrap_runtime(handle, state));
    Ok(())
}

fn runtime_source_policy(source: RuntimeSource) -> UpdateSourcePolicy {
    match source {
        RuntimeSource::Local => UpdateSourcePolicy::Local,
        RuntimeSource::Oss => UpdateSourcePolicy::Oss,
        RuntimeSource::Npm => UpdateSourcePolicy::Npm,
    }
}

fn source_policy_accepts(policy: UpdateSourcePolicy, source: RuntimeSource) -> bool {
    matches!(
        (policy, source),
        (UpdateSourcePolicy::Auto, _)
            | (UpdateSourcePolicy::Local, RuntimeSource::Local)
            | (UpdateSourcePolicy::Oss, RuntimeSource::Oss)
            | (UpdateSourcePolicy::Npm, RuntimeSource::Npm)
    )
}

fn refresh_source_checks(state: &HostState, selected: UpdateSourcePolicy) {
    if let Ok(items) = state.source_items.lock() {
        for (source, item) in items.iter() {
            let _ = item.set_checked(*source == selected);
        }
    }
}

fn refresh_runtime_versions(state: HostState) {
    thread::spawn(move || {
        let result = (|| -> Result<_> {
            let paths = AppPaths::discover()?;
            let manager = RuntimeManager::new(paths)?;
            let settings = manager.read_settings()?;
            let versions = manager.list_available_versions()?;
            refresh_source_checks(&state, settings.source);
            Ok((versions, settings.source, settings.version))
        })();
        render_runtime_versions(&state, result);
    });
}

fn render_runtime_versions(
    state: &HostState,
    result: Result<(Vec<AvailableUpdate>, UpdateSourcePolicy, Option<String>)>,
) {
    let Ok(menu) = state.version_menu.lock() else {
        return;
    };
    let Some(menu) = menu.as_ref() else {
        return;
    };
    let old_items = state
        .version_items
        .lock()
        .map(|mut items| std::mem::take(&mut *items))
        .unwrap_or_default();
    for item in old_items {
        let _ = menu.remove(&item);
    }
    if let Ok(mut selections) = state.version_selections.lock() {
        selections.clear();
    }

    let (versions, selected_source, selected_version) = match result {
        Ok(result) => result,
        Err(error) => {
            if let Ok(item) = MenuItem::with_id(
                menu.app_handle(),
                "runtime-version-error",
                format!("无法读取版本：{error:#}"),
                false,
                None::<&str>,
            ) {
                let _ = menu.append(&item);
                if let Ok(mut items) = state.version_items.lock() {
                    *items = vec![item];
                }
            }
            return;
        }
    };

    let menu_enabled = state.update.lock().map_or(true, |update| !update.busy);
    let mut items = Vec::with_capacity(versions.len());
    for (index, version) in versions.iter().enumerate() {
        let selected = selected_version.as_deref() == Some(version.release.as_str())
            && source_policy_accepts(selected_source, version.source);
        let marker = if selected { "✓ " } else { "" };
        let label = format!(
            "{marker}{} ({})",
            version.release,
            source_label(version.source)
        );
        let id = format!("runtime-version-{index}");
        let Ok(item) = MenuItem::with_id(menu.app_handle(), &id, label, menu_enabled, None::<&str>)
        else {
            continue;
        };
        if let Ok(mut selections) = state.version_selections.lock() {
            selections.insert(id, (version.source, version.release.clone()));
        }
        items.push(item);
    }
    if items.is_empty() {
        if let Ok(item) = MenuItem::with_id(
            menu.app_handle(),
            "runtime-version-empty",
            "没有可激活的兼容版本",
            false,
            None::<&str>,
        ) {
            let _ = menu.append(&item);
            items.push(item);
        }
    } else {
        let refs = items
            .iter()
            .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>)
            .collect::<Vec<_>>();
        let _ = menu.append_items(&refs);
    }
    if let Ok(mut stored) = state.version_items.lock() {
        *stored = items;
    }
}

fn select_runtime_source(app: &AppHandle, source: UpdateSourcePolicy) {
    let state = app.state::<HostState>().inner().clone();
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let paths = AppPaths::discover()?;
            let manager = RuntimeManager::new(paths)?;
            let mut settings = manager.read_settings()?;
            settings.source = source;
            settings.version = None;
            manager.write_settings(&settings)
        })();
        match result {
            Ok(()) => {
                refresh_source_checks(&state, source);
                refresh_runtime_versions(state.clone());
                run_update_check(state, true, true);
            }
            Err(error) => dialogs::error("DSH Desktop", format!("保存运行时来源失败：{error:#}")),
        }
    });
}

fn select_runtime_version(app: &AppHandle, id: &str) {
    let state = app.state::<HostState>().inner().clone();
    let selection = state
        .version_selections
        .lock()
        .ok()
        .and_then(|selections| selections.get(id).cloned());
    let Some((source, version)) = selection else {
        return;
    };
    thread::spawn(move || {
        let policy = runtime_source_policy(source);
        let result = (|| -> Result<()> {
            let paths = AppPaths::discover()?;
            let manager = RuntimeManager::new(paths)?;
            let mut settings = manager.read_settings()?;
            settings.source = policy;
            settings.version = Some(version);
            manager.write_settings(&settings)
        })();
        match result {
            Ok(()) => {
                refresh_source_checks(&state, policy);
                refresh_runtime_versions(state.clone());
                run_update_check(state, true, true);
            }
            Err(error) => dialogs::error("DSH Desktop", format!("保存运行时版本失败：{error:#}")),
        }
    });
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
                        update.available = Some(AvailableUpdate {
                            release: staged.release.clone(),
                            source: staged.source,
                        });
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
    refresh_runtime_versions(state.clone());
    let phase = state
        .update
        .lock()
        .ok()
        .map_or(UpdatePhase::Check, |state| state.phase);
    match phase {
        UpdatePhase::Check | UpdatePhase::Failed => run_update_check(state, false, true),
        UpdatePhase::Stage => stage_update(state, true),
        UpdatePhase::Restart => activate_update(app, state),
    }
}

fn start_update_scheduler(state: HostState) {
    thread::spawn(move || {
        run_update_check(state.clone(), true, false);
        loop {
            thread::sleep(Duration::from_hours(6));
            run_update_check(state.clone(), true, false);
        }
    });
}

fn run_update_check(state: HostState, automatic: bool, notify_errors: bool) {
    if !begin_update_work(&state, UpdatePhase::Check) {
        if notify_errors {
            dialogs::info("DSH Desktop", "运行时更新正在进行，请稍后再选择版本。");
        }
        return;
    }
    thread::spawn(move || {
        let result = (|| -> Result<Option<AvailableUpdate>> {
            let paths = AppPaths::discover()?;
            RuntimeManager::new(paths)?.check_for_update()
        })();
        match result {
            Ok(Some(update_available)) => {
                let version = update_available.release.clone();
                let source = source_label(update_available.source);
                if let Ok(mut update) = state.update.lock() {
                    update.available = Some(update_available);
                    update.phase = UpdatePhase::Stage;
                    if !automatic {
                        update.busy = false;
                    }
                }
                if automatic {
                    set_update_text(&state, "Downloading and staging update");
                    stage_update_reserved(state, false);
                } else {
                    set_update_text(&state, "Download and stage update");
                    dialogs::info(
                        "DSH Desktop",
                        format!("发现可用运行时更新：{version}（来源：{source}）"),
                    );
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
                if !automatic || notify_errors {
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
    stage_update_reserved(state, notify);
}

fn stage_update_reserved(state: HostState, notify: bool) {
    thread::spawn(move || {
        let result = (|| -> Result<Option<StagedRelease>> {
            let paths = AppPaths::discover()?;
            RuntimeManager::new(paths)?.stage_update()
        })();
        match result {
            Ok(Some(staged)) => {
                let version = staged.release.clone();
                let source = source_label(staged.source);
                let staged_source = staged.source;
                if let Ok(mut update) = state.update.lock() {
                    update.staged = Some(staged);
                    update.available = Some(AvailableUpdate {
                        release: version.clone(),
                        source: staged_source,
                    });
                    update.phase = UpdatePhase::Restart;
                    update.busy = false;
                }
                set_update_text(&state, "Restart to activate update");
                if notify {
                    dialogs::info(
                        "DSH Desktop",
                        format!(
                            "运行时 {version}（来源：{source}）已下载并通过 doctor，可从托盘重启激活。"
                        ),
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
    set_runtime_menu_enabled(state, enabled);
}

fn set_runtime_menu_enabled(state: &HostState, enabled: bool) {
    if let Ok(items) = state.source_items.lock() {
        for (_, item) in items.iter() {
            let _ = item.set_enabled(enabled);
        }
    }
    let selectable_ids = state
        .version_selections
        .lock()
        .map(|selections| selections.keys().cloned().collect::<HashSet<_>>())
        .unwrap_or_default();
    if let Ok(items) = state.version_items.lock() {
        for item in items.iter() {
            let selectable = selectable_ids.contains(item.id().as_ref());
            let _ = item.set_enabled(enabled && selectable);
        }
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
