//! The client's part of encryption: what only a device does. A person's key is made on a device of theirs and never
//! leaves one unencrypted: it's kept encrypted to each device they let in, and what a server is given of it is one
//! artifact's keys, encrypted to that server. What a person types that's secret is encrypted on the device for the
//! servers that open what it's kept in, so nothing that carries a call can read it.
//!
//! None of that is a command of its own. A tool says which of its inputs a device fills in, by the input's `format`
//! in the tool's schema, and whatever calls the tool through this client (an app, the CLI, an agent over MCP) gets
//! them filled in the same way:
//!
//! - `secret`: the text given is encrypted for the servers that the plugin's tool named by `opened_by` lists.
//! - `artifact-keys`: the keys of the artifact that tool names, for each server in the input `servers`, from this
//!   device's copy of its owner's key. A server that was listed and no longer is starts a new epoch.
//! - `owner-key`: the owner's key for the device in the input `device`: a new one if they have none, and otherwise
//!   this device's copy, or the recovery key given.
//! - `backup-recipient`: the public half of the backup key (`cipher::backups`) of whoever the plugin's tool named
//!   by `opened_by` says is its `owner`: the network, for Backups.
//! - `backup-key`: that backup key itself, encrypted to each server the same tool lists, for a restore.
//! - `device` and `device-name`: this device's key and its name, where none is given.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use data_encoding::BASE64;
use iroh::PublicKey;
use serde_json::{Map, Value, json};

use crate::Client;
use crate::cipher::{self, Key};

/// The inputs a device fills in, by their tool's name: each one's name, and what its schema says of it.
pub(crate) type Fills = HashMap<String, Vec<(String, Value)>>;

/// The encryption plugin's tool that says what a device holds of its owner's key.
const HOLDS: &str = "encryption_key";

/// What a device fills in, in the order it does: an owner's key is for a device, which is filled in first.
const FORMATS: [&str; 7] =
    ["device", "device-name", "secret", "owner-key", "artifact-keys", "backup-recipient", "backup-key"];

/// The inputs a device fills in of each tool a server listed.
pub(crate) fn fills(listed: &Value) -> Fills {
    let tools = listed["tools"].as_array().map(Vec::as_slice).unwrap_or_default();
    let filled = |tool: &Value| {
        let inputs = tool["inputSchema"]["properties"].as_object()?;
        let mut fills: Vec<(String, Value)> = inputs
            .iter()
            .filter(|(_, input)| FORMATS.iter().any(|format| input["format"] == *format))
            .map(|(name, input)| (name.clone(), input.clone()))
            .collect();
        fills.sort_by_key(|(_, input)| FORMATS.iter().position(|format| input["format"] == *format));
        (!fills.is_empty()).then(|| Some((tool["name"].as_str()?.to_owned(), fills)))?
    };
    tools.iter().filter_map(filled).collect()
}

impl Client {
    /// The inputs a device fills in, as a server last listed its tools: asked for, if this client hasn't seen a list.
    async fn fills(&self) -> Result<Arc<Fills>> {
        if let Some(fills) = self.fills.lock().expect("no ask panics").clone() {
            return Ok(fills);
        }
        Box::pin(self.request("tools/list", json!({}))).await?;
        Ok(self.fills.lock().expect("no ask panics").clone().unwrap_or_default())
    }

    /// Fills in what this device does of a tool's inputs, if `request` calls a tool that has any.
    pub(crate) async fn fill(&self, request: &mut Value) -> Result<()> {
        let Some(tool) = request["params"]["name"].as_str().filter(|_| request["method"] == "tools/call") else {
            return Ok(());
        };
        let (tool, fills) = (tool.to_owned(), self.fills().await?);
        let Some(fills) = fills.get(&tool) else { return Ok(()) };
        let plugin = tool.split('_').next().unwrap_or_default();
        // What the device asks to fill these in is for whoever the call is.
        let who = request["params"]["_meta"]["for"].as_str().map(str::to_owned);
        let who = who.as_deref();
        if !request["params"]["arguments"].is_object() {
            request["params"]["arguments"] = json!({});
        }
        let inputs = request["params"]["arguments"].as_object_mut().expect("it was made one");
        for (name, says) in fills {
            let filled = match (says["format"].as_str().unwrap_or_default(), inputs.get(name).and_then(Value::as_str)) {
                ("device", None) => self.key.public().to_string(),
                ("device-name", None) => gethostname::gethostname().to_string_lossy().into_owned(),
                ("secret", Some(text)) => self.secret(plugin, says, text, who).await?,
                ("owner-key", typed) => self.owner_key_for(inputs, typed, who).await?,
                ("artifact-keys", None) => self.artifact_keys(plugin, says, inputs, who).await?,
                ("backup-recipient", None) => self.backups_key(plugin, says, who).await?.0.public().to_string(),
                ("backup-key", None) => self.backup_key(plugin, says, who).await?,
                _ => continue,
            };
            inputs.insert(name.clone(), filled.into());
        }
        Ok(())
    }

    /// Calls a tool for someone, if it says who, and gives what it answered.
    async fn tool_for(&self, name: &str, inputs: Value, who: Option<&str>) -> Result<Value> {
        let mut params = json!({"name": name, "arguments": inputs});
        if let Some(who) = who {
            params["_meta"]["for"] = who.into();
        }
        let answer = Box::pin(self.request("tools/call", params)).await?;
        let said = answer["content"][0]["text"].as_str().unwrap_or_default();
        if answer["isError"] == true {
            bail!("{said}")
        }
        Ok(serde_json::from_str(said).unwrap_or_else(|_| said.into()))
    }

    /// What the plugin's tool that an input names says of the artifact it's kept in: its name, its epoch, and the
    /// servers that open it.
    async fn opened_by(&self, plugin: &str, says: &Value, who: Option<&str>) -> Result<Value> {
        let tool = says["opened_by"].as_str().context("the input doesn't say which tool lists the servers")?;
        self.tool_for(&format!("{plugin}_{tool}"), json!({}), who).await
    }

    /// `text` encrypted to each server that opens what it's kept in, by the server's key.
    async fn secret(&self, plugin: &str, says: &Value, text: &str, who: Option<&str>) -> Result<String> {
        let opened_by = self.opened_by(plugin, says, who).await?;
        let servers = opened_by["servers"].as_array().map(Vec::as_slice).unwrap_or_default();
        if servers.is_empty() {
            bail!("No server opens what this is kept in yet. Choose one first, from a device that holds your key.")
        }
        let mut sealed = Map::new();
        for server in servers.iter().filter_map(Value::as_str) {
            let to: PublicKey = server.parse().context("that isn't a server's key")?;
            sealed.insert(
                server.to_owned(),
                BASE64.encode(&cipher::seal(&to, &cipher::for_secret(server), text.as_bytes())?).into(),
            );
        }
        Ok(Value::Object(sealed).to_string())
    }

    /// This device's copy of the key of `who`, or of whoever it asks for, with who that is: a person, a group or the
    /// network. It's kept encrypted to this device, and opened here each time it's needed.
    async fn owner_key_of(&self, who: Option<&str>) -> Result<(Key, String)> {
        let holds = self.tool_for(HOLDS, json!({}), who).await?;
        let (owner, me) = (holds["owner"].as_str().unwrap_or_default().to_owned(), self.key.public().to_string());
        let Some(slot) = holds["slot"].as_str() else {
            bail!("This device doesn't hold your key. Let it in from a device that does, or with your recovery key.")
        };
        let slot = BASE64.decode(slot.as_bytes()).context("that isn't a key for this device")?;
        let key = cipher::open(&self.key, &cipher::for_device(&owner, &me), &slot)?;
        Ok((cipher::key(&key)?, owner))
    }

    /// The key of `who`, or of whoever this client asks for, encrypted to the device the input `device` names: the
    /// one typed as a recovery key, this device's copy, or for someone who has none yet, a new one, for this device
    /// itself.
    async fn owner_key_for(&self, inputs: &Map<String, Value>, typed: Option<&str>, who: Option<&str>) -> Result<String> {
        let holds = self.tool_for(HOLDS, json!({}), who).await?;
        let (owner, me) = (holds["owner"].as_str().unwrap_or_default(), self.key.public().to_string());
        let device = inputs.get("device").and_then(Value::as_str).unwrap_or(&me);
        let key = match (typed, holds["check"].as_str()) {
            (Some(typed), Some(check)) => {
                let key = cipher::recovered(typed)?;
                if cipher::check(&key) != check {
                    bail!("That isn't your recovery key. Check it against what you wrote down.")
                }
                key
            }
            (Some(_), None) => bail!("There's no key to recover: nothing of yours is encrypted yet."),
            (None, Some(_)) => self.owner_key_of(who).await?.0,
            (None, None) if device == me => cipher::owner(),
            (None, None) => bail!("You have no key yet. It's made on the first device you set up, for itself."),
        };
        let to: PublicKey = device.parse().context("that isn't a device's key")?;
        let slot = cipher::seal(&to, &cipher::for_device(owner, device), key.as_slice())?;
        Ok(json!({"check": cipher::check(&key), "slot": BASE64.encode(&slot)}).to_string())
    }

    /// The keys of the artifact a plugin's tool names, encrypted to each server in the input `servers`. Taking a
    /// server off starts a new epoch, whose key it was never given.
    async fn artifact_keys(
        &self,
        plugin: &str,
        says: &Value,
        inputs: &Map<String, Value>,
        who: Option<&str>,
    ) -> Result<String> {
        let opened_by = self.opened_by(plugin, says, who).await?;
        let artifact = opened_by["artifact"].as_str().context("the tool didn't name the artifact")?;
        let was = opened_by["epoch"].as_u64().unwrap_or_default() as u32;
        let keys = |value: &Value| -> Vec<String> {
            value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
        };
        let (had, servers) = (keys(&opened_by["servers"]), keys(inputs.get("servers").unwrap_or(&Value::Null)));
        let epoch = if was == 0 || had.iter().any(|server| !servers.contains(server)) { was + 1 } else { was };
        let (owner, _) = self.owner_key_of(who).await?;
        let joined = cipher::joined(&cipher::artifact(&owner, artifact, epoch));
        let mut slots = Map::new();
        for server in &servers {
            let to: PublicKey = server.parse().context("that isn't a server's key")?;
            slots.insert(
                server.clone(),
                BASE64.encode(&cipher::seal(&to, &cipher::for_server(artifact, server), &joined)?).into(),
            );
        }
        Ok(json!({"artifact": artifact, "epoch": epoch, "was": was, "slots": slots}).to_string())
    }

    /// The backup key of whoever the plugin's tool that the input names says owns the backups, with what the tool
    /// said.
    async fn backups_key(&self, plugin: &str, says: &Value, who: Option<&str>) -> Result<(iroh::SecretKey, Value)> {
        let opened_by = self.opened_by(plugin, says, who).await?;
        let owner = opened_by["owner"].as_str().context("the tool didn't say whose backup key it is")?;
        Ok((cipher::backups(&self.owner_key_of(Some(owner)).await?.0), opened_by))
    }

    /// The backup key, encrypted to each server the plugin's tool lists, for one of them to restore with.
    async fn backup_key(&self, plugin: &str, says: &Value, who: Option<&str>) -> Result<String> {
        let (key, opened_by) = self.backups_key(plugin, says, who).await?;
        let mut sealed = Map::new();
        for server in opened_by["servers"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let to: PublicKey = server.parse().context("that isn't a server's key")?;
            let key = zeroize::Zeroizing::new(key.to_bytes());
            sealed.insert(
                server.to_owned(),
                BASE64.encode(&cipher::seal(&to, &cipher::for_restoring(server), key.as_slice())?).into(),
            );
        }
        Ok(Value::Object(sealed).to_string())
    }

    /// The recovery key of `who`, or of whoever this client asks for: a person, or a group or the network they own.
    /// It's their key as a person writes it down. Only a device that holds the key gives it, and only its window
    /// asks: no command or tool answers with one.
    pub async fn recovery_key(&self, who: Option<&str>) -> Result<String> {
        Ok(cipher::recovery(&self.owner_key_of(who).await?.0))
    }

    /// Lets this device hold the key of `who`, or of whoever this client asks for, with the recovery key its person
    /// typed in its window.
    pub async fn recover(&self, recovery_key: &str, who: Option<&str>) -> Result<()> {
        self.tool_for("encryption_let_in", json!({"key": recovery_key}), who).await.map(drop)
    }
}
