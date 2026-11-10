//! The network's bucket as a device reaches it holding no credentials: each request goes to a URL the URL signer
//! (`cloudflare/signer`) presigned, asked at the network's address with a request signed by the device's own key
//! (`keys::signed_request`, `one/urls`). URLs are kept until a minute before they stop working, and reading or
//! creating a database's step asks for the next positions too, and the signatures beside them, so a device catching up
//! or writing on asks once.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use super::{Failed, Trouble};

/// How many positions after a step a read, HEAD or create of it asks for too, for a node; a client device only reads,
/// and reads a log from its start, so it asks for more at a time.
const READ_AHEAD: u64 = 15;
const CLIENT_READ_AHEAD: u64 = 31;
const CREATE_AHEAD: u64 = 3;

pub type Address = Arc<dyn Fn() -> Result<String> + Send + Sync>;

/// A request to sign: its method, the object's name ("" for the bucket), its query, and the headers it must carry.
#[derive(Debug)]
pub struct Ask<'a> {
    pub method: &'a str,
    pub name: &'a str,
    pub query: &'a [(&'a str, &'a str)],
    pub headers: &'a [(&'a str, &'a str)],
}

/// One pool of connections for every request of a bucket's, so a request after the first reaches it over a connection
/// already open, not a new TCP and TLS handshake: each is `timeout` seconds at most.
pub(super) fn pooled(pool: &'static std::sync::OnceLock<ureq::Agent>, timeout: u64) -> ureq::Agent {
    pool.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(timeout))).max_idle_connections_per_host(16).build().into()).clone()
}

/// The pool for asking the signer, and for a client device's downloads of the URLs it signs.
pub fn signer_requests() -> ureq::Agent {
    static POOL: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    pooled(&POOL, 30)
}

pub struct Signer {
    address: Address,
    key: ed25519_dalek::SigningKey,
    /// What every URL it's given starts with: the bucket's own, `<endpoint>/<bucket>`. A client device knows only the
    /// signer's address, and what it reads is sealed and signed, so it takes the URLs as they come.
    base: Option<String>,
    read_ahead: u64,
    /// URLs by `<kind> <name>` (read, head, create), with until when (ms) they work.
    kept: Mutex<HashMap<String, (String, u64)>>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the URL signer, for {}", self.base.as_deref().unwrap_or("a client device"))
    }
}

/// The URL signer's failures since `take_failures` last read them.
static FAILURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The URL signer's failures since this last asked, for insights.
pub fn take_failures() -> u64 {
    FAILURES.swap(0, std::sync::atomic::Ordering::Relaxed)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

impl Signer {
    pub fn new(address: Address, key: ed25519_dalek::SigningKey, base: String) -> Signer {
        Signer { address, key, base: Some(base.trim_end_matches('/').to_string()), read_ahead: READ_AHEAD, kept: Default::default() }
    }

    /// A client device's: it only reads.
    pub fn reading(address: String, key: ed25519_dalek::SigningKey) -> Signer {
        Signer { address: Arc::new(move || Ok(address.clone())), key, base: None, read_ahead: CLIENT_READ_AHEAD, kept: Default::default() }
    }

    /// Asks the signer: `fields` signed, with what this build reads of who's who, which the signer
    /// checks (`database::READS`), → its answer.
    pub fn ask(&self, mut fields: Value) -> Result<Value> {
        let address = (self.address)()?;
        fields["reads"] = serde_json::json!(crate::database::READS);
        let body = crate::keys::signed_request(&self.key, crate::protocol::URLS, fields, now_ms());
        let mut resp = signer_requests().post(format!("{}/urls", address.trim_end_matches('/'))).send_json(body).map_err(|e| {
            FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Failed(Trouble::Unreachable, format!("couldn't be reached ({address}: {e})"))
        })?;
        let status = resp.status().as_u16();
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        let said = out["error"].as_str().unwrap_or("nothing").to_string();
        if status != 200 {
            FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
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
        if let Some(url) = self.kept(kind, ask.name) {
            return Ok(url);
        }
        let id = format!("{kind} {}", ask.name);
        // A database's step or the signature beside one: the positions after it too, which come next, and the
        // signature beside each.
        let ahead = if kind == "create" { CREATE_AHEAD } else { self.read_ahead };
        let items: Vec<Value> = match crate::database::stored::position(ask.name) {
            Some((prefix, at)) => {
                let sigs = (at..=at + ahead).filter(|_| crate::database::stored::signed_beside(prefix)).map(|n| json!(crate::database::stored::sig_name(prefix, n)));
                // A signature asked of a database that has none beside its steps is asked as itself, so it's answered (none).
                let asked = ask.name.ends_with(".sig") && !crate::database::stored::signed_beside(prefix);
                std::iter::once(json!({ "steps": prefix, "from": at, "to": at + ahead })).chain(sigs).chain(asked.then(|| json!(ask.name))).collect()
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
        kept.get(&id).map(|(url, _)| url.clone()).context("the URL signer didn't sign that, for the network's bucket")
    }

    /// A read URL kept, with a minute to spare.
    fn kept(&self, kind: &str, name: &str) -> Option<String> {
        self.kept.lock().unwrap().get(&format!("{kind} {name}")).filter(|(_, until)| *until > now_ms() + 60_000).map(|(url, _)| url.clone())
    }

    /// The URL to read `name`, and those just signed to read the positions after it, if it's a database's step:
    /// `(name's, [(a later name, its URL)])`.
    pub fn reads_from(&self, name: &str) -> Result<(String, Vec<(String, String)>)> {
        let url = self.sign(&Ask { method: "GET", name, query: &[], headers: &[] })?;
        let after = crate::database::stored::position(name).filter(|_| !name.ends_with(".sig")).into_iter().flat_map(|(prefix, at)| (at + 1..=at + self.read_ahead).map(move |n| crate::database::stored::step_name(prefix, n)));
        Ok((url, after.filter_map(|n| Some((n.clone(), self.kept("read", &n)?))).collect()))
    }

    /// One URL of a kind that's asked for each time.
    fn only(&self, kind: &str, fields: Value, name: &str) -> Result<String> {
        let given = self.ask(fields)?;
        given[kind][name].as_str().filter(|u| self.ours(u)).map(str::to_string).context("the URL signer didn't sign that, for the network's bucket")
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
        given["list"][0].as_str().filter(|u| self.ours(u)).map(str::to_string).context("the URL signer didn't sign that, for the network's bucket")
    }

    /// Whether a URL is the network's bucket's, so a signer elsewhere can't send this device anywhere else.
    fn ours(&self, url: &str) -> bool {
        self.base.as_ref().is_none_or(|base| url.strip_prefix(base.as_str()).is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?')))
    }
}
