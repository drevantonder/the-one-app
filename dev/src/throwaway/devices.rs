//! The devices a client is put on, borrowed from this machine's own network's Agent Devices (not the throwaway network's,
//! which has none), and what the clients on them share.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use one::config::NAME;
use serde_json::{Value, json};

/// The devices borrowed for the throwaway network, by kind: `{ "android": "<name>" }`.
pub(crate) fn devices(dir: &Path) -> serde_json::Map<String, Value> {
    one::files::read_json(&dir.join("devices.json"))
}

/// One of this machine's network's Agent Devices tools (not the throwaway network's, which has none). When the node
/// restarts during it (an update), or its service is still coming back, it's asked again once they answer.
pub(crate) fn agent_devices(tool: &str, input: Value) -> Result<Value> {
    Ok(again(|| crate::node().post("/v1/call", json!({ "app": "agent-devices", "tool": tool, "input": input, "client": "one-dev throwaway" })))?["ok"].clone())
}

/// What `ask` gives, asking again for up to 3 minutes while this machine's node or a service of its restarts.
pub(crate) fn again(ask: impl Fn() -> Result<Value>) -> Result<Value> {
    let start = Instant::now();
    loop {
        match ask() {
            Err(err) if start.elapsed() < Duration::from_secs(180) && one::protocol::Coded::is(&err, one::protocol::code::RESTARTING) => {
                eprintln!("{NAME} is restarting here; asking again once it's back…");
                std::thread::sleep(Duration::from_secs(5));
            }
            out => return out,
        }
    }
}

pub(crate) fn on_device(device: &str, command: &str) -> Result<String> {
    let out = agent_devices("run", json!({ "device": device, "command": command }))?;
    Ok(out.as_str().map(str::to_string).unwrap_or_else(|| out.to_string()))
}

/// The agent devices on this machine's network's devices, each with who borrowed it and what for (Computer's own list
/// leaves that out).
pub(crate) fn listed_devices() -> Value {
    agent_devices("_list", json!({})).map(|l| l["devices"].clone()).unwrap_or_default()
}

/// What a device borrowed for this checkout's throwaway network says it's for: `one-dev throwaway client <kind>, for <checkout>`.
pub(crate) fn borrowed_for(checkout: &Path, kind: &str) -> String {
    format!("one-dev throwaway client {kind}, for {}", checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

/// Gives back every device borrowed for the throwaway network: those devices.json names, and any Agent Devices lists as
/// borrowed for this checkout (what the file lost, or never got because the node restarted as it was lent).
pub(crate) fn give_back(dir: &Path, checkout: &Path) {
    let mut all: Vec<(String, String)> = devices(dir).into_iter().filter_map(|(kind, d)| Some((kind, d.as_str()?.to_string()))).collect();
    let ours = format!(", for {}", checkout.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    for d in listed_devices().as_array().into_iter().flatten() {
        if let (Some(name), Some(what)) = (d["name"].as_str(), d["borrowed"]["for"].as_str())
            && what.starts_with("one-dev throwaway client ")
            && what.ends_with(&ours)
            && !all.iter().any(|(_, n)| n == name)
        {
            all.push((what["one-dev throwaway client ".len()..what.len() - ours.len()].to_string(), name.to_string()));
        }
    }
    for (kind, device) in all {
        if let Err(err) = agent_devices("give_back", json!({ "device": device })) {
            eprintln!("Couldn't give back the {kind} device {device}: {err:#}");
        }
    }
}


pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Runs a build script of the checkout's, at low priority, as other threads share this machine.
pub(crate) fn build(checkout: &Path, script: &str, envs: &[(&str, &str)]) -> Result<String> {
    eprintln!("Building {script}…");
    let out = std::process::Command::new("nice").arg(checkout.join(script)).envs(envs.iter().copied()).current_dir(checkout).stderr(std::process::Stdio::inherit()).output()?;
    anyhow::ensure!(out.status.success(), "{script} failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A client that didn't sign in, with the last of what it said (`said`: who said it, and what), which says why; a client that
/// said nothing says so, and where else to look.
pub(crate) fn didnt_sign_in(what: &str, said: &[(&str, String)], elsewhere: &str) -> anyhow::Error {
    let mut message = format!("the {what} didn't sign in");
    let mut any = false;
    for (who, text) in said {
        let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        if !lines.is_empty() {
            any = true;
            message += &format!("\n{who} said, last:");
            for line in &lines[lines.len().saturating_sub(15)..] {
                message += &format!("\n  {}", line.chars().take(300).collect::<String>());
            }
        }
    }
    if !any {
        message += &format!(" and said nothing: {elsewhere}");
    }
    anyhow::anyhow!(message)
}

/// Whether text is the code a new device shows to be approved: "ABCD EFGH IJKL MNOP".
pub(crate) fn is_code(text: &str) -> bool {
    text.len() == 19 && text.char_indices().all(|(i, c)| if i % 5 == 4 { c == ' ' } else { c.is_ascii_uppercase() || ('2'..='7').contains(&c) })
}

/// A native client's code, approved by the throwaway network's machine, as its user's device: then it's let in.
pub(crate) fn approve_code(dir: &Path, code: &str) -> Result<()> {
    eprintln!("Approving the device that shows {code}…");
    let approved = super::network::node_in(dir)?.post("/v1/devices/approve", json!({ "code": code.replace(' ', ""), "confirm": true }));
    // Let in already, by an invite naming their email, by that same machine: nothing waits.
    match approved {
        Err(err) if format!("{err:#}").contains("No device is waiting") => Ok(()),
        r => r.map(drop),
    }
}
