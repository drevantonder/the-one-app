//! What's done with the recovery key on a computer: letting back in someone who lost every device, and who holds it. The
//! key is read here, on the machine of whoever runs this, from a file only they can name, so these are commands and not
//! tools: a node never opens a file for it.

use anyhow::{Context, Result, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

use crate::cli::{Command, Given};
use ::custom_build::config::{CLI, NAME, YOURS};

pub fn commands() -> Command {
    Command {
        help: vec![
            ("devices approve <code> --recovery-key <file> [--confirm]".into(), format!("let back in someone who lost every device, on a computer running {NAME}, with {YOURS}'s recovery key from a file, or - for stdin; without --confirm, says whose device it is")),
            ("devices holders".into(), "who holds the recovery key, and which of their devices have it".into()),
            ("devices holders add|remove <user>".into(), "for an admin: name someone as holding the recovery key, or take them off".into()),
            ("devices holders seal [<code>] --recovery-key <file> [--confirm]".into(), "seal the recovery key, from a file or - for stdin, to the device of someone named that shows this code on its own screen (`devices code` there); with no code, says which are waiting".into()),
            ("devices restore <code> [--confirm]".into(), "for someone holding the recovery key: give the device showing this code, on a user who lost every device, their keys; an admin lets it in first".into()),
            ("devices recovery-key replace --recovery-key <file> --new-key-file <file> [--confirm]".into(), "replace the recovery key, for one that was seen or stolen: a new one is written to the new file (or read from it, to finish a run that stopped), every scope gets new keys, and old holders' copies stop counting; nothing already sealed is deleted".into()),
            ("devices code".into(), "this device's code, for someone sealing it the recovery key or restoring you".into()),
        ],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["devices", "approve", code @ ..] => {
            let Some(file) = g.flag("recovery-key") else { return Ok(false) };
            let code = code.first().copied().or(g.flag("code")).context("give the code the new device shows")?;
            let recovery = key::recovery_key_in(file)?;
            let answer = g.node.post_for(g.agent, "/api/devices/approve-with-recovery", json!({ "code": code, "confirm": g.has("confirm"), "recovery": recovery }))?;
            println!("{}", answer["said"].as_str().unwrap_or(""));
        }
        ["devices", "holders"] => holders(&g.node.get("/api/devices/recovery-holders")?),
        ["devices", "holders", how @ ("add" | "remove"), user] => {
            let answer = g.node.post_for(g.agent, "/api/devices/recovery-holder", json!({ "user": user, "holds": *how == "add" }))?;
            println!("{}", answer["said"].as_str().unwrap_or(""));
        }
        ["devices", "holders", "seal", code @ ..] => seal(g, code.first().copied())?,
        ["devices", "restore", code] => {
            let answer = g.node.post_for(g.agent, "/api/devices/restore", json!({ "code": code, "confirm": g.has("confirm") }))?;
            println!("{}", answer["said"].as_str().unwrap_or(""));
        }
        ["devices", "recovery-key", "replace"] => replace(g)?,
        ["devices", "code"] => println!("{}", g.node.get("/api/devices/code")?["code"].as_str().unwrap_or("")),
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

/// Replaces the recovery key. The new one is written to its file first, unreadable by anyone else, so a failure later
/// never leaves a network whose key is lost; and read from it on a run that finishes one that stopped.
fn replace(g: &Given) -> Result<()> {
    let recovery = key::recovery_key_in(g.flag("recovery-key").context("give --recovery-key: the file the recovery key is in, or - for stdin")?)?;
    let at = g.flag("new-key-file").context("give --new-key-file: a file for the new key, which goes in your admins' password manager")?;
    let confirm = g.has("confirm");
    let next = key::next_recovery_key(at, confirm)?;
    let answer = g.node.post_for(g.agent, "/api/devices/recovery-key/replace", json!({ "recovery": recovery, "next": next, "confirm": confirm }))?;
    println!("{}", answer["said"].as_str().unwrap_or(""));
    if confirm {
        println!("The new recovery key is in {at}.");
    }
    Ok(())
}

/// Seals the recovery key from its file to the one named holder's device showing `code`, here on this machine: the node
/// keeping it only ever sees what's sealed. What's listed comes from a node, which could be hacked, and a device's
/// `platform` is only what it says of itself, so the only thing that says this key is really that device is the code
/// computed here from the very key and seal key used, against what a person read off that device's own screen.
fn seal(g: &Given, code: Option<&str>) -> Result<()> {
    let file = g.flag("recovery-key").context("give --recovery-key: the file the recovery key is in, or - for stdin")?;
    let secret = key::unhex(&key::recovery_key_in(file)?)?;
    let said = g.node.get("/api/devices/recovery-holders")?;
    let recovery = ed25519_dalek::SigningKey::from_bytes(&secret);
    ensure!(said["recovery"].as_str() == Some(key::hex(&recovery.verifying_key().to_bytes()).as_str()), "That file doesn't hold {YOURS}'s recovery key.");
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
    let Some(code) = code else {
        for (name, device) in &owed {
            println!("Waiting: {}, {name}'s device.", device["name"].as_str().unwrap_or(""));
        }
        println!("Whoever holds it can read everything in {YOURS}. To seal it to one, run `{CLI} devices code` on that device and give what it shows, from its own screen, here.");
        return Ok(());
    };
    let wanted: String = code.to_uppercase().chars().filter(char::is_ascii_alphanumeric).collect();
    let (name, device, to, seal) = owed
        .iter()
        .find_map(|(name, device)| {
            let endpoint = key::Endpoint { key: key::unhex(device["key"].as_str()?).ok()?, seal: key::unhex(device["seal"].as_str()?).ok()? };
            (key::join_code(&endpoint) == wanted).then_some((*name, *device, endpoint.key, endpoint.seal))
        })
        .context("No device waiting for it shows that code. Read it again from that device's own screen: a different code means what this computer was told isn't that device.")?;
    let label = device["name"].as_str().unwrap_or("");
    if !g.has("confirm") {
        println!("Would seal it to {label}, {name}'s device. Whoever holds it can read everything in {YOURS}. Run it again with --confirm.");
        return Ok(());
    }
    let to = key::hex(&to);
    let sealed = key::seal_to(&seal, &secret)?;
    let copy = key::held_copy(&recovery, &to, &sealed);
    g.node.post_for(g.agent, "/api/devices/recovery-holders/seal", json!({ "to": to, "copy": B64U.encode(&copy) }))?;
    println!("Sealed it to {label}, {name}'s device.");
    Ok(())
}
