//! Letting back in someone who lost every device. The recovery key is read here, on the machine of whoever runs this,
//! from a file only they can name, so it's a command and not a tool: a node never opens a file for it.

use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::{Command, Given};
use crate::config::{NAME, YOURS};

pub fn commands() -> Command {
    Command {
        help: vec![("devices approve <code> --recovery-key <file> [--confirm]".into(), format!("let back in someone who lost every device, on a computer running {NAME}, with {YOURS}'s recovery key from a file, or - for stdin; without --confirm, says whose device it is"))],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    let Some(file) = g.flag("recovery-key") else { return Ok(false) };
    let words = g.words();
    let ["devices", "approve", code @ ..] = words.as_slice() else { return Ok(false) };
    let code = code.first().copied().or(g.flag("code")).context("give the code the new device shows")?;
    let recovery = crate::keys::recovery_key_in(file)?;
    let answer = g.node.post_for(g.agent, "/v1/devices/approve-with-recovery", json!({ "code": code, "confirm": g.has("confirm"), "recovery": recovery }))?;
    println!("{}", answer["said"].as_str().unwrap_or(""));
    Ok(true)
}
