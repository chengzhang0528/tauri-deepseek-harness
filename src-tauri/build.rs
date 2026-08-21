fn main() {
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../version.json"))
        .expect("version.json must be valid JSON");
    let fallback_version = metadata
        .get("launcherVersion")
        .and_then(serde_json::Value::as_str)
        .expect("version.json must define launcherVersion");
    let version = std::env::var("DSH_RELEASE_VERSION").unwrap_or_else(|_| fallback_version.into());
    assert!(
        is_release_version(&version),
        "release version must be a semantic x.y.z version"
    );
    println!("cargo:rustc-env=DSH_LAUNCHER_VERSION={version}");
    println!("cargo:rerun-if-changed=../version.json");
    println!("cargo:rerun-if-env-changed=DSH_RELEASE_VERSION");
    tauri_build::build();
}

fn is_release_version(value: &str) -> bool {
    let (core, prerelease) = value
        .split_once('-')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let core_parts = core.split('.').collect::<Vec<_>>();
    core_parts.len() == 3
        && core_parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && prerelease.is_none_or(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        })
}
