//! Shell's service: runs a command on its device, as its person, and logs each in its folder.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use one_service::{Answer, Call, Host, Service};
use serde_json::json;

fn main() -> Result<()> {
    one_service::serve(Service { name: "shell", prefer_here: &["run"], call, start: None })
}

fn call(host: &Host, call: &Call) -> Result<Answer> {
    if call.function != "run" {
        bail!("shell has no function {}", call.function);
    }
    let command = call.input["command"].as_str().unwrap_or("").to_string();
    if command.trim().is_empty() {
        bail!("give a command");
    }
    let dir = call.input["dir"].as_str().map(str::to_string).unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    let timeout = call.input["timeout"].as_u64().unwrap_or(60).clamp(1, 600);
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0)
        .spawn()
        .context("couldn't start sh")?;
    // Read as it prints, so a command stopped for taking too long still gives what it printed.
    let (out, err) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
    let readers = [read_into(child.stdout.take(), out.clone()), read_into(child.stderr.take(), err.clone())];
    let until = Instant::now() + Duration::from_secs(timeout);
    let (exit, stopped) = loop {
        if let Some(status) = child.try_wait()? {
            break (status.code().unwrap_or(-1), false);
        }
        if Instant::now() >= until {
            // SAFETY: a signal to the process group of a child of this process, which it hasn't reaped.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            let _ = child.wait();
            break (-1, true);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A background process the command left may keep its output open: wait a moment, not for it.
    let read_by = Instant::now() + Duration::from_secs(2);
    while readers.iter().any(|r| !r.is_finished()) && Instant::now() < read_by {
        std::thread::sleep(Duration::from_millis(10));
    }
    let stdout = clip(&out.lock().unwrap());
    let mut stderr = clip(&err.lock().unwrap());
    if stopped {
        stderr.push_str(&format!("{}(stopped after {timeout}s)", if stderr.is_empty() || stderr.ends_with('\n') { "" } else { "\n" }));
    }
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let ran = json!({ "time": time, "command": command, "dir": dir, "exit": exit, "by": call.caller.name, "agent": call.caller.agent });
    if let Err(err) = std::fs::OpenOptions::new().create(true).append(true).open(host.dir.join("ran.jsonl")).and_then(|mut f| writeln!(f, "{ran}")) {
        eprintln!("logging it: {err}");
    }
    Ok(Answer::new(json!({ "exit": exit, "stdout": stdout, "stderr": stderr })))
}

fn read_into(pipe: Option<impl Read + Send + 'static>, into: Arc<Mutex<Vec<u8>>>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut buf = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            into.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    })
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
