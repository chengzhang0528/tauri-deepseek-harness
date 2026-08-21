use std::fs;

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use url::Url;

use crate::manifest::Bootstrap;

pub const DEFAULT_ORIGIN: &str =
    "https://shared-public-assets.oss-cn-beijing.aliyuncs.com/atlas-dsh-desktop/";
pub const REMOTE_BOOTSTRAP_KEY: &str = "bootstrap/windows-x64.json";
const MAX_BOOTSTRAP_BYTES: u64 = 1024 * 1024;

pub fn embedded_seed() -> Result<Bootstrap> {
    let bootstrap: Bootstrap =
        serde_json::from_str(include_str!("../resources/bootstrap.windows-x64.seed.json"))
            .context("embedded bootstrap is invalid JSON")?;
    if !bootstrap.is_placeholder() {
        bootstrap
            .validate()
            .context("embedded bootstrap is invalid")?;
    }
    Ok(bootstrap)
}

pub fn release_origin() -> Result<Url> {
    Url::parse(DEFAULT_ORIGIN).context("release origin is not a URL")
}

pub fn fetch_bootstrap(client: &Client, origin: &Url) -> Result<Bootstrap> {
    let url = origin
        .join(REMOTE_BOOTSTRAP_KEY)
        .context("cannot resolve bootstrap URL")?;
    let response = client
        .get(url)
        .send()
        .context("cannot download bootstrap")?
        .error_for_status()
        .context("bootstrap request failed")?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BOOTSTRAP_BYTES)
    {
        bail!("bootstrap exceeds the client maximum size");
    }
    let body = response.bytes().context("cannot read bootstrap")?;
    ensure!(
        u64::try_from(body.len()).unwrap_or(u64::MAX) <= MAX_BOOTSTRAP_BYTES,
        "bootstrap exceeds the client maximum size"
    );
    let bootstrap: Bootstrap =
        serde_json::from_slice(&body).context("bootstrap JSON is invalid")?;
    bootstrap.validate().context("bootstrap is invalid")?;
    Ok(bootstrap)
}

pub fn load_seed_or_remote(client: &Client, origin: &Url) -> Result<Bootstrap> {
    match fetch_bootstrap(client, origin) {
        Ok(bootstrap) => Ok(bootstrap),
        Err(remote_error) => {
            let seed = embedded_seed()?;
            if seed.is_placeholder() {
                bail!("runtime bootstrap is unavailable: {remote_error}");
            }
            seed.validate().context("embedded seed is invalid")?;
            Ok(seed)
        }
    }
}

pub fn runtime_preparation_message(error: &anyhow::Error) -> String {
    let detail = format!("{error:#}");
    if detail.contains("runtime bootstrap is unavailable") {
        return format!(
            "没有已发布的受管 runtime。薄安装器不会使用系统 Node、npm 或其他下载源。\n\n请先将已验证的 runtime ZIP、manifest 和 bootstrap 发布到批准的 atlas-dsh-desktop/ OSS 前缀，然后重试启动。\n\n诊断：{detail}"
        );
    }
    format!("运行时准备失败：{detail}")
}

pub fn write_seed_copy(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let source = include_bytes!("../resources/bootstrap.windows-x64.seed.json");
    fs::write(path, source).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_explicitly_unusable_until_a_release_is_published() {
        assert!(embedded_seed().expect("seed JSON").is_placeholder());
    }

    #[test]
    fn origin_keeps_the_single_project_prefix() {
        let origin = Url::parse(DEFAULT_ORIGIN).expect("origin");
        assert_eq!(
            origin.join(REMOTE_BOOTSTRAP_KEY).expect("URL").path(),
            "/atlas-dsh-desktop/bootstrap/windows-x64.json"
        );
    }

    #[test]
    fn unavailable_bootstrap_explains_the_thin_installer_boundary() {
        let message = runtime_preparation_message(&anyhow::anyhow!(
            "runtime bootstrap is unavailable: bootstrap request failed"
        ));
        assert!(message.contains("没有已发布的受管 runtime"));
        assert!(message.contains("不会使用系统 Node"));
    }
}
