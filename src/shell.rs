//! Shell: run a command on one of your devices.

use std::time::Duration;

use anyhow::bail;
use serde_json::{Value, json};

use crate::apps::{AppEntry, Me};
use crate::entry::Content;
use crate::native::Reply;

pub fn describe() -> Value {
    json!({
        "name": "shell", "title": "Shell", "icon": "terminal", "runs": "here",
        "instructions": "Run a shell command on one of your own devices with shell_run; give device to choose which. Every command is logged in the space.",
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

pub fn call(op: String, input: Value, caller: Me, _entries: Vec<AppEntry>) -> Reply {
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
        let child = tokio::process::Command::new("sh").arg("-c").arg(&command).current_dir(&dir).kill_on_drop(true).output();
        let (exit, stdout, stderr) = match tokio::time::timeout(Duration::from_secs(timeout), child).await {
            Ok(Ok(out)) => (out.status.code().unwrap_or(-1), clip(&out.stdout), clip(&out.stderr)),
            Ok(Err(err)) => bail!("couldn't start sh: {err}"),
            Err(_) => (-1, String::new(), format!("timed out after {timeout}s")),
        };
        let log = Content { app: "shell".into(), kind: "ran".into(), data: json!({ "command": command, "dir": dir, "exit": exit, "by": caller.name }) };
        Ok((json!({ "exit": exit, "stdout": stdout, "stderr": stderr }), vec![log]))
    })
}

/// Keeps the end of long output, where errors usually are.
fn clip(bytes: &[u8]) -> String {
    const MAX: usize = 64 * 1024;
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= MAX {
        return text.into_owned();
    }
    let cut = text.len() - MAX;
    let start = (cut..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
    format!("…{}", &text[start..])
}
