//! The install's bucket as a device reaches it holding no credentials: each request goes to a URL the URL signer
//! (`cloudflare/signer`) presigned, asked at the install's address with a request signed by the device's own key
//! (`keys::signed_request`, `one/urls`). URLs are kept until a minute before they stop working, and reading or
//! creating a database's step asks for the next positions too, and the signatures beside them, so a device catching up
//! or writing on asks once.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use super::Ask;
use super::{Failed, Trouble};

/// How many positions after a step a read, HEAD or create of it asks for too.
const READ_AHEAD: u64 = 15;
const CREATE_AHEAD: u64 = 3;

/// Where the install's address is, as the device knows it now.
pub type Address = Arc<dyn Fn() -> Result<String> + Send + Sync>;

/// Asks the URL signer for a device.
pub struct Signer {
    address: Address,
    key: ed25519_dalek::SigningKey,
    /// What every URL it's given starts with: the bucket's own, `<endpoint>/<bucket>`.
    base: String,
    /// URLs by `<kind> <name>` (read, head, create), with until when (ms) they work.
    kept: Mutex<HashMap<String, (String, u64)>>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the URL signer, for {}", self.base)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

impl Signer {
    pub fn new(address: Address, key: ed25519_dalek::SigningKey, base: String) -> Signer {
        Signer { address, key, base: base.trim_end_matches('/').to_string(), kept: Default::default() }
    }

    /// Asks the signer: `fields` signed, → its answer.
    pub fn ask(&self, fields: Value) -> Result<Value> {
        let address = (self.address)()?;
        let body = crate::keys::signed_request(&self.key, "one/urls", fields, now_ms());
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
        let mut resp = agent.post(format!("{}/urls", address.trim_end_matches('/'))).send_json(body).map_err(|e| Failed(Trouble::Unreachable, format!("couldn't be reached ({address}: {e})")))?;
        let status = resp.status().as_u16();
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        let said = out["error"].as_str().unwrap_or("nothing").to_string();
        match status {
            200 => Ok(out),
            401 | 403 => Err(Failed(Trouble::Refused, format!("won't sign for this device: {said}")).into()),
            500.. => Err(Failed(Trouble::Unreachable, format!("'s URL signer said {status}: {said}")).into()),
            _ => bail!("the URL signer said {status}: {said}"),
        }
    }

    /// Whether each object is in the bucket with the ETag given, asked in one trip: ETags open nothing.
    pub fn confirm(&self, objects: &[(&str, &str)]) -> Result<BTreeMap<String, bool>> {
        let mut out = BTreeMap::new();
        for chunk in objects.chunks(100) {
            let asked: Vec<Value> = chunk.iter().map(|(name, etag)| json!({ "name": name, "etag": etag })).collect();
            let given = self.ask(json!({ "confirm": asked }))?;
            for (name, _) in chunk {
                out.insert(name.to_string(), given["confirm"][*name] == true);
            }
        }
        Ok(out)
    }

    /// A request's URL, presigned: a read, a HEAD, a create (`If-None-Match: *`), a replace of the version an ETag
    /// names, a delete, or a page of a list.
    pub fn sign(&self, ask: &Ask) -> Result<String> {
        let header = |h: &str| ask.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(h)).map(|(_, v)| *v);
        let kind = match (ask.method, ask.name.is_empty(), header("if-none-match"), header("if-match")) {
            ("GET", true, None, None) => return self.list(ask),
            ("GET", false, None, None) => "read",
            ("HEAD", false, None, None) => "head",
            ("PUT", false, Some("*"), None) => "create",
            ("PUT", false, None, Some(tag)) => return self.only("replace", json!({ "replace": [{ "name": ask.name, "etag": tag }] }), ask.name),
            ("DELETE", false, None, None) => return self.only("remove", json!({ "remove": [ask.name] }), ask.name),
            _ => bail!("the bucket is only read, listed, created in, replaced by ETag and deleted from"),
        };
        ensure!(ask.query.is_empty(), "an object is reached by its name alone");
        let id = format!("{kind} {}", ask.name);
        if let Some((url, until)) = self.kept.lock().unwrap().get(&id) {
            if *until > now_ms() + 60_000 {
                return Ok(url.clone());
            }
        }
        // A database's step or the signature beside one: the positions after it too, which come next, and the
        // signature beside each.
        let ahead = if kind == "create" { CREATE_AHEAD } else { READ_AHEAD };
        let items: Vec<Value> = match crate::db::position(ask.name) {
            Some((prefix, at)) => {
                let sigs = (at..=at + ahead).filter(|_| crate::db::signed_beside(prefix)).map(|n| json!(crate::db::sig_name(prefix, n)));
                std::iter::once(json!({ "steps": prefix, "from": at, "to": at + ahead })).chain(sigs).collect()
            }
            None => vec![json!(ask.name)],
        };
        let given = self.ask(json!({ kind: items }))?;
        let until = given["until"].as_u64().context("the URL signer didn't say until when")?;
        let mut kept = self.kept.lock().unwrap();
        kept.retain(|_, (_, u)| *u > now_ms());
        for (name, url) in given[kind].as_object().into_iter().flatten() {
            if let Some(url) = url.as_str().filter(|u| self.ours(u)) {
                kept.insert(format!("{kind} {name}"), (url.to_string(), until));
            }
        }
        kept.get(&id).map(|(url, _)| url.clone()).context("the URL signer didn't sign that, for the install's bucket")
    }

    /// One URL of a kind that's asked for each time.
    fn only(&self, kind: &str, fields: Value, name: &str) -> Result<String> {
        let given = self.ask(fields)?;
        given[kind][name].as_str().filter(|u| self.ours(u)).map(str::to_string).context("the URL signer didn't sign that, for the install's bucket")
    }

    /// A page of a list: its prefix, delimiter and continuation token from the query, as S3 takes them.
    fn list(&self, ask: &Ask) -> Result<String> {
        let mut page = serde_json::Map::new();
        for (k, v) in ask.query {
            match *k {
                "list-type" if *v == "2" => {}
                "prefix" => drop(page.insert("prefix".into(), json!(v))),
                "delimiter" => drop(page.insert("delimiter".into(), json!(v))),
                "continuation-token" => drop(page.insert("token".into(), json!(v))),
                _ => bail!("the bucket is listed only by prefix"),
            }
        }
        page.entry("prefix").or_insert(json!(""));
        let given = self.ask(json!({ "list": [page] }))?;
        given["list"][0].as_str().filter(|u| self.ours(u)).map(str::to_string).context("the URL signer didn't sign that, for the install's bucket")
    }

    /// Whether a URL is the install's bucket's, so a signer elsewhere can't send this device anywhere else.
    fn ours(&self, url: &str) -> bool {
        url.strip_prefix(&self.base).is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?'))
    }
}
