//! What every device and the Workers are on, and being asked to change that. Each device tells the relay its version when
//! it reads who's connected (`read_connected`), which keeps what it answers here, and what a deploy says of the Workers.
//! Someone asking this device to take the newest published build is an ask the asker's device signed and left for this
//! key (`{ update }`, `POST /relay/tell`). The relay carries it blind, so this checks who signed it and that they may remove
//! this device, as who's who says, then starts `one update`, and does nothing else: an update that fails leaves this
//! device as it was, and says why in its log (`one logs <device> --grep update`). The device the Workers were last deployed
//! from is asked the same way to deploy them again (`{ cloudflare }`, which the relay leaves for it), by an admin, and starts
//! only `cloudflare deploy`, as any admin's device may.

use std::process::Stdio;

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use ::custom_build::config::{CLI, YOURS};
use ::scope::rank::CLOUDFLARE;
use ::scope::scopes;
use ::key::hex;
use ::oned_update::latest::{Latest, Thing, wanted};

use super::Node;
use ::custom_build::now_ms;

/// How the record names what a version is on.
type Find = for<'a> fn(&'a Latest, &'a str) -> Option<&'a Thing>;

impl Node {
    /// What each device told the relay it's on: `{ key: { version, newest, behind } }`, and the Workers as `cloudflare`, with
    /// the version of the Cloudflare node (`node`) if there is one, and the name of the device they were last deployed from (`from`). `newest` is the build published for what it runs on,
    /// from the signed record, which a browser can't read for itself (a redirect to the bucket). The Workers are behind
    /// if their sources or the web client are; the Cloudflare node is on what the device that deploys it is, so it follows that.
    pub async fn versions(&self) -> Result<Value> {
        let (told, workers) = self.with_or((Value::Null, Value::Null), |i| Ok((i.versions.clone(), i.workers.clone())));
        let address = self.address().unwrap_or_default();
        // A record that's missing or doesn't read just offers no Update: the device that takes one checks it again itself.
        let builds = tokio::task::spawn_blocking(move || ::oned_update::latest::fetch(&address).ok().flatten().and_then(|text| Latest::read(platform::mode::here().release_key(), &text).ok())).await?.unwrap_or_default();
        let of = |version: &str, find: Find| -> Option<Value> {
            let (commit, committed, on) = ::custom_build::protocol::told(version)?;
            let newest = find(&builds, on);
            Some(json!({ "version": commit, "newest": newest.map(|b| &b.commit), "behind": newest.is_some_and(|b| wanted(b, commit, committed)) }))
        };
        let mut out: serde_json::Map<String, Value> = told.as_object().map(|v| v.iter().filter_map(|(key, version)| Some((key.clone(), of(version.as_str()?, Latest::to_update_to)?))).collect()).unwrap_or_default();
        let part = |of_what: &str, find: Find| workers[of_what].as_str().and_then(|v| of(v, find));
        if let (Some(sources), Some(web)) = (part("workers", Latest::get), part("web", Latest::get)) {
            let from = workers["from"].as_str().and_then(|k| ::key::unhex(k).ok());
            let from = from.and_then(|key| self.with_or(None, |i| Ok(i.r.db().machine(&key)?.map(|d| d.name))));
            out.insert(CLOUDFLARE.into(), json!({ "version": sources["version"], "newest": sources["newest"], "behind": sources["behind"] == true || web["behind"] == true, "node": part("node", Latest::to_update_to).map(|n| n["version"].clone()), "from": from }));
        }
        Ok(Value::Object(out))
    }

    /// What the relay left for this device, if anything (`asked` in its answer).
    pub(super) fn heard_ask(&self, asked: &Value) {
        if !asked.is_null() {
            self.with_or((), |i| Ok(heard(i.r.db(), &self.0.id, asked)));
        }
    }

    /// The device that signed this ask, if it's one who may remove this device (an admin's, for the Workers), and it asks
    /// this device: else why not.
    pub fn who_asked_to_update(&self, asked: &Value) -> Result<(String, Ask)> {
        self.with(|i| who_asked(i.r.db(), &self.0.id, asked))
    }
}

/// What the relay left for the device `me` (`asked` in its answer to `POST /relay/computes`), as who's who says: taken
/// if who signed it may ask it, said in this program's log either way. A computer's onet hears its device's.
pub fn heard(who: &scopes::ScopeDb, me: &::key::Key, asked: &Value) {
    match who_asked(who, me, asked).and_then(|(by, ask)| Ok((by, ask.start()?, ask))) {
        Ok((by, (), ask)) => ::device_logs::say(&format!("asked by {by}: starting {CLI} {}", ask.command().join(" "))),
        Err(err) => ::device_logs::say(&format!("an ask to update wasn't taken: {err:#}")),
    }
}

/// The device that signed `asked`, if it's one who may remove the device `me` (an admin's, for the Workers), and it asks
/// that device: else why not.
fn who_asked(who: &scopes::ScopeDb, me: &::key::Key, asked: &Value) -> Result<(String, Ask)> {
    let (by, fields) = ::key::check_request(::custom_build::protocol::RELAY, asked, now_ms())?;
    // The relay leaves the Workers' ask for the device they were last deployed from.
    let ask = if fields["cloudflare"] == true { Ask::Deploy } else { Ask::Update };
    ensure!(ask == Ask::Deploy || fields["update"] == hex(me), "it asks another device");
    let asker = who.machine(&by)?.filter(|d| !d.removed);
    let Some(asker) = asker else { anyhow::bail!("{YOURS} doesn't list the device that asked") };
    match ask {
        Ask::Update => ensure!(who.may(&asker.user, "remove", &scopes::device(me)), "{} may not remove this device", asker.name),
        Ask::Deploy => ensure!(who.may(&asker.user, "change-settings", scopes::NETWORK), "{} isn't an admin's device", asker.name),
    }
    Ok((asker.name, ask))
}

/// What a device is asked to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    /// Take the newest published build.
    Update,
    /// Deploy the Workers again, which were last deployed from this device.
    Deploy,
}

impl Ask {
    fn command(self) -> &'static [&'static str] {
        match self {
            Ask::Update => &["update"],
            Ask::Deploy => &["cloudflare", "deploy"],
        }
    }

    /// Starts it detached. An update replaces this program and then asks this process to stop, so what it says last is
    /// said to nobody: only a failure leaves words here.
    fn start(self) -> Result<()> {
        let words = self.command();
        let started = platform::os::named(CLI)?.args(words).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        std::thread::spawn(move || match started.wait_with_output() {
            Ok(out) => ::device_logs::say(&format!("{CLI} {}: {}{}", words.join(" "), String::from_utf8_lossy(&out.stdout).trim(), String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => ::device_logs::say(&format!("{CLI} {}: {err}", words.join(" "))),
        });
        Ok(())
    }
}
