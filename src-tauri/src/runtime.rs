use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;
use wait_timeout::ChildExt;
use zip::ZipArchive;

use crate::bootstrap::{fetch_bootstrap, load_seed_or_remote, release_origin, write_seed_copy};
use crate::manifest::{
    AssetRef, ReleaseManifest, RuntimeComponent, compare_versions, is_safe_relative_path,
};
use crate::paths::AppPaths;

pub const MAX_ARTIFACT_BYTES: u64 = 768 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;
pub const MAX_EXTRACTED_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const BRIDGE_PATCH_FILE: &str = "desktop-bridge.patch.yml";
const BRIDGE_SCRIPT_FILE: &str = "desktop-bridge.mjs";
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRelease {
    pub release: String,
    pub manifest_sha256: String,
    #[serde(default)]
    pub manifest: Option<ReleaseManifest>,
    #[serde(default)]
    pub manifest_snapshot_sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedRuntime {
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RuntimeManager {
    paths: AppPaths,
    client: Client,
}

impl RuntimeManager {
    pub fn new(paths: AppPaths) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_mins(1))
            .build()
            .context("cannot create HTTP client")?;
        Ok(Self { paths, client })
    }

    pub fn prepare_current(&self) -> Result<PreparedRuntime> {
        self.paths.create()?;
        write_seed_copy(&self.paths.launcher.join("bootstrap.windows-x64.seed.json"))?;
        let origin = release_origin()?;
        let current = self.read_current()?;
        let remote = (|| -> Result<(crate::manifest::Bootstrap, ReleaseManifest)> {
            let bootstrap = if current.is_some() {
                fetch_bootstrap(&self.client, &origin)?
            } else {
                load_seed_or_remote(&self.client, &origin)?
            };
            ensure_launcher_compatible(&bootstrap.minimum_launcher)?;
            let manifest_bytes = self.download_verified(&origin, &bootstrap.manifest)?;
            let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)
                .context("runtime manifest JSON is invalid")?;
            manifest.validate().context("runtime manifest is invalid")?;
            ensure_launcher_compatible(&manifest.minimum_launcher)?;
            if manifest.release != bootstrap.release {
                bail!("bootstrap release does not match runtime manifest release");
            }
            Ok((bootstrap, manifest))
        })();
        let (bootstrap, manifest) = match remote {
            Ok(remote) => remote,
            Err(remote_error) => {
                if let Some(prepared) = self.prepare_existing_current(current.as_ref()) {
                    return Ok(prepared);
                }
                return Err(remote_error)
                    .context("runtime source is unavailable and no usable current runtime exists");
            }
        };
        ensure_no_downgrade(current.as_ref(), &manifest.release)?;
        let target_root = self.paths.runtimes.join(&manifest.release);
        let pointer_matches = current.as_ref().is_some_and(|release| {
            release.release == manifest.release
                && release
                    .manifest_sha256
                    .eq_ignore_ascii_case(&bootstrap.manifest.sha256)
        });
        if pointer_matches && target_root.is_dir() {
            if bridge_files_match(&target_root)
                && Self::run_doctors(&manifest, &target_root).is_ok()
            {
                return Ok(PreparedRuntime { root: target_root });
            }
            bail!(
                "current runtime {} failed bridge or doctor validation; repair is required",
                manifest.release
            );
        }
        if target_root.exists() {
            bail!(
                "runtime directory collision for release {}",
                manifest.release
            );
        }

        let staging = self
            .paths
            .staging
            .join(format!("{}-{}", manifest.release, Uuid::new_v4()));
        fs::create_dir_all(&staging)
            .with_context(|| format!("cannot create staging root {}", staging.display()))?;
        let staged = (|| -> Result<()> {
            for component in &manifest.components {
                self.stage_component(&origin, component, &staging)?;
            }
            materialize_bridge_files(&staging)?;
            Self::run_doctors(&manifest, &staging)?;
            Ok(())
        })();
        if let Err(error) = staged {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        fs::rename(&staging, &target_root).context("cannot activate runtime directory")?;
        let pointer = CurrentRelease {
            release: manifest.release.clone(),
            manifest_sha256: bootstrap.manifest.sha256,
            manifest_snapshot_sha256: Some(manifest_snapshot_sha256(&manifest)?),
            manifest: Some(manifest),
        };
        if let Err(error) = self.write_current(&pointer) {
            let _ = fs::remove_dir_all(&target_root);
            return Err(error);
        }
        Ok(PreparedRuntime { root: target_root })
    }

    fn prepare_existing_current(
        &self,
        current: Option<&CurrentRelease>,
    ) -> Option<PreparedRuntime> {
        let current = current?;
        let manifest = current.manifest.as_ref()?;
        if !current_snapshot_is_valid(current, manifest) {
            return None;
        }
        let target_root = self.paths.runtimes.join(&current.release);
        if !target_root.is_dir() || !bridge_files_match(&target_root) {
            return None;
        }
        if Self::run_doctors(manifest, &target_root).is_ok() {
            return Some(PreparedRuntime { root: target_root });
        }
        None
    }

    pub fn check_for_update(&self) -> Result<Option<String>> {
        let origin = release_origin()?;
        let bootstrap = fetch_bootstrap(&self.client, &origin)?;
        ensure_launcher_compatible(&bootstrap.minimum_launcher)?;
        let current = self.read_current()?;
        ensure_no_downgrade(current.as_ref(), &bootstrap.release)?;
        Ok(match current {
            Some(current)
                if current.release == bootstrap.release
                    && current
                        .manifest_sha256
                        .eq_ignore_ascii_case(&bootstrap.manifest.sha256) =>
            {
                None
            }
            _ => Some(bootstrap.release),
        })
    }

    fn stage_component(
        &self,
        origin: &Url,
        component: &RuntimeComponent,
        staging: &Path,
    ) -> Result<()> {
        let archive = self.download_verified(origin, &component.asset)?;
        let target = staging.join(&component.install_root);
        fs::create_dir_all(&target)
            .with_context(|| format!("cannot create component root {}", target.display()))?;
        extract_zip_safely(&archive, &target)
            .with_context(|| format!("cannot unpack component {}", component.id))
    }

    fn download_verified(&self, origin: &Url, asset: &AssetRef) -> Result<Vec<u8>> {
        ensure!(
            asset.bytes <= MAX_ARTIFACT_BYTES,
            "artifact exceeds the client maximum size"
        );
        let cache_path = self.paths.cache.join(&asset.sha256);
        if cache_path.is_file() && verify_file(&cache_path, asset).is_ok() {
            return fs::read(&cache_path)
                .with_context(|| format!("cannot read {}", cache_path.display()));
        }

        let url = origin
            .join(&asset.object_key)
            .with_context(|| format!("cannot resolve artifact {}", asset.object_key))?;
        let mut response = self
            .client
            .get(url)
            .send()
            .context("artifact download failed")?
            .error_for_status()
            .context("artifact request failed")?;
        let mut bytes =
            Vec::with_capacity(usize::try_from(asset.bytes.min(16 * 1024 * 1024)).unwrap_or(0));
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = response
                .read(&mut buffer)
                .context("cannot read artifact response")?;
            if read == 0 {
                break;
            }
            let next_length = u64::try_from(bytes.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            ensure!(
                next_length <= asset.bytes && next_length <= MAX_ARTIFACT_BYTES,
                "artifact download exceeds declared size"
            );
            bytes.extend_from_slice(&buffer[..read]);
        }
        verify_bytes(&bytes, asset)?;

        let part = cache_path.with_extension(format!("{}.part", Uuid::new_v4()));
        fs::write(&part, &bytes).with_context(|| format!("cannot cache {}", part.display()))?;
        if verify_file(&part, asset).is_err() {
            let _ = fs::remove_file(&part);
            bail!("cached artifact verification failed");
        }
        if cache_path.exists() {
            let _ = fs::remove_file(&part);
        } else {
            fs::rename(&part, &cache_path)
                .with_context(|| format!("cannot publish cache {}", cache_path.display()))?;
        }
        Ok(bytes)
    }

    fn run_doctors(manifest: &ReleaseManifest, root: &Path) -> Result<()> {
        for component in &manifest.components {
            let doctor_path = resolve_relative(root, &component.doctor.program)?;
            let mut command = Command::new(doctor_path);
            command
                .args(&component.doctor.args)
                .current_dir(root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            hide_console(&mut command);
            let mut child = command
                .spawn()
                .with_context(|| format!("cannot start doctor for {}", component.id))?;
            let timeout = Duration::from_secs(component.doctor.timeout_seconds);
            match child
                .wait_timeout(timeout)
                .context("cannot wait for doctor")?
            {
                Some(status) if status.success() => {}
                Some(status) => bail!("doctor for {} exited with {status}", component.id),
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("doctor for {} timed out", component.id);
                }
            }
        }
        Ok(())
    }

    fn read_current(&self) -> Result<Option<CurrentRelease>> {
        let pointer = self.paths.current_pointer();
        if !pointer.exists() {
            return Ok(None);
        }
        let bytes =
            fs::read(&pointer).with_context(|| format!("cannot read {}", pointer.display()))?;
        let current =
            serde_json::from_slice(&bytes).context("current runtime pointer is corrupt")?;
        Ok(Some(current))
    }

    fn write_current(&self, current: &CurrentRelease) -> Result<()> {
        let pointer = self.paths.current_pointer();
        let temporary = pointer.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let content =
            serde_json::to_vec_pretty(current).context("cannot serialize runtime pointer")?;
        fs::write(&temporary, content)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        atomic_replace(&temporary, &pointer)
            .with_context(|| format!("cannot publish {}", pointer.display()))
    }
}

fn ensure_launcher_compatible(minimum_launcher: &str) -> Result<()> {
    if compare_versions(minimum_launcher, LAUNCHER_VERSION)
        .map_err(|error| anyhow::anyhow!("invalid launcher version: {error}"))?
        .is_gt()
    {
        bail!("setup-required: runtime requires Launcher {minimum_launcher} or newer");
    }
    Ok(())
}

fn ensure_no_downgrade(current: Option<&CurrentRelease>, target: &str) -> Result<()> {
    let Some(current) = current else {
        return Ok(());
    };
    if compare_versions(target, &current.release)
        .map_err(|error| anyhow::anyhow!("invalid current runtime version: {error}"))?
        .is_lt()
    {
        bail!(
            "runtime downgrade rejected: current {} is newer than {}",
            current.release,
            target
        );
    }
    Ok(())
}

fn manifest_snapshot_sha256(manifest: &ReleaseManifest) -> Result<String> {
    let bytes =
        serde_json::to_vec(manifest).context("cannot serialize runtime manifest snapshot")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn current_snapshot_is_valid(current: &CurrentRelease, manifest: &ReleaseManifest) -> bool {
    if manifest.validate().is_err()
        || current.release != manifest.release
        || current.manifest_sha256.len() != crate::manifest::SHA256_HEX_LENGTH
        || !current
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    let Some(expected) = current.manifest_snapshot_sha256.as_deref() else {
        return false;
    };
    let Ok(actual) = manifest_snapshot_sha256(manifest) else {
        return false;
    };
    expected.eq_ignore_ascii_case(&actual)
}

pub fn resolve_relative(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(is_safe_relative_path(relative), "unsafe relative path");
    let path = root.join(relative);
    ensure!(path.starts_with(root), "path escapes runtime root");
    Ok(path)
}

fn verify_bytes(bytes: &[u8], asset: &AssetRef) -> Result<()> {
    ensure!(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) == asset.bytes,
        "artifact size mismatch"
    );
    let digest = hex::encode(Sha256::digest(bytes));
    ensure!(
        digest.eq_ignore_ascii_case(&asset.sha256),
        "artifact SHA-256 mismatch"
    );
    Ok(())
}

fn verify_file(path: &Path, asset: &AssetRef) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    ensure!(metadata.len() == asset.bytes, "artifact size mismatch");
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest).context("cannot hash cached artifact")?;
    ensure!(
        hex::encode(digest.finalize()).eq_ignore_ascii_case(&asset.sha256),
        "artifact SHA-256 mismatch"
    );
    Ok(())
}

pub fn extract_zip_safely(bytes: &[u8], root: &Path) -> Result<()> {
    let reader = io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader).context("invalid ZIP archive")?;
    ensure!(
        archive.len() <= MAX_ARCHIVE_ENTRIES,
        "runtime archive has too many entries"
    );
    let mut names = HashSet::new();
    let mut extracted_bytes = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .context("cannot inspect ZIP entry")?;
        let name = entry.name().to_owned();
        ensure!(names.insert(name.clone()), "duplicate ZIP entry {name}");
        let path = Path::new(&name);
        ensure!(
            !path.is_absolute()
                && path
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "unsafe ZIP entry {name}"
        );
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            bail!("symbolic links are not accepted in runtime archives");
        }
        let target = root.join(path);
        ensure!(target.starts_with(root), "ZIP entry escapes destination");
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("cannot create {}", target.display()))?;
            continue;
        }
        let declared_size = entry.size();
        ensure!(
            declared_size <= MAX_EXTRACTED_FILE_BYTES,
            "ZIP entry exceeds per-file limit"
        );
        extracted_bytes = extracted_bytes
            .checked_add(declared_size)
            .context("runtime archive exceeds extracted size limit")?;
        ensure!(
            extracted_bytes <= MAX_EXTRACTED_BYTES,
            "runtime archive exceeds extracted size limit"
        );
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .with_context(|| format!("cannot create {}", target.display()))?;
        let copied = io::copy(&mut entry.take(MAX_EXTRACTED_FILE_BYTES + 1), &mut output)
            .with_context(|| format!("cannot extract {}", target.display()))?;
        ensure!(copied == declared_size, "ZIP entry size mismatch");
        output.flush().context("cannot flush extracted file")?;
    }
    Ok(())
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

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn bridge_files_match(root: &Path) -> bool {
    [
        (
            BRIDGE_PATCH_FILE,
            include_bytes!("../resources/desktop-bridge.patch.yml").as_slice(),
        ),
        (
            BRIDGE_SCRIPT_FILE,
            include_bytes!("../resources/desktop-bridge.mjs").as_slice(),
        ),
    ]
    .into_iter()
    .all(|(name, expected)| fs::read(root.join(name)).is_ok_and(|actual| actual == expected))
}

fn materialize_bridge_files(root: &Path) -> Result<()> {
    for (name, contents) in [
        (
            BRIDGE_PATCH_FILE,
            include_bytes!("../resources/desktop-bridge.patch.yml").as_slice(),
        ),
        (
            BRIDGE_SCRIPT_FILE,
            include_bytes!("../resources/desktop-bridge.mjs").as_slice(),
        ),
    ] {
        let target = root.join(name);
        if target.exists() {
            ensure!(
                fs::read(&target).with_context(|| format!("cannot read {}", target.display()))?
                    == contents,
                "runtime contains an unexpected desktop bridge file {}",
                target.display()
            );
            continue;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .with_context(|| format!("cannot create {}", target.display()))?;
        file.write_all(contents)
            .with_context(|| format!("cannot write {}", target.display()))?;
        file.flush()
            .with_context(|| format!("cannot flush {}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn safe_extractor_rejects_path_traversal() {
        let mut bytes = io::Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut bytes);
            zip.start_file("../outside.txt", SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(b"no").expect("data");
            zip.finish().expect("finish");
        }
        let root = tempdir().expect("temp");
        assert!(extract_zip_safely(bytes.get_ref(), root.path()).is_err());
    }

    #[test]
    fn safe_extractor_writes_normal_entries_only_once() {
        let mut bytes = io::Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut bytes);
            zip.start_file("node/node.exe", SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(b"node").expect("data");
            zip.finish().expect("finish");
        }
        let root = tempdir().expect("temp");
        extract_zip_safely(bytes.get_ref(), root.path()).expect("extract");
        assert_eq!(
            fs::read(root.path().join("node/node.exe")).expect("read"),
            b"node"
        );
    }

    #[test]
    fn relative_resolver_rejects_absolute_and_parent_paths() {
        let root = tempdir().expect("temp");
        for path in ["../node.exe", "/node.exe", "C:/node.exe"] {
            assert!(resolve_relative(root.path(), path).is_err(), "{path}");
        }
    }

    #[test]
    fn bridge_files_are_materialized_and_pinned() {
        let root = tempdir().expect("temp");
        materialize_bridge_files(root.path()).expect("bridge files");
        assert!(bridge_files_match(root.path()));
    }

    #[test]
    fn rejects_runtime_downgrades() {
        let current = CurrentRelease {
            release: "1.2.0".into(),
            manifest_sha256: "a".repeat(64),
            manifest: None,
            manifest_snapshot_sha256: None,
        };
        assert!(ensure_no_downgrade(Some(&current), "1.1.9").is_err());
        assert!(ensure_no_downgrade(Some(&current), "1.2.1").is_ok());
    }

    #[test]
    fn current_snapshot_requires_valid_manifest_and_digest() {
        let manifest = ReleaseManifest {
            schema: 1,
            product: crate::manifest::PRODUCT_ID.into(),
            release: "1.2.0".into(),
            platform: crate::manifest::PLATFORM.into(),
            arch: crate::manifest::ARCH.into(),
            minimum_launcher: "0.1.0".into(),
            components: vec![RuntimeComponent {
                id: "runtime".into(),
                version: "1.2.0".into(),
                asset: AssetRef {
                    object_key: "releases/1.2.0/windows-x64/runtime.zip".into(),
                    bytes: 1,
                    sha256: "a".repeat(64),
                },
                archive: "zip".into(),
                install_root: String::new(),
                doctor: crate::manifest::DoctorSpec {
                    program: "node.exe".into(),
                    args: vec!["--version".into()],
                    timeout_seconds: 30,
                },
                licenses: Vec::new(),
            }],
        };
        let current = CurrentRelease {
            release: manifest.release.clone(),
            manifest_sha256: "b".repeat(64),
            manifest_snapshot_sha256: Some(manifest_snapshot_sha256(&manifest).expect("digest")),
            manifest: Some(manifest.clone()),
        };
        assert!(current_snapshot_is_valid(&current, &manifest));

        let mut tampered = manifest;
        tampered.release = "../outside".into();
        assert!(!current_snapshot_is_valid(&current, &tampered));
    }
}
