//! What every device is on, and being asked to change that. Each device tells the relay its version when it reads who's
//! connected (`read_connected`), which keeps what it answers here. Someone asking this device to take the newest
//! published build is an ask the asker's device signed
//! and left for this key (`{ update }`, `POST /relay/tell`). The relay carries it blind, so this checks who signed it
//! and that they may remove this device, as who's who says, then starts `one update`, and does nothing else: an update
//! that fails leaves this device as it was, and says why in its log (`one logs <device> --grep update`).

use std::process::Stdio;

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use crate::config::{CLI, YOURS};
use crate::database::scopes;
use crate::keys::hex;
use crate::latest::{Latest, Thing, wanted};
use crate::logs;

use super::{Node, now_ms};

impl Node {
    /// What each device told the relay it's on: `{ key: { version, newest, behind } }`. `newest` is the build published
    /// for what it runs on, from the signed record, which a browser can't read for itself (a redirect to the bucket).
    pub async fn versions(&self) -> Result<Value> {
        let told = self.with_or(Value::Null, |i| Ok(i.versions.clone()));
        let address = self.address().unwrap_or_default();
        // A record that's missing or doesn't read just offers no Update: the device that takes one checks it again itself.
        let builds = tokio::task::spawn_blocking(move || crate::latest::fetch(&address).ok().flatten().and_then(|text| Latest::read(crate::mode::here().release_key(), &text).ok())).await?.unwrap_or_default();
        let of = |version: &str| -> Option<Value> {
            let mut words = version.split(' ');
            let (commit, committed, target) = (words.next()?, words.next()?.parse().ok()?, words.next()?);
            let newest = builds.get(&Thing::node_name(target));
            Some(json!({ "version": commit, "newest": newest.map(|b| &b.commit), "behind": newest.is_some_and(|b| wanted(b, commit, committed, "")) }))
        };
        Ok(told.as_object().map(|v| v.iter().filter_map(|(key, version)| Some((key.clone(), of(version.as_str()?)?))).collect()).unwrap_or_default())
    }

    /// What the relay left for this device, if anything (`asked` in its answer).
    pub(super) fn heard_ask(&self, asked: &Value) {
        if asked.is_null() {
            return;
        }
        match self.who_asked_to_update(asked).and_then(|by| Ok((by, start_update()?))) {
            Ok((by, ())) => logs::say(&format!("asked to update by {by}: starting {CLI} update")),
            Err(err) => logs::say(&format!("an ask to update wasn't taken: {err:#}")),
        }
    }

    /// The device that signed this ask, if it's one who may remove this device, and it asks this device: else why not.
    pub fn who_asked_to_update(&self, asked: &Value) -> Result<String> {
        let (by, fields) = crate::keys::check_request(crate::protocol::RELAY, asked, now_ms())?;
        ensure!(fields["update"] == hex(&self.0.id), "it asks another device");
        self.with(|i| {
            let who = i.r.db();
            let asker = who.machine(&by)?.filter(|d| !d.removed);
            let Some(asker) = asker else { anyhow::bail!("{YOURS} doesn't list the device that asked") };
            ensure!(who.may(&asker.user, "remove", &scopes::device(&self.0.id)), "{} may not remove this device", asker.name);
            Ok(asker.name)
        })
    }
}

/// Starts `one update`, which replaces this program and then asks this process to stop, so what it says last is said
/// to nobody: only a failure leaves words here.
fn start_update() -> Result<()> {
    let update = crate::os::named(CLI)?.arg("update").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    std::thread::spawn(move || match update.wait_with_output() {
        Ok(out) => logs::say(&format!("{CLI} update: {}{}", String::from_utf8_lossy(&out.stdout).trim(), String::from_utf8_lossy(&out.stderr).trim())),
        Err(err) => logs::say(&format!("{CLI} update: {err}")),
    });
    Ok(())
}
