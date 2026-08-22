use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use tar::Archive;
use url::Url;
use uuid::Uuid;
use wait_timeout::ChildExt;
use zip::ZipArchive;

use crate::bootstrap::{fetch_bootstrap, load_seed_or_remote, release_origin, write_seed_copy};
use crate::manifest::{
    AssetRef, Bootstrap, ReleaseCatalog, ReleaseManifest, RuntimeComponent, compare_versions,
    is_safe_relative_path,
};
use crate::paths::AppPaths;

pub const MAX_ARTIFACT_BYTES: u64 = 768 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;
pub const MAX_EXTRACTED_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_CATALOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CATALOG_RELEASES: usize = 4096;
const BRIDGE_PATCH_FILE: &str = "desktop-bridge.patch.yml";
const BRIDGE_SCRIPT_FILE: &str = "desktop-bridge.mjs";
pub const LAUNCHER_VERSION: &str = env!("DSH_LAUNCHER_VERSION");

const DEFAULT_NPM_REGISTRY: &str = "https://registry.npmjs.org/";
const DEFAULT_NPM_PACKAGE: &str = "@deepseek-ai/dsh";
const MAX_NPM_PACKUMENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_NPM_TARBALL_BYTES: u64 = MAX_ARTIFACT_BYTES;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeSource {
    Local,
    #[default]
    Oss,
    Npm,
}

impl RuntimeSource {
    fn rank(self) -> u8 {
        match self {
            Self::Local => 0,
            Self::Oss => 1,
            Self::Npm => 2,
        }
    }

    fn directory_suffix(self) -> &'static str {
        match self {
            Self::Local | Self::Oss => "",
            Self::Npm => "-npm",
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
    fn accepts(self, source: RuntimeSource) -> bool {
        matches!(
            (self, source),
            (Self::Auto, _)
                | (Self::Local, RuntimeSource::Local)
                | (Self::Oss, RuntimeSource::Oss)
                | (Self::Npm, RuntimeSource::Npm)
        )
    }

    fn is_fixed(self) -> bool {
        !matches!(self, Self::Auto)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Local => "local",
            Self::Oss => "oss",
            Self::Npm => "npm",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NpmUpdateSettings {
    #[serde(default = "default_npm_registry")]
    pub registry: String,
    #[serde(default = "default_npm_package")]
    pub package: String,
}

impl Default for NpmUpdateSettings {
    fn default() -> Self {
        Self {
            registry: default_npm_registry(),
            package: default_npm_package(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateSettings {
    #[serde(default)]
    pub source: UpdateSourcePolicy,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub npm: NpmUpdateSettings,
}

impl Default for RuntimeUpdateSettings {
    fn default() -> Self {
        Self {
            source: UpdateSourcePolicy::Auto,
            version: None,
            npm: NpmUpdateSettings::default(),
        }
    }
}

impl RuntimeUpdateSettings {
    fn validate(&self) -> Result<()> {
        if let Some(version) = &self.version {
            compare_versions(version, "0.0.0")
                .map_err(|error| anyhow!("invalid configured runtime version: {error}"))?;
        }
        let registry =
            Url::parse(&self.npm.registry).context("configured npm registry is not a URL")?;
        ensure!(
            registry.host_str().is_some(),
            "configured npm registry has no host"
        );
        ensure!(
            self.npm.package == DEFAULT_NPM_PACKAGE,
            "only the authorized @deepseek-ai/dsh npm package is supported"
        );
        Ok(())
    }
}

fn default_npm_registry() -> String {
    DEFAULT_NPM_REGISTRY.into()
}

fn default_npm_package() -> String {
    DEFAULT_NPM_PACKAGE.into()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRelease {
    pub release: String,
    pub manifest_sha256: String,
    #[serde(default)]
    pub manifest: Option<ReleaseManifest>,
    #[serde(default)]
    pub manifest_snapshot_sha256: Option<String>,
    #[serde(default)]
    pub source: RuntimeSource,
    #[serde(default)]
    pub source_identity: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StagedRelease {
    pub release: String,
    pub manifest_sha256: String,
    pub manifest: ReleaseManifest,
    pub manifest_snapshot_sha256: String,
    #[serde(default)]
    pub source: RuntimeSource,
    #[serde(default)]
    pub source_identity: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepairRecord {
    pub release: String,
    pub phase: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct PreparedRuntime {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub release: String,
    pub source: RuntimeSource,
}

#[derive(Debug, Clone)]
pub struct RuntimeManager {
    paths: AppPaths,
    client: Client,
}

#[derive(Debug, Clone)]
struct RuntimeCandidate {
    source: RuntimeSource,
    release: String,
    minimum_launcher: String,
    manifest: AssetRef,
    manifest_value: Option<ReleaseManifest>,
    source_identity: String,
    acquisition: CandidateAcquisition,
}

#[derive(Debug, Clone)]
enum CandidateAcquisition {
    Local {
        root: PathBuf,
    },
    Oss {
        origin: Url,
    },
    Npm {
        descriptor: NpmRuntimeDescriptor,
        tarball: Url,
        integrity: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NpmRuntimeDescriptor {
    manifest: ReleaseManifest,
    #[serde(default = "default_npm_package_root")]
    package_root: String,
}

fn default_npm_package_root() -> String {
    "package".into()
}

#[derive(Debug, Clone, Deserialize)]
struct NpmPackument {
    versions: BTreeMap<String, NpmPackageVersion>,
}

#[derive(Debug, Clone, Deserialize)]
struct NpmPackageVersion {
    version: String,
    dist: NpmDistribution,
    #[serde(rename = "dshDesktopRuntime")]
    runtime: Option<NpmRuntimeDescriptor>,
}

#[derive(Debug, Clone, Deserialize)]
struct NpmDistribution {
    tarball: Url,
    #[serde(default)]
    integrity: Option<String>,
}

impl RuntimeManager {
    pub fn new(paths: AppPaths) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_mins(1))
            .build()
            .context("cannot create HTTP client")?;
        Ok(Self { paths, client })
    }

    fn load_oss_candidates(
        &self,
        origin: &Url,
        bootstrap: &Bootstrap,
    ) -> Result<Vec<RuntimeCandidate>> {
        if let Some(catalog_asset) = &bootstrap.catalog {
            ensure!(
                catalog_asset.bytes <= MAX_CATALOG_BYTES,
                "runtime catalog exceeds the client maximum size"
            );
            let catalog_bytes = self.download_verified(origin, catalog_asset)?;
            let catalog: ReleaseCatalog = serde_json::from_slice(&catalog_bytes)
                .context("runtime catalog JSON is invalid")?;
            catalog.validate().context("runtime catalog is invalid")?;
            ensure!(
                catalog.releases.len() <= MAX_CATALOG_RELEASES,
                "runtime catalog contains too many releases"
            );
            return Ok(catalog
                .releases
                .into_iter()
                .map(|entry| RuntimeCandidate {
                    source: RuntimeSource::Oss,
                    release: entry.release,
                    minimum_launcher: entry.minimum_launcher,
                    source_identity: entry.manifest.sha256.clone(),
                    manifest: entry.manifest,
                    manifest_value: None,
                    acquisition: CandidateAcquisition::Oss {
                        origin: origin.clone(),
                    },
                })
                .collect());
        }

        Ok(vec![RuntimeCandidate {
            source: RuntimeSource::Oss,
            release: bootstrap.release.clone(),
            minimum_launcher: bootstrap.minimum_launcher.clone(),
            source_identity: bootstrap.manifest.sha256.clone(),
            manifest: bootstrap.manifest.clone(),
            manifest_value: None,
            acquisition: CandidateAcquisition::Oss {
                origin: origin.clone(),
            },
        }])
    }

    fn load_local_candidates(&self) -> Result<Vec<RuntimeCandidate>> {
        let mut candidates = Vec::new();
        for pointer in [
            self.read_current()?,
            self.read_staged()?.map(|staged| CurrentRelease {
                release: staged.release,
                manifest_sha256: staged.manifest_sha256,
                manifest: Some(staged.manifest),
                manifest_snapshot_sha256: Some(staged.manifest_snapshot_sha256),
                source: staged.source,
                source_identity: staged.source_identity,
            }),
        ] {
            let Some(pointer) = pointer else {
                continue;
            };
            let Some(manifest) = pointer.manifest.clone() else {
                continue;
            };
            if !current_snapshot_is_valid(&pointer, &manifest) {
                continue;
            }
            let root = self.runtime_root(&pointer.release, pointer.source);
            if !root.is_dir()
                || !bridge_files_match(&root)
                || Self::run_doctors(&manifest, &root).is_err()
            {
                continue;
            }
            let source_identity = pointer
                .source_identity
                .clone()
                .unwrap_or_else(|| pointer.manifest_sha256.clone());
            candidates.push(RuntimeCandidate {
                source: RuntimeSource::Local,
                release: pointer.release.clone(),
                minimum_launcher: manifest.minimum_launcher.clone(),
                manifest: AssetRef {
                    object_key: format!("local/{}", manifest.release),
                    bytes: 1,
                    sha256: pointer.manifest_sha256.clone(),
                },
                manifest_value: Some(manifest),
                source_identity,
                acquisition: CandidateAcquisition::Local { root },
            });
        }
        Ok(candidates)
    }

    fn load_npm_packument(&self, settings: &NpmUpdateSettings) -> Result<NpmPackument> {
        let registry = Url::parse(&settings.registry).context("npm registry is not a URL")?;
        let encoded_package = settings.package.replace('/', "%2f");
        let url = registry
            .join(&encoded_package)
            .context("cannot resolve npm package metadata URL")?;
        let response = self
            .client
            .get(url)
            .send()
            .context("npm registry metadata request failed")?
            .error_for_status()
            .context("npm registry metadata request failed")?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_NPM_PACKUMENT_BYTES)
        {
            bail!("npm registry metadata exceeds the client maximum size");
        }
        let body = response
            .bytes()
            .context("cannot read npm registry metadata")?;
        ensure!(
            u64::try_from(body.len()).unwrap_or(u64::MAX) <= MAX_NPM_PACKUMENT_BYTES,
            "npm registry metadata exceeds the client maximum size"
        );
        serde_json::from_slice(&body).context("npm registry metadata JSON is invalid")
    }

    fn load_npm_candidates(&self, settings: &NpmUpdateSettings) -> Result<Vec<RuntimeCandidate>> {
        let packument = self.load_npm_packument(settings)?;
        let mut candidates = Vec::new();
        for (version, entry) in packument.versions {
            if version != entry.version {
                continue;
            }
            let Some(descriptor) = entry.runtime else {
                continue;
            };
            if !is_safe_relative_path(&descriptor.package_root)
                || Path::new(&descriptor.package_root).components().count() != 1
                || descriptor.manifest.validate().is_err()
                || descriptor.manifest.release != version
            {
                continue;
            }
            let Ok(source_identity) = npm_distribution_identity(&entry.dist) else {
                continue;
            };
            let Ok(manifest) = npm_manifest_asset(&descriptor.manifest) else {
                continue;
            };
            candidates.push(RuntimeCandidate {
                source: RuntimeSource::Npm,
                release: version,
                minimum_launcher: descriptor.manifest.minimum_launcher.clone(),
                manifest,
                manifest_value: Some(descriptor.manifest.clone()),
                source_identity,
                acquisition: CandidateAcquisition::Npm {
                    descriptor,
                    tarball: entry.dist.tarball,
                    integrity: entry.dist.integrity,
                },
            });
        }
        Ok(candidates)
    }

    fn load_npm_versions(&self, settings: &NpmUpdateSettings) -> Result<Vec<String>> {
        Ok(npm_version_names(self.load_npm_packument(settings)?))
    }

    fn discover_candidates(
        &self,
        settings: &RuntimeUpdateSettings,
        current: Option<&CurrentRelease>,
    ) -> Result<Vec<RuntimeCandidate>> {
        settings.validate()?;
        let mut candidates = Vec::new();
        if !settings.source.is_fixed() || matches!(settings.source, UpdateSourcePolicy::Local) {
            candidates.extend(self.load_local_candidates()?);
        }
        let origin = release_origin()?;
        if !settings.source.is_fixed() || matches!(settings.source, UpdateSourcePolicy::Oss) {
            let bootstrap_result = if current.is_some() {
                fetch_bootstrap(&self.client, &origin)
            } else {
                load_seed_or_remote(&self.client, &origin)
            };
            match bootstrap_result {
                Ok(bootstrap) => candidates.extend(self.load_oss_candidates(&origin, &bootstrap)?),
                Err(error) if settings.source.is_fixed() => return Err(error),
                Err(_) => {}
            }
        }
        if !settings.source.is_fixed() || matches!(settings.source, UpdateSourcePolicy::Npm) {
            match self.load_npm_candidates(&settings.npm) {
                Ok(npm) => candidates.extend(npm),
                Err(error) if settings.source.is_fixed() => return Err(error),
                Err(_) => {}
            }
        }
        Ok(candidates)
    }

    fn select_candidate(
        &self,
        settings: &RuntimeUpdateSettings,
        current: Option<&CurrentRelease>,
    ) -> Result<Option<RuntimeCandidate>> {
        let candidates = self.discover_candidates(settings, current)?;
        validate_requested_candidate(&candidates, current, settings)?;
        Ok(choose_candidate(candidates, current, settings))
    }

    fn runtime_root(&self, release: &str, source: RuntimeSource) -> PathBuf {
        self.paths
            .runtimes
            .join(format!("{release}{}", source.directory_suffix()))
    }
}

fn validate_requested_candidate(
    candidates: &[RuntimeCandidate],
    current: Option<&CurrentRelease>,
    settings: &RuntimeUpdateSettings,
) -> Result<()> {
    let Some(version) = settings.version.as_deref() else {
        return Ok(());
    };
    let matching = candidates
        .iter()
        .filter(|candidate| {
            settings.source.accepts(candidate.source) && candidate.release == version
        })
        .collect::<Vec<_>>();
    ensure!(
        !matching.is_empty(),
        "selected runtime {version} is unavailable from source policy {}; the source may not provide a complete runtime closure",
        settings.source.label()
    );
    ensure!(
        matching.iter().any(|candidate| {
            compare_versions(&candidate.minimum_launcher, LAUNCHER_VERSION)
                .is_ok_and(|ordering| !ordering.is_gt())
        }),
        "selected runtime {version} requires a newer Launcher"
    );
    if let Some(current) = current
        && compare_versions(version, &current.release).is_ok_and(std::cmp::Ordering::is_lt)
    {
        bail!(
            "selected runtime {version} is older than current runtime {}",
            current.release
        );
    }
    Ok(())
}

fn choose_candidate(
    mut candidates: Vec<RuntimeCandidate>,
    current: Option<&CurrentRelease>,
    settings: &RuntimeUpdateSettings,
) -> Option<RuntimeCandidate> {
    candidates.retain(|candidate| settings.source.accepts(candidate.source));
    if let Some(version) = &settings.version {
        candidates.retain(|candidate| candidate.release == *version);
    }
    candidates.retain(|candidate| {
        compare_versions(&candidate.minimum_launcher, LAUNCHER_VERSION)
            .is_ok_and(|ordering| !ordering.is_gt())
    });
    candidates.retain(|candidate| {
        current.is_none_or(|current| {
            compare_versions(&candidate.release, &current.release)
                .is_ok_and(|ordering| !ordering.is_lt())
        })
    });
    candidates.sort_by(|left, right| {
        compare_versions(&left.release, &right.release)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.source.rank().cmp(&left.source.rank()))
    });

    let candidate = candidates.pop()?;
    if current.is_some_and(|current| {
        current.release == candidate.release
            && current_identity(current).eq_ignore_ascii_case(&candidate.source_identity)
    }) {
        return None;
    }
    Some(candidate)
}

fn current_identity(current: &CurrentRelease) -> &str {
    current
        .source_identity
        .as_deref()
        .unwrap_or(&current.manifest_sha256)
}

fn staged_identity(staged: &StagedRelease) -> &str {
    staged
        .source_identity
        .as_deref()
        .unwrap_or(&staged.manifest_sha256)
}

fn npm_distribution_identity(distribution: &NpmDistribution) -> Result<String> {
    let Some(integrity) = &distribution.integrity else {
        bail!("npm runtime package requires sha512 integrity metadata");
    };
    let Some(encoded) = integrity.strip_prefix("sha512-") else {
        bail!("npm runtime integrity must use sha512");
    };
    let decoded = base64_decode(encoded).context("npm runtime integrity is invalid")?;
    ensure!(decoded.len() == 64, "npm runtime integrity is not sha512");
    Ok(format!("npm-sha512-{}", integrity.trim()))
}

fn npm_manifest_asset(manifest: &ReleaseManifest) -> Result<AssetRef> {
    let bytes = serde_json::to_vec(manifest).context("cannot serialize npm runtime manifest")?;
    Ok(AssetRef {
        object_key: format!("npm/{}/manifest.json", manifest.release),
        bytes: u64::try_from(bytes.len()).context("npm runtime manifest is too large")?,
        sha256: hex::encode(Sha256::digest(bytes)),
    })
}

fn base64_decode(value: &str) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(value.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    for byte in value.bytes() {
        let six = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => bail!("invalid base64 character"),
        };
        buffer = (buffer << 6) | u32::from(six);
        bits = bits.saturating_add(6);
        if bits >= 8 {
            bits -= 8;
            output.push(u8::try_from(buffer >> bits).context("base64 decode overflow")?);
            buffer &= (1_u32 << bits) - 1;
        }
    }
    Ok(output)
}

impl RuntimeManager {
    #[allow(clippy::too_many_lines)]
    pub fn prepare_current(&self) -> Result<PreparedRuntime> {
        self.paths.create()?;
        write_seed_copy(&self.paths.launcher.join("bootstrap.windows-x64.seed.json"))?;
        let current = self.read_current()?;
        let settings = self.read_settings()?;
        let selected = (|| -> Result<Option<(RuntimeCandidate, ReleaseManifest)>> {
            let Some(candidate) = self.select_candidate(&settings, current.as_ref())? else {
                return Ok(None);
            };
            let manifest = self.load_candidate_manifest(&candidate)?;
            Ok(Some((candidate, manifest)))
        })();
        let (candidate, manifest) = match selected {
            Ok(Some(selected)) => selected,
            Ok(None) => {
                if let Some(prepared) = self.prepare_existing_current(current.as_ref()) {
                    return Ok(prepared);
                }
                return Err(anyhow!("no compatible runtime candidate is available"));
            }
            Err(remote_error) => {
                if let Some(prepared) = self.prepare_existing_current(current.as_ref()) {
                    return Ok(prepared);
                }
                return Err(remote_error)
                    .context("runtime source is unavailable and no usable current runtime exists");
            }
        };
        ensure_no_downgrade(current.as_ref(), &manifest.release)?;
        let target_root = self.runtime_root(&manifest.release, candidate.source);
        let pointer_matches = current.as_ref().is_some_and(|release| {
            release.release == manifest.release
                && current_identity(release).eq_ignore_ascii_case(&candidate.source_identity)
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
            self.stage_candidate(&candidate, &manifest, &staging)?;
            materialize_bridge_files(&staging)?;
            Self::run_doctors(&manifest, &staging)?;
            Ok(())
        })();
        if let Err(error) = staged {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        fs::rename(&staging, &target_root).context("cannot activate runtime directory")?;
        let activated = (|| -> Result<()> {
            materialize_bridge_files(&target_root)?;
            Self::run_doctors(&manifest, &target_root)
        })();
        if let Err(error) = activated {
            let _ = fs::remove_dir_all(&target_root);
            return Err(error).context("activated runtime failed bridge or doctor validation");
        }
        let pointer = CurrentRelease {
            release: manifest.release.clone(),
            manifest_sha256: candidate.manifest.sha256,
            manifest_snapshot_sha256: Some(manifest_snapshot_sha256(&manifest)?),
            manifest: Some(manifest),
            source: candidate.source,
            source_identity: Some(candidate.source_identity),
        };
        if let Err(error) = self.write_current(&pointer) {
            let _ = fs::remove_dir_all(&target_root);
            return Err(error);
        }
        Ok(PreparedRuntime { root: target_root })
    }

    fn load_candidate_manifest(&self, candidate: &RuntimeCandidate) -> Result<ReleaseManifest> {
        let manifest = if let Some(manifest) = &candidate.manifest_value {
            manifest.clone()
        } else {
            let CandidateAcquisition::Oss { origin } = &candidate.acquisition else {
                bail!("runtime candidate has no manifest source");
            };
            let manifest_bytes = self.download_verified(origin, &candidate.manifest)?;
            serde_json::from_slice(&manifest_bytes).context("runtime manifest JSON is invalid")?
        };
        manifest.validate().context("runtime manifest is invalid")?;
        ensure_launcher_compatible(&manifest.minimum_launcher)?;
        ensure!(
            manifest.release == candidate.release
                && manifest.minimum_launcher == candidate.minimum_launcher,
            "runtime candidate does not match its manifest"
        );
        Ok(manifest)
    }

    fn stage_candidate(
        &self,
        candidate: &RuntimeCandidate,
        manifest: &ReleaseManifest,
        staging: &Path,
    ) -> Result<()> {
        match &candidate.acquisition {
            CandidateAcquisition::Local { root } => {
                copy_runtime_tree(root, staging)?;
            }
            CandidateAcquisition::Oss { origin } => {
                for component in &manifest.components {
                    self.stage_component(origin, component, staging)?;
                }
            }
            CandidateAcquisition::Npm { descriptor, .. } => {
                self.stage_npm_runtime(candidate, descriptor, staging)?;
            }
        }
        Ok(())
    }

    fn stage_npm_runtime(
        &self,
        candidate: &RuntimeCandidate,
        descriptor: &NpmRuntimeDescriptor,
        staging: &Path,
    ) -> Result<()> {
        let CandidateAcquisition::Npm {
            tarball, integrity, ..
        } = &candidate.acquisition
        else {
            bail!("not an npm runtime candidate");
        };
        let bytes = self.download_npm_tarball(tarball, integrity.as_deref())?;
        extract_npm_archive(&bytes, staging, &descriptor.package_root)?;
        ensure!(
            staging.join("package.json").is_file()
                || staging.join("node.exe").is_file()
                || staging.join("node/node.exe").is_file(),
            "npm runtime package does not contain a complete runtime closure"
        );
        Ok(())
    }

    fn download_npm_tarball(&self, url: &Url, integrity: Option<&str>) -> Result<Vec<u8>> {
        let mut response = self
            .client
            .get(url.clone())
            .send()
            .context("npm runtime package download failed")?
            .error_for_status()
            .context("npm runtime package request failed")?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_NPM_TARBALL_BYTES)
        {
            bail!("npm runtime package exceeds the client maximum size");
        }
        let mut bytes = Vec::new();
        response
            .read_to_end(&mut bytes)
            .context("cannot read npm runtime package")?;
        ensure!(
            u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= MAX_NPM_TARBALL_BYTES,
            "npm runtime package exceeds the client maximum size"
        );
        let Some(integrity) = integrity else {
            bail!("npm runtime package requires sha512 integrity metadata");
        };
        verify_npm_integrity(&bytes, integrity)?;
        Ok(bytes)
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
        let target_root = self.runtime_root(&current.release, current.source);
        if !target_root.is_dir() || !bridge_files_match(&target_root) {
            return None;
        }
        if Self::run_doctors(manifest, &target_root).is_ok() {
            return Some(PreparedRuntime { root: target_root });
        }
        None
    }

    pub fn read_settings(&self) -> Result<RuntimeUpdateSettings> {
        let Some(settings) = read_json_state::<RuntimeUpdateSettings>(
            &self.paths.settings_path(),
            "client settings",
        )?
        else {
            let settings = RuntimeUpdateSettings::default();
            self.write_settings(&settings)?;
            return Ok(settings);
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn write_settings(&self, settings: &RuntimeUpdateSettings) -> Result<()> {
        settings.validate()?;
        self.paths.create()?;
        let pointer = self.paths.settings_path();
        let temporary = pointer.with_extension(format!("{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, serde_json::to_vec_pretty(settings)?)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        atomic_replace(&temporary, &pointer)
            .with_context(|| format!("cannot publish {}", pointer.display()))
    }

    pub fn check_for_update(&self) -> Result<Option<AvailableUpdate>> {
        let current = self.read_current()?;
        let settings = self.read_settings()?;
        Ok(self
            .select_candidate(&settings, current.as_ref())?
            .map(|candidate| AvailableUpdate {
                release: candidate.release,
                source: candidate.source,
            }))
    }

    /// Lists every runtime version visible to the current source policy.
    ///
    /// This is intentionally broader than `check_for_update`: the native UI needs to
    /// let users choose a concrete source/version pair. Complete closure and doctor
    /// validation remains enforced when the selected version is staged.
    pub fn list_available_versions(&self) -> Result<Vec<AvailableUpdate>> {
        let current = self.read_current()?;
        let settings = self.read_settings()?;
        let candidates = self.discover_candidates(&settings, current.as_ref())?;
        let mut versions = list_compatible_versions(candidates, current.as_ref(), &settings);
        if settings.source.accepts(RuntimeSource::Npm) {
            let npm_versions = match self.load_npm_versions(&settings.npm) {
                Ok(versions) => versions,
                Err(error) if settings.source.is_fixed() => return Err(error),
                Err(_) => Vec::new(),
            };
            for release in npm_versions {
                if current.as_ref().is_some_and(|current| {
                    compare_versions(&release, &current.release).is_ok_and(Ordering::is_lt)
                }) {
                    continue;
                }
                if !versions.iter().any(|version| {
                    version.release == release && version.source == RuntimeSource::Npm
                }) {
                    versions.push(AvailableUpdate {
                        release,
                        source: RuntimeSource::Npm,
                    });
                }
            }
            versions.sort_by(|left, right| {
                compare_versions(&right.release, &left.release)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| right.source.rank().cmp(&left.source.rank()))
            });
        }
        Ok(versions)
    }

    /// Download, verify, unpack, and doctor a compatible runtime without changing current.
    pub fn stage_update(&self) -> Result<Option<StagedRelease>> {
        self.paths.create()?;
        let current = self.read_current()?;
        let settings = self.read_settings()?;
        let Some(candidate) = self.select_candidate(&settings, current.as_ref())? else {
            let _ = fs::remove_file(self.paths.staged_pointer());
            return Ok(None);
        };

        let manifest = self.load_candidate_manifest(&candidate)?;

        if current.as_ref().is_some_and(|current| {
            current.release == manifest.release
                && current_identity(current).eq_ignore_ascii_case(&candidate.source_identity)
        }) {
            let _ = fs::remove_file(self.paths.staged_pointer());
            return Ok(None);
        }

        let snapshot = manifest_snapshot_sha256(&manifest)?;
        if let Some(staged) = self.read_staged()? {
            if staged.release == manifest.release
                && staged_identity(&staged).eq_ignore_ascii_case(&candidate.source_identity)
                && staged
                    .manifest_snapshot_sha256
                    .eq_ignore_ascii_case(&snapshot)
            {
                let target_root = self.runtime_root(&staged.release, staged.source);
                if target_root.is_dir()
                    && staged_snapshot_is_valid(&staged)
                    && bridge_files_match(&target_root)
                    && Self::run_doctors(&staged.manifest, &target_root).is_ok()
                {
                    return Ok(Some(staged));
                }
            }
            let _ = fs::remove_file(self.paths.staged_pointer());
        }

        let target_root = self.runtime_root(&manifest.release, candidate.source);
        if target_root.exists() {
            ensure!(
                target_root.is_dir() && bridge_files_match(&target_root),
                "runtime directory collision for release {}",
                manifest.release
            );
            Self::run_doctors(&manifest, &target_root)
                .context("existing runtime candidate failed doctor")?;
        } else {
            let staging =
                self.paths
                    .staging
                    .join(format!("{}-{}", manifest.release, Uuid::new_v4()));
            fs::create_dir_all(&staging)
                .with_context(|| format!("cannot create staging root {}", staging.display()))?;
            let staged = (|| -> Result<()> {
                self.stage_candidate(&candidate, &manifest, &staging)?;
                materialize_bridge_files(&staging)?;
                Self::run_doctors(&manifest, &staging)?;
                Ok(())
            })();
            if let Err(error) = staged {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
            if let Err(error) = fs::rename(&staging, &target_root) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error).context("cannot publish staged runtime directory");
            }
            let activated = (|| -> Result<()> {
                materialize_bridge_files(&target_root)?;
                Self::run_doctors(&manifest, &target_root)
            })();
            if let Err(error) = activated {
                let _ = fs::remove_dir_all(&target_root);
                return Err(error).context("activated runtime failed bridge or doctor validation");
            }
        }

        let staged = StagedRelease {
            release: manifest.release.clone(),
            manifest_sha256: candidate.manifest.sha256,
            manifest,
            manifest_snapshot_sha256: snapshot,
            source: candidate.source,
            source_identity: Some(candidate.source_identity),
        };
        self.write_staged(&staged)?;
        Ok(Some(staged))
    }

    /// Atomically promote a previously doctored staged runtime to current.
    pub fn activate_staged(&self) -> Result<PreparedRuntime> {
        self.activate_staged_with(Self::run_doctors)
    }

    fn activate_staged_with<F>(&self, validator: F) -> Result<PreparedRuntime>
    where
        F: Fn(&ReleaseManifest, &Path) -> Result<()>,
    {
        let staged = self
            .read_staged()?
            .context("no staged runtime is available")?;
        let current = self.read_current()?;
        ensure_no_downgrade(current.as_ref(), &staged.release)?;
        ensure!(
            staged_snapshot_is_valid(&staged),
            "staged runtime pointer is corrupt"
        );
        let target_root = self.runtime_root(&staged.release, staged.source);
        ensure!(
            target_root.is_dir() && bridge_files_match(&target_root),
            "staged runtime directory is missing or invalid"
        );
        validator(&staged.manifest, &target_root).context("staged runtime doctor failed")?;

        let pointer = CurrentRelease {
            release: staged.release.clone(),
            manifest_sha256: staged.manifest_sha256.clone(),
            manifest_snapshot_sha256: Some(staged.manifest_snapshot_sha256.clone()),
            manifest: Some(staged.manifest.clone()),
            source: staged.source,
            source_identity: staged.source_identity.clone(),
        };
        self.write_current(&pointer)?;
        let _ = fs::remove_file(self.paths.staged_pointer());
        let _ = self.clear_repair();
        Ok(PreparedRuntime { root: target_root })
    }

    pub fn record_forward_repair(
        &self,
        release: impl Into<String>,
        phase: impl Into<String>,
        error: impl Into<String>,
    ) -> Result<()> {
        self.paths.create()?;
        let record = RepairRecord {
            release: release.into(),
            phase: phase.into(),
            error: error.into(),
        };
        let temporary = self
            .paths
            .repair_record()
            .with_extension(format!("{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, serde_json::to_vec_pretty(&record)?)
            .with_context(|| format!("cannot write repair record {}", temporary.display()))?;
        atomic_replace(&temporary, &self.paths.repair_record())
            .context("cannot publish repair record")
    }

    pub fn read_repair(&self) -> Result<Option<RepairRecord>> {
        read_json_state(&self.paths.repair_record(), "repair record")
    }

    pub fn read_staged(&self) -> Result<Option<StagedRelease>> {
        read_json_state(&self.paths.staged_pointer(), "staged runtime pointer")
    }

    pub fn clear_repair(&self) -> Result<()> {
        let path = self.paths.repair_record();
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("cannot remove repair record {}", path.display()))?;
        }
        Ok(())
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
        read_json_state(&pointer, "current runtime pointer")
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

    fn write_staged(&self, staged: &StagedRelease) -> Result<()> {
        let pointer = self.paths.staged_pointer();
        let temporary = pointer.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let content =
            serde_json::to_vec_pretty(staged).context("cannot serialize staged pointer")?;
        fs::write(&temporary, content)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        atomic_replace(&temporary, &pointer)
            .with_context(|| format!("cannot publish {}", pointer.display()))
    }
}

fn list_compatible_versions(
    candidates: Vec<RuntimeCandidate>,
    current: Option<&CurrentRelease>,
    settings: &RuntimeUpdateSettings,
) -> Vec<AvailableUpdate> {
    let mut seen = HashSet::new();
    let mut versions = candidates
        .into_iter()
        .filter(|candidate| settings.source.accepts(candidate.source))
        .filter(|candidate| {
            compare_versions(&candidate.minimum_launcher, LAUNCHER_VERSION)
                .is_ok_and(|ordering| !ordering.is_gt())
        })
        .filter(|candidate| {
            current.is_none_or(|current| {
                compare_versions(&candidate.release, &current.release)
                    .is_ok_and(|ordering| !ordering.is_lt())
            })
        })
        .filter_map(|candidate| {
            if !seen.insert((candidate.release.clone(), candidate.source)) {
                return None;
            }
            Some(AvailableUpdate {
                release: candidate.release,
                source: candidate.source,
            })
        })
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| {
        compare_versions(&right.release, &left.release)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.source.rank().cmp(&left.source.rank()))
    });
    versions
}

fn npm_version_names(packument: NpmPackument) -> Vec<String> {
    packument
        .versions
        .into_iter()
        .filter_map(|(version, entry)| {
            (version == entry.version && compare_versions(&version, "0.0.0").is_ok())
                .then_some(version)
        })
        .collect()
}

fn read_json_state<T>(path: &Path, label: &str) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .with_context(|| format!("{label} is corrupt"))
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

fn staged_snapshot_is_valid(staged: &StagedRelease) -> bool {
    staged.manifest.validate().is_ok()
        && staged.release == staged.manifest.release
        && staged.manifest_sha256.len() == crate::manifest::SHA256_HEX_LENGTH
        && staged
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && manifest_snapshot_sha256(&staged.manifest).is_ok_and(|actual| {
            staged
                .manifest_snapshot_sha256
                .eq_ignore_ascii_case(&actual)
        })
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

fn verify_npm_integrity(bytes: &[u8], integrity: &str) -> Result<()> {
    let encoded = integrity
        .strip_prefix("sha512-")
        .context("npm runtime integrity must use sha512")?;
    let expected = base64_decode(encoded).context("npm runtime integrity is invalid")?;
    ensure!(expected.len() == 64, "npm runtime integrity is not sha512");
    ensure!(
        Sha512::digest(bytes).as_slice() == expected.as_slice(),
        "npm runtime package integrity mismatch"
    );
    Ok(())
}

fn copy_runtime_tree(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)
        .with_context(|| format!("cannot enumerate local runtime {}", source.display()))?
    {
        let entry = entry.context("cannot inspect local runtime entry")?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)
            .with_context(|| format!("cannot inspect {}", source_path.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "local runtime contains a symbolic link"
        );
        if metadata.is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("cannot create {}", target_path.display()))?;
            copy_runtime_tree(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)
                .with_context(|| format!("cannot copy {}", source_path.display()))?;
        }
    }
    Ok(())
}

fn extract_npm_archive(bytes: &[u8], root: &Path, package_root: &str) -> Result<()> {
    ensure!(
        is_safe_relative_path(package_root),
        "unsafe npm package root"
    );
    ensure!(
        Path::new(package_root).components().count() == 1,
        "npm package root must be one path component"
    );
    let decoder = GzDecoder::new(io::Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    let mut names = HashSet::new();
    let mut extracted_bytes = 0_u64;
    for entry in archive
        .entries()
        .context("cannot inspect npm package archive")?
    {
        let mut entry = entry.context("cannot inspect npm package entry")?;
        let path = entry
            .path()
            .context("cannot read npm package path")?
            .to_path_buf();
        let components = path.components().collect::<Vec<_>>();
        ensure!(
            !components.is_empty()
                && components[0] == Component::Normal(std::ffi::OsStr::new(package_root))
                && components[1..]
                    .iter()
                    .all(|component| matches!(component, Component::Normal(_))),
            "unsafe npm package path {}",
            path.display()
        );
        if components.len() == 1 {
            ensure!(
                entry.header().entry_type().is_dir(),
                "npm package root is not a directory"
            );
            continue;
        }
        let relative = components[1..]
            .iter()
            .fold(PathBuf::new(), |mut path, component| {
                path.push(component.as_os_str());
                path
            });
        let target = root.join(relative);
        ensure!(names.insert(target.clone()), "duplicate npm package entry");
        ensure!(target.starts_with(root), "npm package path escapes staging");
        let entry_type = entry.header().entry_type();
        ensure!(
            !entry_type.is_symlink()
                && !entry_type.is_hard_link()
                && !entry_type.is_character_special()
                && !entry_type.is_block_special()
                && !entry_type.is_fifo(),
            "npm package links and special files are not accepted"
        );
        if entry_type.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("cannot create {}", target.display()))?;
            continue;
        }
        let declared_size = entry
            .header()
            .size()
            .context("npm package entry has no size")?;
        ensure!(
            declared_size <= MAX_EXTRACTED_FILE_BYTES,
            "npm package entry exceeds per-file limit"
        );
        extracted_bytes = extracted_bytes
            .checked_add(declared_size)
            .context("npm package exceeds extracted size limit")?;
        ensure!(
            extracted_bytes <= MAX_EXTRACTED_BYTES,
            "npm package exceeds extracted size limit"
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
        let copied = io::copy(&mut entry, &mut output)
            .with_context(|| format!("cannot extract {}", target.display()))?;
        ensure!(copied == declared_size, "npm package entry size mismatch");
        output.flush().context("cannot flush npm package entry")?;
    }
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
    let Ok(expected_patch) = rendered_bridge_patch(root) else {
        return false;
    };
    fs::read(root.join(BRIDGE_PATCH_FILE)).is_ok_and(|actual| actual == expected_patch)
        && fs::read(root.join(BRIDGE_SCRIPT_FILE))
            .is_ok_and(|actual| actual == include_bytes!("../resources/desktop-bridge.mjs"))
}

fn materialize_bridge_files(root: &Path) -> Result<()> {
    let script_target = root.join(BRIDGE_SCRIPT_FILE);
    let script = include_bytes!("../resources/desktop-bridge.mjs");
    if script_target.exists() {
        ensure!(
            fs::read(&script_target)
                .with_context(|| format!("cannot read {}", script_target.display()))?
                == script,
            "runtime contains an unexpected desktop bridge file {}",
            script_target.display()
        );
    } else {
        write_new_file(&script_target, script)?;
    }

    // The patch names this root's bridge through an absolute file URL. It must be
    // rewritten after staging is atomically renamed to its final runtime directory.
    write_replacing_file(&root.join(BRIDGE_PATCH_FILE), &rendered_bridge_patch(root)?)
}

fn rendered_bridge_patch(root: &Path) -> Result<Vec<u8>> {
    const BRIDGE_MODULE_PLACEHOLDER: &str = "__DSH_DESKTOP_BRIDGE_MODULE__";
    let template = std::str::from_utf8(include_bytes!("../resources/desktop-bridge.patch.yml"))
        .context("desktop bridge patch template is not UTF-8")?;
    ensure!(
        template.matches(BRIDGE_MODULE_PLACEHOLDER).count() == 1,
        "desktop bridge patch template must contain exactly one module placeholder"
    );
    let bridge_url = Url::from_file_path(root.join(BRIDGE_SCRIPT_FILE))
        .map_err(|()| anyhow::anyhow!("cannot render desktop bridge file URL"))?;
    Ok(template
        .replace(BRIDGE_MODULE_PLACEHOLDER, bridge_url.as_str())
        .into_bytes())
}

fn write_new_file(target: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .with_context(|| format!("cannot create {}", target.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", target.display()))?;
    file.flush()
        .with_context(|| format!("cannot flush {}", target.display()))
}

fn write_replacing_file(target: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(target)
        .with_context(|| format!("cannot open {}", target.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", target.display()))?;
    file.flush()
        .with_context(|| format!("cannot flush {}", target.display()))
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
    fn bridge_patch_is_rerendered_after_runtime_directory_rename() {
        let parent = tempdir().expect("temp");
        let staging = parent.path().join("staging");
        let runtime = parent.path().join("runtime");
        fs::create_dir(&staging).expect("staging");
        materialize_bridge_files(&staging).expect("bridge files");
        fs::rename(&staging, &runtime).expect("activate runtime");

        assert!(!bridge_files_match(&runtime));
        materialize_bridge_files(&runtime).expect("rerender bridge files");
        assert!(bridge_files_match(&runtime));
        let patch = fs::read_to_string(runtime.join(BRIDGE_PATCH_FILE)).expect("patch");
        let runtime_url = Url::from_file_path(runtime.join(BRIDGE_SCRIPT_FILE))
            .expect("runtime URL")
            .to_string();
        let staging_url = Url::from_file_path(staging.join(BRIDGE_SCRIPT_FILE))
            .expect("staging URL")
            .to_string();
        assert!(patch.contains(&runtime_url));
        assert!(!patch.contains(&staging_url));
    }

    #[test]
    fn rejects_runtime_downgrades() {
        let current = CurrentRelease {
            release: "1.2.0".into(),
            manifest_sha256: "a".repeat(64),
            manifest: None,
            manifest_snapshot_sha256: None,
            source: RuntimeSource::Oss,
            source_identity: None,
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
            source: RuntimeSource::Oss,
            source_identity: None,
        };
        assert!(current_snapshot_is_valid(&current, &manifest));

        let mut tampered = manifest;
        tampered.release = "../outside".into();
        assert!(!current_snapshot_is_valid(&current, &tampered));
    }

    fn test_paths(root: &Path) -> crate::paths::AppPaths {
        crate::paths::AppPaths {
            local_root: root.join("local"),
            roaming_root: root.join("roaming"),
            launcher: root.join("local/launcher"),
            runtimes: root.join("local/runtimes"),
            cache: root.join("local/cache"),
            staging: root.join("local/staging"),
            state: root.join("local/state"),
            logs: root.join("local/logs"),
            dsh_home: root.join("roaming/dsh-home"),
        }
    }

    fn test_manifest(release: &str) -> ReleaseManifest {
        ReleaseManifest {
            schema: 1,
            product: crate::manifest::PRODUCT_ID.into(),
            release: release.into(),
            platform: crate::manifest::PLATFORM.into(),
            arch: crate::manifest::ARCH.into(),
            minimum_launcher: "0.1.0".into(),
            components: vec![RuntimeComponent {
                id: "runtime".into(),
                version: release.into(),
                asset: AssetRef {
                    object_key: format!("releases/{release}/windows-x64/runtime.zip"),
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
        }
    }

    fn candidate(release: &str, minimum_launcher: &str, digest: char) -> RuntimeCandidate {
        candidate_with_source(release, minimum_launcher, digest, RuntimeSource::Oss)
    }

    fn candidate_with_source(
        release: &str,
        minimum_launcher: &str,
        digest: char,
        source: RuntimeSource,
    ) -> RuntimeCandidate {
        RuntimeCandidate {
            source,
            release: release.into(),
            minimum_launcher: minimum_launcher.into(),
            manifest: AssetRef {
                object_key: format!("releases/{release}/windows-x64/manifest.json"),
                bytes: 1,
                sha256: digest.to_string().repeat(64),
            },
            manifest_value: None,
            source_identity: digest.to_string().repeat(64),
            acquisition: CandidateAcquisition::Oss {
                origin: Url::parse("https://example.invalid/").expect("origin"),
            },
        }
    }

    #[test]
    fn candidate_selection_chooses_highest_compatible_release() {
        let selected = choose_candidate(
            vec![
                candidate("1.1.0", "0.1.1", 'a'),
                candidate("1.3.0", "0.1.2", 'b'),
                candidate("1.2.0", "0.1.1", 'c'),
            ],
            None,
            &RuntimeUpdateSettings::default(),
        )
        .expect("compatible candidate");
        assert_eq!(selected.release, "1.2.0");
    }

    #[test]
    fn candidate_selection_does_not_downgrade_or_repeat_same_digest() {
        let current = CurrentRelease {
            release: "1.2.0".into(),
            manifest_sha256: "c".repeat(64),
            manifest: None,
            manifest_snapshot_sha256: None,
            source: RuntimeSource::Oss,
            source_identity: None,
        };
        assert!(
            choose_candidate(
                vec![candidate("1.1.0", "0.1.1", 'a')],
                Some(&current),
                &RuntimeUpdateSettings::default()
            )
            .is_none()
        );
        assert!(
            choose_candidate(
                vec![candidate("1.2.0", "0.1.1", 'c')],
                Some(&current),
                &RuntimeUpdateSettings::default()
            )
            .is_none()
        );
    }

    #[test]
    fn default_policy_selects_highest_compatible_candidate_across_sources() {
        let selected = choose_candidate(
            vec![
                candidate_with_source("1.2.0", "0.1.0", 'a', RuntimeSource::Oss),
                candidate_with_source("1.4.0", "0.1.0", 'b', RuntimeSource::Npm),
                candidate_with_source("1.3.0", "0.1.0", 'c', RuntimeSource::Local),
            ],
            None,
            &RuntimeUpdateSettings::default(),
        )
        .expect("multi-source candidate");
        assert_eq!(selected.release, "1.4.0");
        assert_eq!(selected.source, RuntimeSource::Npm);
    }

    #[test]
    fn available_versions_are_sorted_deduplicated_and_filtered() {
        let current = CurrentRelease {
            release: "1.2.0".into(),
            manifest_sha256: "a".repeat(64),
            manifest: None,
            manifest_snapshot_sha256: None,
            source: RuntimeSource::Oss,
            source_identity: None,
        };
        let versions = list_compatible_versions(
            vec![
                candidate_with_source("1.4.0", "0.1.0", 'a', RuntimeSource::Oss),
                candidate_with_source("1.4.0", "0.1.0", 'b', RuntimeSource::Oss),
                candidate_with_source("1.3.0", "0.1.0", 'c', RuntimeSource::Npm),
                candidate_with_source("1.1.0", "0.1.0", 'd', RuntimeSource::Local),
                candidate_with_source("1.5.0", "9.0.0", 'e', RuntimeSource::Local),
            ],
            Some(&current),
            &RuntimeUpdateSettings::default(),
        );
        assert_eq!(
            versions,
            vec![
                AvailableUpdate {
                    release: "1.4.0".into(),
                    source: RuntimeSource::Oss,
                },
                AvailableUpdate {
                    release: "1.3.0".into(),
                    source: RuntimeSource::Npm,
                },
            ]
        );
    }

    #[test]
    fn npm_version_list_keeps_versions_without_runtime_descriptor() {
        let mut versions = BTreeMap::new();
        versions.insert(
            "0.1.1-rc.2".into(),
            NpmPackageVersion {
                version: "0.1.1-rc.2".into(),
                dist: NpmDistribution {
                    tarball: Url::parse("https://example.invalid/dsh.tgz").expect("tarball"),
                    integrity: None,
                },
                runtime: None,
            },
        );
        assert_eq!(
            npm_version_names(NpmPackument { versions }),
            vec!["0.1.1-rc.2"]
        );
    }

    #[test]
    fn fixed_source_and_version_restrict_candidate_selection() {
        let settings = RuntimeUpdateSettings {
            source: UpdateSourcePolicy::Oss,
            version: Some("1.2.0".into()),
            ..RuntimeUpdateSettings::default()
        };
        let selected = choose_candidate(
            vec![
                candidate_with_source("1.4.0", "0.1.0", 'a', RuntimeSource::Oss),
                candidate_with_source("1.2.0", "0.1.0", 'b', RuntimeSource::Oss),
                candidate_with_source("1.2.0", "0.1.0", 'c', RuntimeSource::Npm),
            ],
            None,
            &settings,
        )
        .expect("fixed candidate");
        assert_eq!(selected.release, "1.2.0");
        assert_eq!(selected.source, RuntimeSource::Oss);
    }

    #[test]
    fn requested_version_is_not_silently_replaced_or_reported_as_current() {
        let settings = RuntimeUpdateSettings {
            source: UpdateSourcePolicy::Npm,
            version: Some("1.4.0".into()),
            ..RuntimeUpdateSettings::default()
        };
        let error = validate_requested_candidate(
            &[candidate_with_source(
                "1.4.0",
                "0.1.0",
                'a',
                RuntimeSource::Oss,
            )],
            None,
            &settings,
        )
        .expect_err("wrong source must be reported");
        assert!(format!("{error:#}").contains("selected runtime 1.4.0 is unavailable"));

        let settings = RuntimeUpdateSettings {
            source: UpdateSourcePolicy::Auto,
            version: Some("1.4.0".into()),
            ..RuntimeUpdateSettings::default()
        };
        let error = validate_requested_candidate(&[], None, &settings)
            .expect_err("missing closure must be reported");
        assert!(format!("{error:#}").contains("complete runtime closure"));
    }

    #[test]
    fn update_settings_default_to_auto_and_round_trip() {
        let settings = RuntimeUpdateSettings::default();
        let encoded = serde_json::to_vec(&settings).expect("settings JSON");
        let decoded: RuntimeUpdateSettings = serde_json::from_slice(&encoded).expect("settings");
        assert_eq!(decoded, settings);
        assert_eq!(decoded.source, UpdateSourcePolicy::Auto);
        assert_eq!(decoded.npm.package, DEFAULT_NPM_PACKAGE);
    }

    #[test]
    fn staged_activation_switches_current_and_removes_pending_state() {
        let root = tempdir().expect("temp");
        let paths = test_paths(root.path());
        let manager = RuntimeManager::new(paths.clone()).expect("manager");
        paths.create().expect("paths");
        let manifest = test_manifest("1.3.0");
        let runtime_root = paths.runtimes.join("1.3.0");
        fs::create_dir_all(&runtime_root).expect("runtime");
        materialize_bridge_files(&runtime_root).expect("bridge");
        let staged = StagedRelease {
            release: manifest.release.clone(),
            manifest_sha256: "b".repeat(64),
            manifest_snapshot_sha256: manifest_snapshot_sha256(&manifest).expect("snapshot"),
            manifest,
            source: RuntimeSource::Oss,
            source_identity: None,
        };
        manager.write_staged(&staged).expect("staged");
        manager
            .activate_staged_with(|_, _| Ok(()))
            .expect("activate");
        assert_eq!(
            manager
                .read_current()
                .expect("current")
                .expect("pointer")
                .release,
            "1.3.0"
        );
        assert!(manager.read_staged().expect("staged").is_none());
    }

    #[test]
    fn staged_activation_failure_keeps_existing_current() {
        let root = tempdir().expect("temp");
        let paths = test_paths(root.path());
        let manager = RuntimeManager::new(paths.clone()).expect("manager");
        paths.create().expect("paths");
        let current_manifest = test_manifest("1.2.0");
        manager
            .write_current(&CurrentRelease {
                release: current_manifest.release.clone(),
                manifest_sha256: "c".repeat(64),
                manifest: Some(current_manifest),
                manifest_snapshot_sha256: None,
                source: RuntimeSource::Oss,
                source_identity: None,
            })
            .expect("current");
        let manifest = test_manifest("1.3.0");
        let runtime_root = paths.runtimes.join("1.3.0");
        fs::create_dir_all(&runtime_root).expect("runtime");
        materialize_bridge_files(&runtime_root).expect("bridge");
        manager
            .write_staged(&StagedRelease {
                release: manifest.release.clone(),
                manifest_sha256: "d".repeat(64),
                manifest_snapshot_sha256: manifest_snapshot_sha256(&manifest).expect("snapshot"),
                manifest,
                source: RuntimeSource::Oss,
                source_identity: None,
            })
            .expect("staged");
        assert!(
            manager
                .activate_staged_with(|_, _| bail!("doctor failed"))
                .is_err()
        );
        assert_eq!(
            manager
                .read_current()
                .expect("current")
                .expect("pointer")
                .release,
            "1.2.0"
        );
        assert!(manager.read_staged().expect("staged").is_some());
    }

    #[test]
    fn forward_repair_record_is_atomic_and_clearable() {
        let root = tempdir().expect("temp");
        let paths = test_paths(root.path());
        let manager = RuntimeManager::new(paths.clone()).expect("manager");
        paths.create().expect("paths");
        manager
            .record_forward_repair("1.3.0", "start", "doctor failed")
            .expect("repair");
        let record = manager.read_repair().expect("read").expect("record");
        assert_eq!(record.release, "1.3.0");
        assert_eq!(record.phase, "start");
        manager.clear_repair().expect("clear");
        assert!(manager.read_repair().expect("read").is_none());
    }
}
