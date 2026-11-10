//! Stay Awake's service: while it runs, `caffeinate -s` keeps its Mac from sleeping, lid shut or not. macOS honours that
//! only on power, so on battery the Mac sleeps as usual.

use std::process::{Child, Command};
use std::sync::Mutex;

use anyhow::{Result, bail};
use one_service::{Answer, Call, Host, Service};
use serde_json::json;

static HOLD: Mutex<Option<Child>> = Mutex::new(None);

fn main() -> Result<()> {
    one_service::serve(Service { name: "awake", prefer_here: &["status"], call, start: Some(start) })
}

/// `-w` lets go when this program ends, however it ends.
fn start(_: &Host) {
    match Command::new("/usr/bin/caffeinate").args(["-s", "-w", &std::process::id().to_string()]).spawn() {
        Ok(child) => *HOLD.lock().unwrap() = Some(child),
        Err(err) => eprintln!("couldn't start caffeinate: {err}"),
    }
}

fn call(_: &Host, call: &Call) -> Result<Answer> {
    if call.function != "status" {
        bail!("awake has no function {}", call.function);
    }
    let holding = HOLD.lock().unwrap().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
    Ok(Answer::new(json!({ "holding": holding, "plugged_in": plugged_in() })))
}

/// pmset's first line names the power source: "Now drawing from 'AC Power'".
fn plugged_in() -> bool {
    Command::new("/usr/bin/pmset").args(["-g", "ps"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().next().is_some_and(|l| l.contains("AC Power")))
}
