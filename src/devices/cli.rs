//! What's done with the recovery key on a computer: letting back in someone who lost every device, and who holds it. The
//! key is read here, on the machine of whoever runs this, from a file only they can name, so these are commands and not
//! tools: a node never opens a file for it.

use anyhow::{Context, Result, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

use crate::cli::{Command, Given};
use crate::config::{CLI, NAME, YOURS};
use crate::keys;

pub fn commands() -> Command {
    Command {
        help: vec![
            ("devices approve <code> --recovery-key <file> [--confirm]".into(), format!("let back in someone who lost every device, on a computer running {NAME}, with {YOURS}'s recovery key from a file, or - for stdin; without --confirm, says whose device it is")),
            ("devices holders".into(), "who holds the recovery key, and which of their devices have it".into()),
            ("devices holders add|remove <user>".into(), "for an admin: name someone as holding the recovery key, or take them off".into()),
            ("devices holders seal --recovery-key <file> [--confirm]".into(), "seal the recovery key, from a file or - for stdin, to the devices of those named; without --confirm, says which".into()),
        ],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["devices", "approve", code @ ..] => {
            let Some(file) = g.flag("recovery-key") else { return Ok(false) };
            let code = code.first().copied().or(g.flag("code")).context("give the code the new device shows")?;
            let recovery = keys::recovery_key_in(file)?;
            let answer = g.node.post_for(g.agent, "/v1/devices/approve-with-recovery", json!({ "code": code, "confirm": g.has("confirm"), "recovery": recovery }))?;
            println!("{}", answer["said"].as_str().unwrap_or(""));
        }
        ["devices", "holders"] => holders(&g.node.get("/v1/devices/recovery-holders")?),
        ["devices", "holders", how @ ("add" | "remove"), user] => {
            let answer = g.node.post_for(g.agent, "/v1/devices/recovery-holder", json!({ "user": user, "holds": *how == "add" }))?;
            println!("{}", answer["said"].as_str().unwrap_or(""));
        }
        ["devices", "holders", "seal"] => seal(g)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn holders(said: &Value) {
    let holders = said["holders"].as_array().map(Vec::as_slice).unwrap_or_default();
    if holders.is_empty() {
        println!("No one is named as holding the recovery key.");
    }
    for holder in holders {
        println!("{}", holder["name"].as_str().unwrap_or(""));
        let devices = holder["devices"].as_array().map(Vec::as_slice).unwrap_or_default();
        if devices.is_empty() {
            println!("  no device that can hold it: a phone, or an app on a computer without a node");
        }
        for device in devices {
            println!("  {}: {}", device["name"].as_str().unwrap_or(""), if device["holds"] == true { "has it" } else { "doesn't have it yet" });
        }
    }
}

/// Seals the recovery key from its file to each named holder's device that doesn't have it, here on this machine: the
/// node keeping them only ever sees what's sealed.
fn seal(g: &Given) -> Result<()> {
    let file = g.flag("recovery-key").context("give --recovery-key: the file the recovery key is in, or - for stdin")?;
    let secret = keys::unhex(&keys::recovery_key_in(file)?)?;
    let said = g.node.get("/v1/devices/recovery-holders")?;
    let public = ed25519_dalek::SigningKey::from_bytes(&secret).verifying_key().to_bytes();
    ensure!(said["recovery"].as_str() == Some(keys::hex(&public).as_str()), "That file doesn't hold {YOURS}'s recovery key.");
    let mut owed = Vec::new();
    for holder in said["holders"].as_array().into_iter().flatten() {
        for device in holder["devices"].as_array().into_iter().flatten().filter(|d| d["holds"] != true) {
            owed.push((holder["name"].as_str().unwrap_or(""), device));
        }
    }
    if owed.is_empty() {
        println!("Every named holder's device that can hold it has it. `{CLI} devices holders` shows them.");
        return Ok(());
    }
    for (name, device) in &owed {
        let label = device["name"].as_str().unwrap_or("");
        if !g.has("confirm") {
            println!("Would seal it to {label}, {name}'s device.");
            continue;
        }
        let (to, seal) = (device["key"].as_str().unwrap_or(""), keys::unhex(device["seal"].as_str().unwrap_or(""))?);
        g.node.post_for(g.agent, "/v1/devices/recovery-holders/seal", json!({ "to": to, "sealed": B64U.encode(&keys::seal_to(&seal, &secret)?) }))?;
        println!("Sealed it to {label}, {name}'s device.");
    }
    if !g.has("confirm") {
        println!("Nothing sealed yet. Whoever holds it can read everything in {YOURS}. Run it again with --confirm.");
    }
    Ok(())
}
