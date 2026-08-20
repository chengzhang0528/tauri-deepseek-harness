use std::collections::VecDeque;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use crate::job::ProcessJob;

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LOG_LINE: usize = 16 * 1024;
const MAX_BRIDGE_LINE: usize = 64 * 1024;
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(2);
const APP_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
const SENTINEL: &str = "@@DSH_DESKTOP@@";
const PROTOCOL_VERSION: u32 = 1;
const BRIDGE_PATCH_FILE: &str = "desktop-bridge.patch.yml";

type BridgeResponses = Arc<(Mutex<VecDeque<BridgeResponse>>, Condvar)>;

#[derive(Debug, Clone, Copy)]
pub struct BridgeStatus {
    pub accepting_new_work: bool,
    pub active_work: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeResponse {
    protocol_version: u32,
    request_id: String,
    ok: bool,
    accepting_new_work: Option<bool>,
    active_work: Option<u64>,
    error: Option<String>,
}

#[derive(Debug)]
pub struct HarnessProcess {
    child: Child,
    stdin: Arc<Mutex<std::process::ChildStdin>>,
    responses: BridgeResponses,
    _job: ProcessJob,
    pub url: Url,
}

impl HarnessProcess {
    pub fn start(root: &Path) -> Result<Self> {
        let node = find_executable(root, "node.exe")?;
        let cli = find_dsh_cli(root)?;
        let bridge_patch = find_bridge_patch(root)?;
        let mut command = Command::new(node);
        command
            .arg(cli)
            .arg("web")
            .arg("--patch")
            .arg(bridge_patch)
            .arg("--port")
            .arg("0")
            .arg("--no-open")
            .current_dir(root)
            .env("DSH_HOME", default_dsh_home())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console(&mut command);

        let mut child = command.spawn().context("cannot start dsh web")?;
        let stdin = Arc::new(Mutex::new(
            child.stdin.take().context("dsh stdin unavailable")?,
        ));
        let job = match ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                return Err(error).context("cannot attach dsh process tree to Job Object");
            }
        };
        let stdout = child.stdout.take().context("dsh stdout unavailable")?;
        let stderr = child.stderr.take().context("dsh stderr unavailable")?;
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let responses = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let stdout_lines = Arc::clone(&lines);
        let stdout_responses = Arc::clone(&responses);
        std::thread::spawn(move || read_stdout(stdout, stdout_lines, stdout_responses));
        let stderr_lines = Arc::clone(&lines);
        std::thread::spawn(move || read_log_lines(stderr, stderr_lines));
        let url = wait_for_ready(&lines)?;
        let process = Self {
            child,
            stdin,
            responses,
            _job: job,
            url,
        };
        process.status()?;
        Ok(process)
    }

    pub fn status(&self) -> Result<BridgeStatus> {
        let response = self.request("status", BRIDGE_TIMEOUT)?;
        bridge_status(&response)
    }

    pub fn begin_drain(&self) -> Result<BridgeStatus> {
        let response = self.request("beginDrain", BRIDGE_TIMEOUT)?;
        bridge_status(&response)
    }

    pub fn app_exit(&self) -> Result<()> {
        self.request("appExit", APP_EXIT_TIMEOUT).map(|_| ())
    }

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child.try_wait().context("cannot inspect dsh process")
    }

    pub fn kill(&mut self) -> Result<()> {
        self.child.kill().context("cannot terminate dsh process")
    }

    fn request(&self, operation: &str, timeout: Duration) -> Result<BridgeResponse> {
        let request_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "requestId": request_id.to_string(),
            "operation": operation,
        });
        {
            let mut stdin = self
                .stdin
                .lock()
                .map_err(|_| anyhow::anyhow!("dsh stdin lock poisoned"))?;
            writeln!(stdin, "{SENTINEL}{payload}")
                .context("cannot write desktop bridge request")?;
            stdin
                .flush()
                .context("cannot flush desktop bridge request")?;
        }

        let deadline = Instant::now() + timeout;
        let (queue, wake) = &*self.responses;
        let mut responses = queue
            .lock()
            .map_err(|_| anyhow::anyhow!("dsh bridge response lock poisoned"))?;
        loop {
            if let Some(index) = responses
                .iter()
                .position(|response| response.request_id == request_id.to_string())
            {
                let response = responses
                    .remove(index)
                    .context("bridge response queue changed unexpectedly")?;
                ensure!(
                    response.protocol_version == PROTOCOL_VERSION,
                    "unsupported desktop bridge protocol {}",
                    response.protocol_version
                );
                if !response.ok {
                    bail!(
                        "desktop bridge operation {operation} failed: {}",
                        response.error.as_deref().unwrap_or("unknown-error")
                    );
                }
                return Ok(response);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("desktop bridge operation {operation} timed out");
            }
            let (next, result) = wake
                .wait_timeout(responses, remaining)
                .map_err(|_| anyhow::anyhow!("dsh bridge response lock poisoned"))?;
            responses = next;
            if result.timed_out() {
                bail!("desktop bridge operation {operation} timed out");
            }
        }
    }
}

fn bridge_status(response: &BridgeResponse) -> Result<BridgeStatus> {
    Ok(BridgeStatus {
        accepting_new_work: response
            .accepting_new_work
            .context("desktop bridge response omitted acceptingNewWork")?,
        active_work: response
            .active_work
            .context("desktop bridge response omitted activeWork")?,
    })
}

fn wait_for_ready(lines: &Arc<Mutex<Vec<String>>>) -> Result<Url> {
    let deadline = Instant::now() + READY_TIMEOUT;
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("cannot create readiness client")?;
    let mut inspected = 0usize;
    while Instant::now() < deadline {
        let snapshot = lines
            .lock()
            .map_err(|_| anyhow::anyhow!("dsh log lock poisoned"))?
            .clone();
        for line in snapshot.iter().skip(inspected) {
            if let Some(port) = extract_port(line) {
                let url = Url::parse(&format!("http://127.0.0.1:{port}/"))?;
                if harness_page_ready(&client, &url)? {
                    return Ok(url);
                }
            }
        }
        inspected = snapshot.len();
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("dsh web did not expose a verified ready page before timeout")
}

fn harness_page_ready(client: &Client, url: &Url) -> Result<bool> {
    let Ok(response) = client.get(url.clone()).send() else {
        return Ok(false);
    };
    if !response.status().is_success() {
        return Ok(false);
    }
    let body = response.text().context("cannot read dsh readiness page")?;
    Ok(body.contains("window.__DSH_BOOT__"))
}

fn extract_port(line: &str) -> Option<u16> {
    for marker in [
        "http://127.0.0.1:",
        "http://localhost:",
        "127.0.0.1:",
        "localhost:",
    ] {
        if let Some(start) = line.find(marker) {
            let digits = line[start + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if let Ok(port) = digits.parse::<u16>()
                && port != 0
            {
                return Some(port);
            }
        }
    }
    None
}

#[allow(clippy::needless_pass_by_value)]
fn read_stdout<R: std::io::Read>(
    reader: R,
    lines: Arc<Mutex<Vec<String>>>,
    responses: BridgeResponses,
) {
    let mut reader = BufReader::new(reader);
    while let Ok(Some(line)) = read_bounded_line(&mut reader, MAX_BRIDGE_LINE) {
        let line = String::from_utf8_lossy(&line);
        let limit = if line.starts_with(SENTINEL) {
            MAX_BRIDGE_LINE
        } else {
            MAX_LOG_LINE
        };
        let bounded = line.chars().take(limit).collect::<String>();
        if let Some(response) = bounded
            .strip_prefix(SENTINEL)
            .and_then(|json| serde_json::from_str::<BridgeResponse>(json).ok())
        {
            let (queue, wake) = &*responses;
            if let Ok(mut pending) = queue.lock() {
                if pending.len() >= 64 {
                    pending.pop_front();
                }
                pending.push_back(response);
                wake.notify_all();
            }
        } else {
            append_log_line(&lines, bounded);
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn read_log_lines<R: std::io::Read>(reader: R, lines: Arc<Mutex<Vec<String>>>) {
    let mut reader = BufReader::new(reader);
    while let Ok(Some(line)) = read_bounded_line(&mut reader, MAX_BRIDGE_LINE) {
        let bounded = String::from_utf8_lossy(&line)
            .chars()
            .take(MAX_LOG_LINE)
            .collect::<String>();
        append_log_line(&lines, bounded);
    }
}

fn read_bounded_line<R: std::io::BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut bounded = Vec::with_capacity(max_bytes.min(4096));
    let mut saw_input = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(saw_input.then_some(bounded));
        }
        saw_input = true;
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(buffer.len());
        let remaining = max_bytes.saturating_sub(bounded.len());
        bounded.extend_from_slice(&buffer[..content_len.min(remaining)]);
        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(bounded));
        }
    }
}

fn append_log_line(lines: &Arc<Mutex<Vec<String>>>, bounded: String) {
    if let Ok(mut output) = lines.lock() {
        if output.len() >= 256 {
            output.remove(0);
        }
        output.push(bounded);
    }
}

fn find_executable(root: &Path, name: &str) -> Result<PathBuf> {
    for relative in [name, "node/node.exe", "runtime/node.exe", "bin/node.exe"] {
        let candidate = root.join(relative);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("required executable {name} is missing from runtime")
}

fn find_dsh_cli(root: &Path) -> Result<PathBuf> {
    for relative in [
        "node_modules/@deepseek-ai/dsh/dist/cli.js",
        "node_modules/@deepseek-ai/deepseek-harness/dist/cli.js",
        "dsh/cli.js",
    ] {
        let candidate = root.join(relative);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("dsh CLI entry is missing from runtime")
}

fn find_bridge_patch(root: &Path) -> Result<PathBuf> {
    let candidate = root.join(BRIDGE_PATCH_FILE);
    ensure!(
        candidate.is_file(),
        "desktop bridge patch is missing from runtime"
    );
    Ok(candidate)
}

fn default_dsh_home() -> PathBuf {
    std::env::var_os("APPDATA").map_or_else(
        || PathBuf::from("dsh-home"),
        |value| PathBuf::from(value).join("DSH Desktop").join("dsh-home"),
    )
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use super::read_bounded_line;
    use super::{BridgeResponse, PROTOCOL_VERSION, bridge_status, extract_port};

    #[test]
    fn extracts_only_nonzero_local_ports() {
        assert_eq!(
            extract_port("Listening on http://127.0.0.1:43123/"),
            Some(43123)
        );
        assert_eq!(extract_port("http://localhost:0/"), None);
        assert_eq!(extract_port("remote http://10.0.0.2:43123/"), None);
    }

    #[test]
    fn parses_bridge_status_fields() {
        let status = bridge_status(&BridgeResponse {
            protocol_version: PROTOCOL_VERSION,
            request_id: "request".into(),
            ok: true,
            accepting_new_work: Some(false),
            active_work: Some(2),
            error: None,
        })
        .expect("status");
        assert!(!status.accepting_new_work);
        assert_eq!(status.active_work, 2);
    }

    #[test]
    fn bounded_reader_consumes_truncated_lines_before_returning_next_line() {
        let mut reader = BufReader::new(&b"123456789\nnext\n"[..]);
        assert_eq!(
            read_bounded_line(&mut reader, 4).expect("line"),
            Some(b"1234".to_vec())
        );
        assert_eq!(
            read_bounded_line(&mut reader, 4).expect("line"),
            Some(b"next".to_vec())
        );
        assert_eq!(read_bounded_line(&mut reader, 4).expect("eof"), None);
    }
}
