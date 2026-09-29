//! Codex usage through `codex app-server` (JSON-RPC over stdio), mirroring the
//! macOS provider: initialize → initialized → account/rateLimits/read.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use super::parser::{parse_codex_response, UsageSnapshot};
use crate::paths;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_LINE: usize = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("codex.notFound")]
    NotFound,
    #[error("codex.launch")]
    Launch,
    #[error("codex.timeout")]
    Timeout,
    #[error("codex.closed")]
    Closed,
    #[error("codex.rejected")]
    Rejected,
    /// Server-provided message, shown verbatim.
    #[error("{0}")]
    Server(String),
    #[error("codex.noLimits")]
    NoLimits,
}

pub fn find_executable() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["codex.exe", "codex.cmd"]
    } else {
        &["codex"]
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    } else {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        dirs.push(paths::home().join(".local/bin"));
    }
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(executable: &Path) -> std::io::Result<Child> {
    let mut command = Command::new(executable);
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn()
}

pub fn fetch() -> Result<UsageSnapshot, CodexError> {
    let executable = find_executable().ok_or(CodexError::NotFound)?;
    let mut child = KillOnDrop(spawn(&executable).map_err(|_| CodexError::Launch)?);
    let mut stdin = child.0.stdin.take().ok_or(CodexError::Launch)?;
    let stdout = child.0.stdout.take().ok_or(CodexError::Launch)?;

    let (tx, rx) = mpsc::channel::<Option<Value>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(n) if n > MAX_LINE => break,
                Ok(_) => {
                    if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                        if tx.send(Some(value)).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        let _ = tx.send(None);
    });

    let send = |stdin: &mut std::process::ChildStdin, value: Value| -> Result<(), CodexError> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| CodexError::Launch)?;
        bytes.push(b'\n');
        stdin
            .write_all(&bytes)
            .and_then(|_| stdin.flush())
            .map_err(|_| CodexError::Closed)
    };
    send(
        &mut stdin,
        json!({"method": "initialize", "id": 1, "params": {"clientInfo": {
            "name": "agent_notch_windows", "title": "AgentNotch for Windows", "version": env!("CARGO_PKG_VERSION")
        }}}),
    )?;

    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let message = match rx.recv_timeout(remaining) {
            Ok(Some(message)) => message,
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => return Err(CodexError::Closed),
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(CodexError::Timeout),
        };
        match message.get("id").and_then(Value::as_i64) {
            Some(1) => {
                if message.get("error").is_some() {
                    return Err(CodexError::Rejected);
                }
                send(&mut stdin, json!({"method": "initialized", "params": {}}))?;
                send(
                    &mut stdin,
                    json!({"method": "account/rateLimits/read", "id": 2, "params": {"excludeResetCreditDetails": true}}),
                )?;
            }
            Some(2) => {
                if let Some(error) = message.get("error") {
                    let text = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex error");
                    return Err(CodexError::Server(text.to_owned()));
                }
                return parse_codex_response(&message, paths::now())
                    .map_err(|_| CodexError::NoLimits);
            }
            _ => {}
        }
    }
}
