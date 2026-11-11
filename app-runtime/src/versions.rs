//! What every device and the Workers are on, and being asked to change that. Each device tells the relay its version when
//! it reads who's connected (`read_connected`), which keeps what it answers here, and what a deploy says of the Workers.
//! Someone asking this device to take the newest published build is an ask the asker's device signed and left for this
//! key (`{ update }`, `POST /relay/tell`). The relay carries it blind, so this checks who signed it and that they may remove
//! this device, as who's who says, then starts `one update`, and does nothing else: an update that fails leaves this
//! device as it was, and says why in its log (`one logs <device> --grep update`). The device the Workers were last deployed
//! from is asked the same way to deploy them again (`{ cloudflare }`, which the relay leaves for it), by an admin, and starts
//! only `cloudflare deploy`, as any admin's device may.

use anyhow::Result;
use serde_json::{Value, json};

use ::scope::rank::CLOUDFLARE;
use ::oned_update::latest::{Latest, Thing, wanted};

use super::Host;
use ::devices::asked::{Ask, heard, who_asked};

/// How the record names what a version is on.
type Find = for<'a> fn(&'a Latest, &'a str) -> Option<&'a Thing>;

impl Host {
    /// What each device told the relay it's on: `{ key: { version, newest, behind } }`, and the Workers as `cloudflare`, with
    /// the version of oned on Cloudflare (`oned`) if there is one, and the name of the device they were last deployed from (`from`). `newest` is the build published for what it runs on,
    /// from the signed record, which a browser can't read for itself (a redirect to the bucket). The Workers are behind
    /// if their sources or the web client are; oned on Cloudflare is on what the device that deploys it is, so it follows that.
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
        // Each by its key, with its name, which is what a person reads.
        let named = |key: &str, mut it: Value| {
            it["name"] = json!(::key::unhex(key).map_or_else(|_| key.to_string(), |k| self.name_of(&k)));
            it
        };
        let mut out: serde_json::Map<String, Value> = told.as_object().map(|v| v.iter().filter_map(|(key, version)| Some((key.clone(), named(key, of(version.as_str()?, Latest::to_update_to)?)))).collect()).unwrap_or_default();
        let part = |of_what: &str, find: Find| workers[of_what].as_str().and_then(|v| of(v, find));
        if let (Some(sources), Some(web)) = (part("workers", Latest::get), part("web", Latest::get)) {
            let from = workers["from"].as_str().and_then(|k| ::key::unhex(k).ok());
            let from = from.and_then(|key| self.with_or(None, |i| Ok(i.r.db().machine(&key)?.map(|d| d.name))));
            out.insert(CLOUDFLARE.into(), json!({ "version": sources["version"], "newest": sources["newest"], "behind": sources["behind"] == true || web["behind"] == true, "oned": part("oned", Latest::to_update_to).map(|n| n["version"].clone()), "from": from }));
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

