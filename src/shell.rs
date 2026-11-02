//! Shell: run a command on one of your servers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;

use anyhow::bail;
use serde_json::{Value, json};

use crate::apps::Me;
use crate::sidecars::{Answer, Record, Reply};

pub fn describe() -> Value {
    json!({
        "name": "shell", "title": "Shell", "icon": "terminal-window", "color": "#3a3a3c", "runs": "here", "prefer": "here", "description": "Runs commands on your own devices",
        "instructions": "Run a shell command on one of your own devices with shell_run; give server (a device's name) to choose which. Every command is logged where only you can read it.",
        "ops": [{
            "name": "run",
            "description": "Run a shell command on this device (or the device given) and return its exit code and output. Every command is logged.",
            "input": { "type": "object", "required": ["command"], "properties": {
                "command": { "type": "string", "description": "run by sh -c" },
                "dir": { "type": "string", "description": "working directory; default is the home directory" },
                "timeout": { "type": "integer", "description": "seconds, default 60, at most 600" },
            }},
        }],
        "widgets": [],
    })
}

pub fn call(op: String, input: Value, caller: Me, _records: Vec<Record>) -> Reply {
    Box::pin(async move {
        if op != "run" {
            bail!("shell has no operation {op}");
        }
        let command = input["command"].as_str().unwrap_or("").to_string();
        if command.trim().is_empty() {
            bail!("give a command");
        }
        let dir = input["dir"].as_str().map(str::to_string).unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()));
        let timeout = input["timeout"].as_u64().unwrap_or(60).clamp(1, 600);
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .process_group(0)
            .spawn()
            .map_err(|err| anyhow::anyhow!("couldn't start sh: {err}"))?;
        // Read as it prints, so a command stopped for taking too long still gives what it printed.
        let (out, err) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
        let readers = [read_into(child.stdout.take(), out.clone()), read_into(child.stderr.take(), err.clone())];
        let (exit, stopped) = match tokio::time::timeout(Duration::from_secs(timeout), child.wait()).await {
            Ok(status) => (status?.code().unwrap_or(-1), false),
            Err(_) => {
                // The whole group, so what the command started stops too.
                if let Some(pid) = child.id() {
                    unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                }
                let _ = child.kill().await;
                (-1, true)
            }
        };
        // A background process the command left may keep its output open: wait a moment, not for it.
        let _ = tokio::time::timeout(Duration::from_secs(2), futures_join(readers)).await;
        let stdout = clip(&out.lock().unwrap());
        let mut stderr = clip(&err.lock().unwrap());
        if stopped {
            stderr.push_str(&format!("{}(stopped after {timeout}s)", if stderr.is_empty() || stderr.ends_with('\n') { "" } else { "\n" }));
        }
        let ran = json!({ "command": command, "dir": dir, "exit": exit, "by": caller.name, "agent": caller.agent });
        Ok(Answer { records: vec![("ran".into(), ran)], ..Answer::new(json!({ "exit": exit, "stdout": stdout, "stderr": stderr })) })
    })
}

fn read_into(pipe: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>, into: Arc<Mutex<Vec<u8>>>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(mut pipe) = pipe else { return };
        let mut buf = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut buf).await {
            if n == 0 {
                break;
            }
            into.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    })
}

async fn futures_join(handles: [tokio::task::JoinHandle<()>; 2]) {
    for h in handles {
        let _ = h.await;
    }
}

/// Keeps the end of long output, where errors usually are, and says how much was cut.
fn clip(bytes: &[u8]) -> String {
    const MAX: usize = 64 * 1024;
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= MAX {
        return text.into_owned();
    }
    let cut = text.len() - MAX;
    let start = (cut..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
    format!("({start} earlier bytes cut) …{}", &text[start..])
}
