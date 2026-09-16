use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::Deserialize;
use url::Url;
use zip::ZipArchive;

use crate::job::ProcessJob;

pub const PACKAGE: &str = "@deepseek-ai/dsh";
pub const REGISTRY: &str = "https://registry.npmjs.org/";
const NODE_VERSION: &str = "24.19.0";
const NODE_URL: &str = "https://nodejs.org/dist/v24.19.0/node-v24.19.0-win-x64.zip";
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_EXTRACTED_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Deserialize)]
struct Packument {
    versions: BTreeMap<String, serde_json::Value>,
}

pub fn versions(client: &Client, registry: &str) -> Result<Vec<String>> {
    let url = Url::parse(&format!(
        "{}/@deepseek-ai%2fdsh",
        registry.trim_end_matches('/')
    ))
    .context("官方版本来源地址无效")?;
    let response = client
        .get(url)
        .send()
        .map_err(|_| anyhow::anyhow!("无法连接官方版本来源"))?;
    ensure!(
        response.status().is_success(),
        "官方版本来源返回 HTTP {}",
        response.status()
    );
    let mut bytes = Vec::new();
    response
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("无法读取官方版本列表")?;
    ensure!(
        u64::try_from(bytes.len())? <= MAX_METADATA_BYTES,
        "版本列表过大"
    );
    let package: Packument = serde_json::from_slice(&bytes).context("官方版本列表格式无效")?;
    let mut versions = package
        .versions
        .into_keys()
        .filter(|value| semver::Version::parse(value).is_ok())
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| compare_versions(right, left));
    Ok(versions)
}

pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    // All callers validate version syntax before storing or comparing it.
    semver::Version::parse(left)
        .ok()
        .cmp(&semver::Version::parse(right).ok())
}

pub fn prepare_node(client: &Client, target: &Path, reusable: Option<&Path>) -> Result<()> {
    let node = target.join("node");
    if let Some(source) = reusable.filter(|root| {
        root.join("node.exe").is_file() && root.join("node_modules/npm/bin/npm-cli.js").is_file()
    }) {
        copy_tree(source, &node)?;
        return Ok(());
    }
    let response = client
        .get(NODE_URL)
        .send()
        .map_err(|_| anyhow::anyhow!("无法下载官方 Node.js"))?;
    ensure!(
        response.status().is_success(),
        "Node.js 下载返回 HTTP {}",
        response.status()
    );
    let mut bytes = Vec::new();
    response
        .take(MAX_DOWNLOAD_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Node.js 下载未完成")?;
    ensure!(
        u64::try_from(bytes.len())? <= MAX_DOWNLOAD_BYTES,
        "Node.js 下载超过本机准备上限"
    );
    let unpack = target.join("node-unpack");
    fs::create_dir_all(&unpack)?;
    extract_zip_safely(&bytes, &unpack)?;
    fs::rename(unpack.join(format!("node-v{NODE_VERSION}-win-x64")), &node)
        .context("无法准备私有 Node.js 目录")?;
    fs::remove_dir(&unpack)?;
    Ok(())
}

pub fn install(root: &Path, version: &str, registry: &str, cache: &Path) -> Result<()> {
    semver::Version::parse(version).context("指定的 Harness 版本无效")?;
    let node_dir = root.join("node");
    let mut command = Command::new(node_dir.join("node.exe"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![node_dir.clone()];
    paths.extend(std::env::split_paths(&path));
    command
        .arg(node_dir.join("node_modules/npm/bin/npm-cli.js"))
        .args([
            "install",
            "--save-exact",
            "--no-audit",
            "--no-fund",
            "--loglevel=error",
        ])
        .arg("--prefix")
        .arg(root)
        .arg("--cache")
        .arg(cache)
        .arg("--registry")
        .arg(registry)
        .arg(format!("{PACKAGE}@{version}"))
        .current_dir(root)
        .env("PATH", std::env::join_paths(paths)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let (mut child, job) = ProcessJob::spawn(&mut command).context("无法启动私有 npm 安装进程")?;
    let deadline = Instant::now() + Duration::from_mins(15);
    loop {
        if let Some(status) = child.try_wait().context("无法读取 npm 安装结果")? {
            ensure!(
                status.success(),
                "npm 安装失败（{status}）；可重试，详细信息在本机 npm 缓存的 _logs 目录"
            );
            let cleanup_deadline = Instant::now() + Duration::from_secs(5);
            while !job.is_empty()? && Instant::now() < cleanup_deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            ensure!(job.is_empty()?, "npm 已返回，但安装子进程尚未结束");
            return Ok(());
        }
        if Instant::now() >= deadline {
            job.terminate().context("无法终止超时安装进程树")?;
            let _ = child.wait();
            bail!("npm 安装超时；当前运行版本与用户数据未改变");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        !fs::symlink_metadata(source)?.file_type().is_symlink(),
        "私有 Node.js 目录不能是链接"
    );
    fs::create_dir(destination)?;
    for item in fs::read_dir(source)? {
        let item = item?;
        let kind = item.file_type()?;
        ensure!(!kind.is_symlink(), "私有 Node.js 不复制链接");
        if kind.is_dir() {
            copy_tree(&item.path(), &destination.join(item.file_name()))?;
        } else {
            fs::copy(item.path(), destination.join(item.file_name()))?;
        }
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
