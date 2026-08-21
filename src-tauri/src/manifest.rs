use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PRODUCT_ID: &str = "atlas-dsh-desktop";
pub const PLATFORM: &str = "windows";
pub const ARCH: &str = "x64";
pub const SHA256_HEX_LENGTH: usize = 64;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub object_key: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub schema: u32,
    pub product: String,
    pub platform: String,
    pub arch: String,
    pub release: String,
    pub minimum_launcher: String,
    pub manifest: AssetRef,
    #[serde(default)]
    pub catalog: Option<AssetRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRelease {
    pub release: String,
    pub minimum_launcher: String,
    pub manifest: AssetRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseCatalog {
    pub schema: u32,
    pub product: String,
    pub platform: String,
    pub arch: String,
    pub releases: Vec<CatalogRelease>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_doctor_timeout")]
    pub timeout_seconds: u64,
}

const fn default_doctor_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeComponent {
    pub id: String,
    pub version: String,
    pub asset: AssetRef,
    pub archive: String,
    pub install_root: String,
    pub doctor: DoctorSpec,
    #[serde(default)]
    pub licenses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub schema: u32,
    pub product: String,
    pub release: String,
    pub platform: String,
    pub arch: String,
    pub minimum_launcher: String,
    pub components: Vec<RuntimeComponent>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("bootstrap schema {0} is not supported")]
    UnsupportedBootstrapSchema(u32),
    #[error("manifest schema {0} is not supported")]
    UnsupportedManifestSchema(u32),
    #[error("{field} does not match this client")]
    WrongTarget { field: &'static str },
    #[error("release is empty")]
    EmptyRelease,
    #[error("asset {field} is invalid: {reason}")]
    InvalidAsset { field: &'static str, reason: String },
    #[error("component {0} is invalid")]
    InvalidComponent(String),
    #[error("duplicate component id {0}")]
    DuplicateComponent(String),
    #[error("manifest has no components")]
    EmptyComponents,
}

impl Bootstrap {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != 1 {
            return Err(ManifestError::UnsupportedBootstrapSchema(self.schema));
        }
        validate_target(&self.product, &self.platform, &self.arch)?;
        validate_release(&self.release)?;
        validate_asset(&self.manifest, "manifest")?;
        if let Some(catalog) = &self.catalog {
            validate_asset(catalog, "catalog")?;
        }
        Ok(())
    }

    pub fn is_placeholder(&self) -> bool {
        self.manifest.bytes == 0
            && self
                .manifest
                .sha256
                .chars()
                .all(|character| character == '0')
    }
}

impl ReleaseCatalog {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != 1 {
            return Err(ManifestError::UnsupportedManifestSchema(self.schema));
        }
        validate_target(&self.product, &self.platform, &self.arch)?;
        if self.releases.is_empty() {
            return Err(ManifestError::EmptyComponents);
        }

        let mut release_ids = std::collections::BTreeSet::new();
        for entry in &self.releases {
            validate_release(&entry.release)?;
            validate_release(&entry.minimum_launcher)?;
            validate_asset(&entry.manifest, "catalog manifest")?;
            if !release_ids.insert(&entry.release) {
                return Err(ManifestError::DuplicateComponent(entry.release.clone()));
            }
        }
        Ok(())
    }
}

impl ReleaseManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != 1 {
            return Err(ManifestError::UnsupportedManifestSchema(self.schema));
        }
        validate_target(&self.product, &self.platform, &self.arch)?;
        validate_release(&self.release)?;
        if self.components.is_empty() {
            return Err(ManifestError::EmptyComponents);
        }

        let mut component_ids = std::collections::BTreeSet::new();
        for component in &self.components {
            if component.id.trim().is_empty()
                || component.version.trim().is_empty()
                || component.archive != "zip"
                || (!component.install_root.is_empty()
                    && !is_safe_relative_path(&component.install_root))
                || !is_safe_relative_path(&component.doctor.program)
                || component.doctor.timeout_seconds == 0
                || component
                    .doctor
                    .args
                    .iter()
                    .any(|argument| argument.contains('\0'))
            {
                return Err(ManifestError::InvalidComponent(component.id.clone()));
            }
            validate_asset(&component.asset, "component asset")?;
            if !component_ids.insert(&component.id) {
                return Err(ManifestError::DuplicateComponent(component.id.clone()));
            }
        }
        Ok(())
    }
}

fn validate_target(product: &str, platform: &str, arch: &str) -> Result<(), ManifestError> {
    if product != PRODUCT_ID {
        return Err(ManifestError::WrongTarget { field: "product" });
    }
    if platform != PLATFORM {
        return Err(ManifestError::WrongTarget { field: "platform" });
    }
    if arch != ARCH {
        return Err(ManifestError::WrongTarget { field: "arch" });
    }
    Ok(())
}

fn validate_release(release: &str) -> Result<(), ManifestError> {
    if release.trim().is_empty()
        || release.starts_with('.')
        || release.starts_with('-')
        || release.ends_with('.')
        || release.ends_with('-')
        || release.contains("..")
        || !release
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(ManifestError::EmptyRelease);
    }
    if compare_versions(release, "0.0.0").is_err() {
        return Err(ManifestError::EmptyRelease);
    }
    Ok(())
}

pub fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering, String> {
    fn parse(value: &str) -> Result<(u64, u64, u64, Vec<&str>), String> {
        let (core, prerelease) = value
            .split_once('-')
            .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
        let mut numbers = core.split('.');
        let major = numbers
            .next()
            .ok_or_else(|| format!("invalid version {value}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid version {value}"))?;
        let minor = numbers
            .next()
            .ok_or_else(|| format!("invalid version {value}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid version {value}"))?;
        let patch = numbers
            .next()
            .ok_or_else(|| format!("invalid version {value}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid version {value}"))?;
        if numbers.next().is_some() || prerelease.is_some_and(str::is_empty) {
            return Err(format!("invalid version {value}"));
        }
        let suffix = prerelease.map_or_else(Vec::new, |value| value.split('.').collect::<Vec<_>>());
        if suffix.iter().any(|part| part.is_empty()) {
            return Err(format!("invalid version {value}"));
        }
        Ok((major, minor, patch, suffix))
    }

    let (left_major, left_minor, left_patch, left_suffix) = parse(left)?;
    let (right_major, right_minor, right_patch, right_suffix) = parse(right)?;
    let core = (left_major, left_minor, left_patch).cmp(&(right_major, right_minor, right_patch));
    if core != std::cmp::Ordering::Equal {
        return Ok(core);
    }
    match (left_suffix.is_empty(), right_suffix.is_empty()) {
        (true, true) => Ok(std::cmp::Ordering::Equal),
        (true, false) => Ok(std::cmp::Ordering::Greater),
        (false, true) => Ok(std::cmp::Ordering::Less),
        (false, false) => {
            for (left, right) in left_suffix.iter().zip(right_suffix.iter()) {
                let left_numeric = left.bytes().all(|byte| byte.is_ascii_digit());
                let right_numeric = right.bytes().all(|byte| byte.is_ascii_digit());
                let ordering = match (left_numeric, right_numeric) {
                    (true, true) => left
                        .parse::<u64>()
                        .map_err(|_| format!("invalid version {left}"))?
                        .cmp(
                            &right
                                .parse::<u64>()
                                .map_err(|_| format!("invalid version {right}"))?,
                        ),
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    (false, false) => left.cmp(right),
                };
                if ordering != std::cmp::Ordering::Equal {
                    return Ok(ordering);
                }
            }
            Ok(left_suffix.len().cmp(&right_suffix.len()))
        }
    }
}

fn validate_asset(asset: &AssetRef, field: &'static str) -> Result<(), ManifestError> {
    if asset.bytes == 0 {
        return Err(ManifestError::InvalidAsset {
            field,
            reason: "bytes must be positive".into(),
        });
    }
    if !is_safe_relative_path(&asset.object_key) {
        return Err(ManifestError::InvalidAsset {
            field,
            reason: "object key is not a safe relative path".into(),
        });
    }
    if asset.sha256.len() != SHA256_HEX_LENGTH
        || !asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ManifestError::InvalidAsset {
            field,
            reason: "sha256 must be 64 hexadecimal characters".into(),
        });
    }
    Ok(())
}

pub fn is_safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.contains('\0') {
        return false;
    }
    let path = Path::new(value);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> AssetRef {
        AssetRef {
            object_key: "releases/0.1.0/windows-x64/runtime.zip".into(),
            bytes: 1,
            sha256: "a".repeat(SHA256_HEX_LENGTH),
        }
    }

    #[test]
    fn rejects_unsafe_object_keys() {
        for object_key in [
            "../runtime.zip",
            "C:/runtime.zip",
            "/runtime.zip",
            "runtime/../file.zip",
        ] {
            let mut candidate = asset();
            candidate.object_key = object_key.into();
            assert!(validate_asset(&candidate, "test").is_err(), "{object_key}");
        }
    }

    #[test]
    fn validates_exact_platform_closure() {
        let manifest = ReleaseManifest {
            schema: 1,
            product: PRODUCT_ID.into(),
            release: "0.1.0".into(),
            platform: PLATFORM.into(),
            arch: ARCH.into(),
            minimum_launcher: "0.1.0".into(),
            components: vec![RuntimeComponent {
                id: "runtime".into(),
                version: "0.1.0".into(),
                asset: asset(),
                archive: "zip".into(),
                install_root: String::new(),
                doctor: DoctorSpec {
                    program: "node.exe".into(),
                    args: vec!["--version".into()],
                    timeout_seconds: 30,
                },
                licenses: vec!["THIRD_PARTY_NOTICES.md".into()],
            }],
        };
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn compares_versions_semantically() {
        assert_eq!(
            compare_versions("0.10.0", "0.9.0").expect("version"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.0.0-alpha", "1.0.0").expect("version"),
            std::cmp::Ordering::Less
        );
        assert!(compare_versions("1.0", "1.0.0").is_err());
        assert_eq!(
            compare_versions("1.0.0-alpha.10", "1.0.0-alpha.2").expect("version"),
            std::cmp::Ordering::Greater
        );
        assert!(compare_versions("1.0.0-alpha..1", "1.0.0").is_err());
    }
}
