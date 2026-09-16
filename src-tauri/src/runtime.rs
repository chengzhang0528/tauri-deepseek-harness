use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::official;
use crate::paths::AppPaths;

pub const LAUNCHER_VERSION: &str = env!("DSH_LAUNCHER_VERSION");
const SCHEMA: u32 = 2;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeSource {
    Local,
    #[default]
    Oss,
    Npm,
}

impl RuntimeSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "本地副本",
            Self::Oss => "旧 OSS 副本",
            Self::Npm => "官方 npm",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateSourcePolicy {
    #[default]
    Auto,
    Local,
    Oss,
    Npm,
}

impl UpdateSourcePolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动选择",
            Self::Local => "仅本机已下载的版本",
            Self::Oss => "仅旧版安装缓存",
            Self::Npm => "官方软件源",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NpmUpdateSettings {
    #[serde(default = "default_registry")]
    pub registry: String,
    #[serde(default = "default_package")]
    pub package: String,
}
fn default_registry() -> String {
    official::REGISTRY.into()
}
fn default_package() -> String {
    official::PACKAGE.into()
}
impl Default for NpmUpdateSettings {
    fn default() -> Self {
        Self {
            registry: default_registry(),
            package: default_package(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateSettings {
    #[serde(default)]
    pub source: UpdateSourcePolicy,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub npm: NpmUpdateSettings,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}
impl RuntimeUpdateSettings {
    fn validate(&self) -> Result<()> {
        if let Some(version) = &self.version {
            semver::Version::parse(version).context("固定版本格式无效")?;
        }
        let registry = Url::parse(&self.npm.registry).context("npm 来源地址无效")?;
        ensure!(registry.host_str().is_some(), "npm 来源地址缺少主机");
        ensure!(
            self.npm.package == official::PACKAGE,
            "仅管理官方 @deepseek-ai/dsh 包"
        );
        Ok(())
    }
}

/// A prepared position in this installation, not a certification of upstream bytes.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCopy {
    pub schema: u32,
    pub location: String,
    pub release: String,
    pub harness_version: Option<String>,
    pub source: RuntimeSource,
    pub publisher: String,
    pub registry: Option<String>,
}
impl RuntimeCopy {
    pub fn version_label(&self) -> &str {
        self.harness_version.as_deref().unwrap_or("未知")
    }
}

#[derive(Debug, Clone)]
pub struct PreparedRuntime {
    pub root: PathBuf,
    pub copy: RuntimeCopy,
    pub bridge_patch: PathBuf,
    pub data_home: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub source: RuntimeSource,
    pub registry: Option<String>,
    pub local_copy: Option<RuntimeCopy>,
}

#[derive(Debug, Clone, Default)]
pub struct CheckResult {
    pub versions: Vec<AvailableUpdate>,
    pub available: Option<AvailableUpdate>,
    pub issues: Vec<String>,
}
impl CheckResult {
    pub fn summary(&self) -> String {
        let result = self.available.as_ref().map_or_else(
            || "没有找到可安装的新版本".into(),
            |target| format!("可更新 Harness {}", target.version),
        );
        if self.issues.is_empty() {
            result
        } else {
            format!("检查不完整：{}\n{result}", self.issues.join("；"))
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepairRecord {
    pub release: String,
    pub phase: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeManager {
    pub paths: AppPaths,
    client: Client,
}

impl RuntimeManager {
    pub fn new(paths: AppPaths) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_mins(3))
            .build()
            .context("无法创建下载客户端")?;
        Ok(Self { paths, client })
    }
    pub fn read_settings(&self) -> Result<RuntimeUpdateSettings> {
        let settings =
            read_json::<RuntimeUpdateSettings>(&self.paths.settings_path())?.unwrap_or_default();
        settings.validate()?;
        Ok(settings)
    }
    pub fn write_settings(&self, settings: &RuntimeUpdateSettings) -> Result<()> {
        settings.validate()?;
        write_json(&self.paths.settings_path(), settings)
    }
    pub fn read_current(&self) -> Result<Option<RuntimeCopy>> {
        self.read_copy(&self.paths.current_pointer())
    }
    pub fn read_staged(&self) -> Result<Option<RuntimeCopy>> {
        self.read_copy(&self.paths.staged_pointer())
    }

    fn read_copy(&self, path: &Path) -> Result<Option<RuntimeCopy>> {
        let Some(value) = read_json::<Value>(path)? else {
            return Ok(None);
        };
        if value.get("schema").is_some() {
            let copy: RuntimeCopy =
                serde_json::from_value(value).context("副本状态格式无效，原状态已保留")?;
            self.validate_copy(&copy)?;
            return Ok(Some(copy));
        }
        // Read the one released legacy layout without overwriting its pointer.
        let release = value
            .get("release")
            .and_then(Value::as_str)
            .context("旧运行时缺少发行号；需要修复")?;
        semver::Version::parse(release).context("旧运行时发行号无效")?;
        let source: RuntimeSource = value
            .get("source")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?
            .unwrap_or_default();
        let location = format!(
            "{release}{}",
            if source == RuntimeSource::Npm {
                "-npm"
            } else {
                ""
            }
        );
        let root = self.paths.runtimes.join(&location);
        let harness_version = package_version(&root);
        let copy = RuntimeCopy {
            schema: SCHEMA,
            location,
            release: release.into(),
            harness_version,
            source,
            publisher: "legacy-desktop-runtime".into(),
            registry: None,
        };
        self.validate_copy(&copy)?;
        Ok(Some(copy))
    }

    fn validate_copy(&self, copy: &RuntimeCopy) -> Result<()> {
        ensure!(
            copy.schema == SCHEMA,
            "setup-required：副本状态需要其他 Launcher 版本"
        );
        ensure!(safe_name(&copy.location), "副本位置不属于安装目录");
        semver::Version::parse(&copy.release).context("副本发行号无效")?;
        if let Some(version) = &copy.harness_version {
            semver::Version::parse(version).context("Harness 版本格式无效")?;
        }
        let root = self.paths.runtimes.join(&copy.location);
        if root.exists() {
            let resolved_root = fs::canonicalize(&root)?;
            let resolved_parent = fs::canonicalize(&self.paths.runtimes)?;
            ensure!(
                resolved_root.parent() == Some(resolved_parent.as_path()),
                "副本实际位置越出运行时目录"
            );
        }
        Ok(())
    }

    pub fn check_for_update(&self) -> Result<CheckResult> {
        let settings = self.read_settings()?;
        let current = self.read_current()?;
        let mut result = CheckResult::default();
        if matches!(
            settings.source,
            UpdateSourcePolicy::Auto | UpdateSourcePolicy::Local | UpdateSourcePolicy::Oss
        ) {
            let staged = self.read_staged();
            let mut copies = current.iter().cloned().collect::<Vec<_>>();
            match staged {
                Ok(Some(copy)) => copies.push(copy),
                Ok(None) => {}
                Err(_) => result.issues.push("暂存记录不可读".into()),
            }
            for copy in copies {
                if self.runtime_files(&copy).is_err() {
                    result
                        .issues
                        .push(format!("本地副本 {} 文件缺失", copy.release));
                    continue;
                }
                if let Some(version) = &copy.harness_version {
                    if settings.source == UpdateSourcePolicy::Oss
                        && copy.source != RuntimeSource::Oss
                    {
                        continue;
                    }
                    result.versions.push(AvailableUpdate {
                        version: version.clone(),
                        source: copy.source,
                        registry: copy.registry.clone(),
                        local_copy: Some(copy),
                    });
                }
            }
            if settings.source == UpdateSourcePolicy::Oss {
                result
                    .issues
                    .push("旧 OSS 来源保留为本机副本；获取官方新版本请选择官方 npm".into());
            }
        }
        if matches!(
            settings.source,
            UpdateSourcePolicy::Auto | UpdateSourcePolicy::Npm
        ) {
            match official::versions(&self.client, &settings.npm.registry) {
                Ok(versions) => {
                    result
                        .versions
                        .extend(versions.into_iter().map(|version| AvailableUpdate {
                            version,
                            source: RuntimeSource::Npm,
                            registry: Some(settings.npm.registry.clone()),
                            local_copy: None,
                        }));
                }
                Err(error) => result.issues.push(error.to_string()),
            }
        }
        result
            .versions
            .sort_by(|left, right| official::compare_versions(&right.version, &left.version));
        result.available = choose_update(&result.versions, current.as_ref(), &settings);
        if let Some(version) = &settings.version {
            let found = result
                .versions
                .iter()
                .any(|target| matches_requested(target, &settings, version));
            if !found {
                result.issues.push(format!(
                    "所选版本 {version} 未在当前来源找到；旧 runtime 发行号不能代替 Harness 版本"
                ));
            }
        }
        Ok(result)
    }

    pub fn prepare_current(&self, progress: &dyn Fn(&str)) -> Result<PreparedRuntime> {
        self.paths.create()?;
        if let Some(copy) = self.read_current()? {
            return self.prepare_for_start(&copy);
        }
        ensure!(
            self.read_repair()?.is_none(),
            "存在上次失败线索，请使用重新准备或修复入口"
        );
        let check = self.check_for_update()?;
        let summary = check.summary();
        let target = check.available.context(summary)?;
        let copy = self.prepare_target(&target, progress)?;
        write_json(&self.paths.current_pointer(), &copy)?;
        self.prepare_for_start(&copy)
    }

    pub fn stage_target(
        &self,
        target: &AvailableUpdate,
        progress: &dyn Fn(&str),
    ) -> Result<RuntimeCopy> {
        self.paths.create()?;
        ensure_forward(self.read_current()?.as_ref(), &target.version)?;
        if let Some(copy) = self.read_staged()?
            && copy.harness_version.as_deref() == Some(&target.version)
            && copy.source == target.source
            && copy.registry == target.registry
            && self.runtime_files(&copy).is_ok()
        {
            return Ok(copy);
        }
        let copy = self.prepare_target(target, progress)?;
        write_json(&self.paths.staged_pointer(), &copy)?;
        Ok(copy)
    }

    fn prepare_target(
        &self,
        target: &AvailableUpdate,
        progress: &dyn Fn(&str),
    ) -> Result<RuntimeCopy> {
        if let Some(copy) = &target.local_copy {
            self.runtime_files(copy)?;
            return Ok(copy.clone());
        }
        ensure!(
            target.source == RuntimeSource::Npm,
            "该来源没有官方安装入口"
        );
        semver::Version::parse(&target.version).context("目标版本无效")?;
        let registry = target.registry.as_deref().context("缺少官方发行来源")?;
        let location = format!("npm-{}-{}", target.version, Uuid::new_v4().simple());
        let root = self.paths.runtimes.join(&location);
        fs::create_dir(&root).context("无法创建新的私有副本目录")?;
        let result = (|| -> Result<RuntimeCopy> {
            progress("正在准备私有 Node.js…");
            let reusable = self
                .read_current()?
                .map(|copy| self.paths.runtimes.join(copy.location).join("node"));
            official::prepare_node(&self.client, &root, reusable.as_deref())?;
            progress(&format!(
                "正在通过官方 npm 下载并安装 Harness {}…",
                target.version
            ));
            official::install(
                &root,
                &target.version,
                registry,
                &self.paths.cache.join("npm"),
            )?;
            let copy = RuntimeCopy {
                schema: SCHEMA,
                location: location.clone(),
                release: target.version.clone(),
                harness_version: Some(target.version.clone()),
                source: RuntimeSource::Npm,
                publisher: official::PACKAGE.into(),
                registry: Some(registry.into()),
            };
            self.runtime_files(&copy)?;
            write_json(&root.join("desktop-copy.json"), &copy)?;
            Ok(copy)
        })();
        if result.is_err() && root.parent() == Some(self.paths.runtimes.as_path()) {
            let _ = fs::remove_dir_all(&root);
        }
        result
    }

    pub fn prepare_for_start(&self, copy: &RuntimeCopy) -> Result<PreparedRuntime> {
        let root = self.runtime_files(copy)?;
        let bridge_dir = self.paths.launcher.join("bridge");
        fs::create_dir_all(&bridge_dir)?;
        let script_path = bridge_dir.join("desktop-bridge.mjs");
        fs::write(
            &script_path,
            include_bytes!("../resources/desktop-bridge.mjs"),
        )?;
        let script_url =
            Url::from_file_path(&script_path).map_err(|()| anyhow::anyhow!("bridge 路径无效"))?;
        let patch = include_str!("../resources/desktop-bridge.patch.yml")
            .replace("__DSH_DESKTOP_BRIDGE_MODULE__", script_url.as_str());
        let bridge_patch = bridge_dir.join("desktop-bridge.patch.yml");
        fs::write(&bridge_patch, patch)?;
        Ok(PreparedRuntime {
            root,
            copy: copy.clone(),
            bridge_patch,
            data_home: self.paths.dsh_home.clone(),
        })
    }

    fn runtime_files(&self, copy: &RuntimeCopy) -> Result<PathBuf> {
        self.validate_copy(copy)?;
        let root = self.paths.runtimes.join(&copy.location);
        ensure!(
            root.join("node/node.exe").is_file()
                && root
                    .join("node_modules/@deepseek-ai/dsh/lib/bin.js")
                    .is_file(),
            "本机副本文件缺失，请重新准备；用户数据已保留"
        );
        Ok(root)
    }

    /// Consume the exact confirmed copy. Never reread staged to select the target.
    pub fn activate(&self, confirmed: &RuntimeCopy) -> Result<PreparedRuntime> {
        let prepared = self.prepare_for_start(confirmed)?;
        if let Some(version) = &confirmed.harness_version {
            ensure_forward(self.read_current()?.as_ref(), version)?;
        }
        write_json(&self.paths.current_pointer(), confirmed)?;
        if self.read_staged()?.as_ref() == Some(confirmed) {
            fs::remove_file(self.paths.staged_pointer())?;
        }
        Ok(prepared)
    }

    pub fn repair_current(&self, progress: &dyn Fn(&str)) -> Result<PreparedRuntime> {
        let current = self
            .read_current()?
            .context("没有可修复的当前版本；请检查并选择官方目标")?;
        if self.runtime_files(&current).is_ok() {
            return self.prepare_for_start(&current);
        }
        ensure!(
            current.source == RuntimeSource::Npm,
            "旧副本缺失；请选择明确的官方版本进行前向修复"
        );
        let version = current
            .harness_version
            .clone()
            .context("当前 Harness 版本未知，不能推定修复目标")?;
        let copy = self.prepare_target(
            &AvailableUpdate {
                version,
                source: RuntimeSource::Npm,
                registry: current.registry.clone(),
                local_copy: None,
            },
            progress,
        )?;
        self.activate(&copy)
    }

    pub fn read_repair(&self) -> Result<Option<RepairRecord>> {
        read_json(&self.paths.repair_record())
    }
    pub fn record_forward_repair(
        &self,
        release: impl Into<String>,
        phase: &str,
        error: &str,
    ) -> Result<()> {
        // Callers supply bounded, user-safe stage descriptions, never raw runtime output.
        write_json(
            &self.paths.repair_record(),
            &RepairRecord {
                release: release.into(),
                phase: phase.into(),
                error: error.chars().take(1024).collect(),
            },
        )
    }
    pub fn clear_repair(&self) -> Result<()> {
        match fs::remove_file(self.paths.repair_record()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn matches_requested(
    target: &AvailableUpdate,
    settings: &RuntimeUpdateSettings,
    version: &str,
) -> bool {
    if matches!(
        settings.source,
        UpdateSourcePolicy::Local | UpdateSourcePolicy::Oss
    ) {
        target.local_copy.as_ref().is_some_and(|copy| {
            copy.release == version || copy.harness_version.as_deref() == Some(version)
        })
    } else {
        target.version == version
    }
}

fn choose_update(
    targets: &[AvailableUpdate],
    current: Option<&RuntimeCopy>,
    settings: &RuntimeUpdateSettings,
) -> Option<AvailableUpdate> {
    targets
        .iter()
        .filter(|target| {
            settings
                .version
                .as_deref()
                .is_none_or(|version| matches_requested(target, settings, version))
        })
        .filter(|target| {
            current.is_none_or(|copy| {
                copy.harness_version.as_ref().is_none_or(|version| {
                    official::compare_versions(&target.version, version).is_gt()
                })
            })
        })
        .max_by(|left, right| official::compare_versions(&left.version, &right.version))
        .cloned()
}
fn ensure_forward(current: Option<&RuntimeCopy>, target: &str) -> Result<()> {
    semver::Version::parse(target)?;
    if let Some(version) = current.and_then(|copy| copy.harness_version.as_deref()) {
        ensure!(
            !official::compare_versions(target, version).is_lt(),
            "不自动降级 Harness 或用户数据"
        );
    }
    Ok(())
}
fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['/', '\\', ':'])
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
fn package_version(root: &Path) -> Option<String> {
    let package: Value = read_json(&root.join("node_modules/@deepseek-ai/dsh/package.json"))
        .ok()
        .flatten()?;
    let version = package.get("version")?.as_str()?;
    semver::Version::parse(version).ok()?;
    Some(version.into())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .context("本机状态不可读，原文件已保留"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("无法读取本机状态"),
    }
}
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("状态文件缺少目录")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".{}.tmp", Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        drop(file);
        atomic_replace(&temp, path).context("无法提交本机状态")
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};

    if !destination.exists() {
        return fs::rename(source, destination);
    }
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are NUL-terminated UTF-16 strings whose storage outlives the call.
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            source_wide.as_ptr(),
            ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            ptr::null(),
            ptr::null(),
        )
    };
    if replaced != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manager_at(root: &Path) -> RuntimeManager {
        let paths = AppPaths {
            local_root: root.into(),
            roaming_root: root.join("user"),
            launcher: root.join("launcher"),
            runtimes: root.join("runtimes"),
            cache: root.join("cache"),
            staging: root.join("staging"),
            state: root.join("state"),
            logs: root.join("logs"),
            dsh_home: root.join("user/home"),
        };
        paths.create().unwrap();
        RuntimeManager::new(paths).unwrap()
    }
    fn copy(manager: &RuntimeManager, version: &str) -> RuntimeCopy {
        let copy = RuntimeCopy {
            schema: SCHEMA,
            location: format!("copy-{version}"),
            release: version.into(),
            harness_version: Some(version.into()),
            source: RuntimeSource::Npm,
            publisher: official::PACKAGE.into(),
            registry: Some(official::REGISTRY.into()),
        };
        let root = manager.paths.runtimes.join(&copy.location);
        fs::create_dir_all(root.join("node")).unwrap();
        fs::create_dir_all(root.join("node_modules/@deepseek-ai/dsh/lib")).unwrap();
        fs::write(root.join("node/node.exe"), b"test").unwrap();
        fs::write(
            root.join("node_modules/@deepseek-ai/dsh/lib/bin.js"),
            b"test",
        )
        .unwrap();
        copy
    }
    #[test]
    fn confirmed_copy_does_not_follow_new_staged_target() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager_at(temp.path());
        let c1 = copy(&manager, "1.0.0");
        let c2 = copy(&manager, "2.0.0");
        let c3 = copy(&manager, "3.0.0");
        write_json(&manager.paths.current_pointer(), &c1).unwrap();
        write_json(&manager.paths.staged_pointer(), &c3).unwrap();
        manager.activate(&c2).unwrap();
        assert_eq!(manager.read_current().unwrap(), Some(c2));
        assert_eq!(manager.read_staged().unwrap(), Some(c3));
    }
    #[test]
    fn legacy_pointer_is_read_without_rewrite_or_version_conflation() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager_at(temp.path());
        let bytes = br#"{"release":"0.1.5","source":"oss"}"#;
        fs::write(manager.paths.current_pointer(), bytes).unwrap();
        let current = manager.read_current().unwrap().unwrap();
        assert_eq!(current.location, "0.1.5");
        assert_eq!(current.harness_version, None);
        assert_eq!(fs::read(manager.paths.current_pointer()).unwrap(), bytes);
    }
    #[test]
    fn failed_target_preserves_current_staged_and_user_data() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager_at(temp.path());
        let current = copy(&manager, "1.0.0");
        let staged = copy(&manager, "2.0.0");
        write_json(&manager.paths.current_pointer(), &current).unwrap();
        write_json(&manager.paths.staged_pointer(), &staged).unwrap();
        fs::write(manager.paths.dsh_home.join("sentinel"), b"user").unwrap();
        let target = AvailableUpdate {
            version: "3.0.0".into(),
            source: RuntimeSource::Oss,
            registry: None,
            local_copy: None,
        };
        assert!(manager.stage_target(&target, &|_| {}).is_err());
        assert_eq!(manager.read_current().unwrap(), Some(current));
        assert_eq!(manager.read_staged().unwrap(), Some(staged));
        assert_eq!(
            fs::read(manager.paths.dsh_home.join("sentinel")).unwrap(),
            b"user"
        );
    }
    #[test]
    fn installation_cannot_reference_an_outside_copy_or_downgrade() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager_at(temp.path());
        let mut current = copy(&manager, "2.0.0");
        assert!(ensure_forward(Some(&current), "1.0.0").is_err());
        current.location = "../outside".into();
        assert!(manager.validate_copy(&current).is_err());
    }
    #[test]
    fn fixed_missing_version_is_not_replaced_with_latest() {
        let settings = RuntimeUpdateSettings {
            version: Some("1.0.0".into()),
            ..Default::default()
        };
        let targets = [AvailableUpdate {
            version: "2.0.0".into(),
            source: RuntimeSource::Npm,
            registry: None,
            local_copy: None,
        }];
        assert!(choose_update(&targets, None, &settings).is_none());
        assert!(
            CheckResult {
                issues: vec!["来源不可用".into()],
                ..Default::default()
            }
            .summary()
            .contains("检查不完整")
        );
    }
}
