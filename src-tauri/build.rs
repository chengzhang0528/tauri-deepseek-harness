fn main() {
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../version.json"))
        .expect("version.json must be valid JSON");
    let version = metadata
        .get("launcherVersion")
        .and_then(serde_json::Value::as_str)
        .expect("version.json must define launcherVersion");
    println!("cargo:rustc-env=DSH_LAUNCHER_VERSION={version}");
    println!("cargo:rerun-if-changed=../version.json");
    tauri_build::build();
}
