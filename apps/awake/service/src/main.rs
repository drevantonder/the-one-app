//! Stay Awake's service: while it's on, `caffeinate -s` keeps its Mac from sleeping, lid shut or not. macOS honours that
//! only on power, so on battery the Mac sleeps as usual. It's on unless turned off (`set`), which it keeps in its
//! folder as a file named `off`.

use std::process::{Child, Command};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use one_service::{Answer, Call, Host, Service};
use serde_json::{Value, json};

static HOLD: Mutex<Option<Child>> = Mutex::new(None);

fn main() -> Result<()> {
    one_service::serve(Service { name: "awake", prefer_here: &["status", "set"], call, start: Some(start) })
}

fn start(host: &Host) {
    hold(!host.dir.join("off").exists());
}

fn call(host: &Host, call: &Call) -> Result<Answer> {
    match call.function.as_str() {
        "status" => {}
        "set" => {
            let on = call.input["on"].as_bool().context("give on: true or false")?;
            let off = host.dir.join("off");
            if on { std::fs::remove_file(&off).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })? } else { std::fs::write(&off, "")? }
            hold(on);
        }
        other => bail!("awake has no function {other}"),
    }
    Ok(Answer::new(status(host)))
}

fn status(host: &Host) -> Value {
    let holding = HOLD.lock().unwrap().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
    json!({ "on": !host.dir.join("off").exists(), "holding": holding, "plugged_in": plugged_in() })
}

/// Holds the Mac awake, or lets go. `-w` lets go when this program ends, however it ends.
fn hold(on: bool) {
    let mut hold = HOLD.lock().unwrap();
    if let Some(child) = hold.as_mut()
        && (!on || !matches!(child.try_wait(), Ok(None)))
    {
        let _ = child.kill();
        let _ = child.wait();
        *hold = None;
    }
    if on && hold.is_none() {
        match Command::new("/usr/bin/caffeinate").args(["-s", "-w", &std::process::id().to_string()]).spawn() {
            Ok(child) => *hold = Some(child),
            Err(err) => eprintln!("couldn't start caffeinate: {err}"),
        }
    }
}

/// pmset's first line names the power source: "Now drawing from 'AC Power'".
fn plugged_in() -> bool {
    Command::new("/usr/bin/pmset").args(["-g", "ps"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().next().is_some_and(|l| l.contains("AC Power")))
}
