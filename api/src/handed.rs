//! The keys a caller hands an app runtime for one call: which a kind of request may be handed, sealed to the app runtime that runs it,
//! and what an app runtime that lacks one asks for.

use anyhow::{Context, Result, anyhow};
use data_encoding::{BASE64URL_NOPAD as B64U, HEXLOWER};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use key::*;

/// A database key a caller hands an app runtime for one call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handed {
    pub scope: String,
    pub epoch: u32,
    pub app: String,
    pub database: String,
    #[serde(with = "key::b64key")]
    pub key: [u8; 32],
}

/// What a caller hands an app runtime, sealed to it: the database keys a call needs, for calls with one access token (`for`,
/// its sha256 in hex) or from one client device over iroh (`for`, its key in hex), until `until` (milliseconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    #[serde(rename = "for")]
    pub for_: String,
    pub until: u64,
    pub keys: Vec<Handed>,
}

/// A handoff sealed to the app runtime's seal key, as base64url: the `x-one-keys` header's value.
pub fn seal_handoff(to: &[u8; 32], handoff: &Handoff) -> Result<String> {
    Ok(B64U.encode(&seal_to(to, &serde_json::to_vec(handoff)?)?))
}

/// Opens a handoff sealed to this app runtime, with its seal secret.
pub fn open_handoff(secret: &[u8; 32], sealed: &str) -> Result<Handoff> {
    let bytes = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("the keys handed aren't base64url"))?;
    let plain = open_sealed(secret, &bytes).context("the keys handed weren't sealed to this device")?;
    serde_json::from_slice(&plain).context("the keys handed aren't a handoff")
}

/// What an app runtime (oned on Cloudflare, or a machine's passing on a call it was handed keys
/// for) asks another app runtime to run a call with (`app-runtime/src/net.rs`): the call, naming `scopes`, the scopes it reaches,
/// the user it's for, and the keys of those scopes it was handed, sealed to that app runtime's `seal` key for calls from
/// its own endpoint (`from`), until `until`. Never keys of a scope the call doesn't name. The other app runtime runs it on those keys alone, and only if `from` may run
/// every one of `scopes`.
pub fn forwarded(call: &Value, user: &str, client: &str, from: &[u8; 32], seal: &[u8; 32], until: u64, keys: Vec<Handed>, scopes: &[String]) -> Result<Value> {
    let mut call = call.as_object().cloned().unwrap_or_default();
    for k in ["me", "keys"] {
        call.remove(k);
    }
    call.insert("me".into(), json!({ "user": user, "client": client }));
    call.insert("scopes".into(), json!(scopes));
    let keys: Vec<Handed> = keys.into_iter().filter(|k| scopes.contains(&k.scope)).collect();
    call.insert("keys".into(), seal_handoff(seal, &Handoff { for_: HEXLOWER.encode(from), until, keys })?.into());
    Ok(json!({ "kind": custom_build::protocol::message::ASK, "body": { "call": call } }))
}

/// Keys a call needs that it wasn't handed: the API answers 428 with them, and with this device's key and the key to
/// seal them to.
#[derive(Debug, Clone, schemars::JsonSchema)]
pub struct Needs {
    pub need: Vec<Need>,
    pub to: String,
    pub seal: String,
}

/// A key a call lacked, which its caller hands it when it sends the call again: what an app runtime answers with 428.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Need {
    pub scope: String,
    pub epoch: u32,
    pub app: String,
    pub database: String,
}

/// What a request runs, which decides what a client hands an app runtime with it (`may_hand`), and what it remembers each kind of
/// request needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// An app's call: `/api/call` for it, or its own request (`/<app>/…`).
    Call(String),
    /// An app's screens.
    Screens(String),
    /// Any other path.
    Path(String),
}

impl Route {
    /// The route of a request for `path` (a query is left off); `call_app` names the app of a `/api/call`, from its body.
    pub fn of(path: &str, call_app: impl FnOnce() -> String) -> Route {
        use crate::path;
        let at = path.split('?').next().unwrap_or(path);
        if let Some(app) = path::app_request(at) {
            return Route::Call(app.to_string());
        }
        match at.strip_prefix(path::SCREENS).and_then(|s| s.split('/').next()) {
            Some(app) => Route::Screens(app.to_string()),
            None if at == path::CALL => Route::Call(call_app()),
            None => Route::Path(at.to_string()),
        }
    }

    /// The app it runs, for a call or its screens.
    fn app(&self) -> Option<&str> {
        match self {
            Route::Call(app) | Route::Screens(app) => Some(app).filter(|a| !a.is_empty()).map(String::as_str),
            Route::Path(_) => None,
        }
    }
}

impl std::fmt::Display for Route {
    /// What a client remembers a kind of request by.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Call(app) => write!(f, "call {app}"),
            Route::Screens(app) => write!(f, "{}{app}", crate::path::SCREENS),
            Route::Path(path) => f.write_str(path),
        }
    }
}

/// Whether a client hands an app runtime the key of `need`'s database with a kind of request (`route`), when the app runtime asks (428):
/// only what that kind of request can use. One that runs an app, that app's databases; an
/// MCP request (`/mcp`), which may run any, or widgets (and the tool that shows them) and badges, which run every app's, apps' own databases; and each
/// of those the app runtime's Vault and log of a scope too (which fill in its items, and say who's who there), and `global`'s
/// blobs, where apps' deployed code is (an app runtime that keeps nothing, oned on Cloudflare, reads it there). The list of
/// apps, which says what each does from its code, the log and that code. The Vault's routes, its Vault and log; any
/// other, the log alone.
///
/// AI Gateway's rows of a scope go with whatever may call a model: an app's request, MCP, and its own tools' routes.
///
/// `global`'s blobs are those its entries name (deploys' bundles, releases, apps' icons), which everyone in the network
/// may read already: any of them may add any app, and read its code. Another scope's are its own users' files, and
/// never go with a request.
pub fn may_hand(route: &Route, need: &Need) -> bool {
    use ::key::CoreDb::{self, AiGateway as AI, Blobs as BLOBS, Log as LOG, Vault as VAULT};
    use crate::path;
    let node = |of: &[CoreDb]| CoreDb::of(&need.app, &need.database).is_some_and(|c| of.contains(&c));
    let apps = need.app != ::key::APP && need.database == ::key::APP_DATABASE;
    let code = need.scope == "global" && node(&[BLOBS]);
    match (route.app(), route) {
        (Some(app), _) => need.app == app || node(&[VAULT, LOG, AI]) || code,
        (None, Route::Path(p)) if p == path::MCP => apps || node(&[VAULT, LOG, AI]) || code,
        (None, Route::Path(p)) if matches!(p.as_str(), path::BADGES) || p == path::WIDGETS || p.starts_with(&format!("{}/", path::WIDGETS)) => apps || node(&[VAULT, LOG]) || code,
        (None, Route::Path(p)) if p.starts_with(&format!("{}/", path::AI_GATEWAY)) => node(&[VAULT, LOG, AI]),
        (None, Route::Path(p)) if p == path::APPS || p == path::OPENAPI => node(&[LOG]) || code,
        (None, Route::Path(p)) if p == path::VAULT || p.starts_with(&format!("{}/", path::VAULT)) => node(&[VAULT, LOG]),
        (None, _) => node(&[LOG]),
    }
}

/// What `may_hand` lets a client hand with `route`, as the OpenAPI document says it: an app's own database (`app`, any
/// app's on what draws several), the Vault's, the log's, and the code apps run from, in `global`'s blobs.
pub fn hands(route: &Route) -> Vec<&'static str> {
    use ::key::{APP, APP_DATABASE, CoreDb};
    let need = |scope: &str, app: &str, database: &str| Need { scope: scope.into(), epoch: 1, app: app.into(), database: database.into() };
    let kinds = [("app", need("user:x", route.app().unwrap_or("x"), APP_DATABASE)), ("vault", need("user:x", APP, CoreDb::Vault.name())), ("log", need("user:x", APP, CoreDb::Log.name())), ("code", need("global", APP, CoreDb::Blobs.name()))];
    kinds.into_iter().filter(|(_, need)| may_hand(route, need)).map(|(kind, _)| kind).collect()
}

/// The oldest epoch of `database`'s key a reader needs, of a scope whose newest epoch is `newest`: the one its newest
/// copy is under, as its newest hint (`hint`, if it has one) says, which holds all before it; every epoch from the first
/// while it has no copy; the newest if it has no hint, being new. Blobs have neither: each is kept under the epoch it
/// was put in, so every epoch from the first. The log, a scope's steps, needs every epoch from the first too: every
/// reader replays it from the first (`steps::Truth::read`), whatever copies there are. App runtimes ask for no older, and
/// clients hand no older.
pub fn oldest_needed(database: &str, hint: Option<&[u8]>, newest: u32) -> u32 {
    use ::key::CoreDb;
    if [CoreDb::Blobs, CoreDb::Log].map(CoreDb::name).contains(&database) {
        return 1;
    }
    let newest = newest.max(1);
    let Some(hint) = hint else { return newest };
    let v: Value = serde_json::from_slice(hint).unwrap_or_default();
    match (v["copy"].as_u64().unwrap_or(0), v["epoch"].as_u64().unwrap_or(0)) {
        (0, _) => 1,
        (_, epoch) => u32::try_from(epoch).unwrap_or(u32::MAX).clamp(1, newest),
    }
}

/// The most databases an app runtime's 428 may ask one request to hand keys of: a request that reads every app on a page
/// (`/api/badges`, `/api/widgets`, `/mcp`) needs each one's. More is an app runtime asking for everything. Each comes in
/// every epoch from the oldest it needs, which isn't counted: a scope may have had dozens (a device taken out makes one).
pub const MOST_NEEDED: usize = 64;

/// What an app runtime's 428 (`answer`) asks a request of `route` to hand: taken only from the app runtime that answered (`node`, hex,
/// as its connection proved), naming itself; one or more keys, of MOST_NEEDED databases at most, each a database that kind
/// of request uses (`may_hand`). None if it asks for anything else. The seal key it names isn't read: what's handed is
/// sealed to the one the network's entries list for that app runtime.
pub fn asked_for(route: &Route, node: &str, answer: &Value) -> Option<Vec<Need>> {
    if answer["to"].as_str() != Some(node) {
        return None;
    }
    let mut need: Vec<Need> = serde_json::from_value(answer["need"].clone()).ok()?;
    need.sort();
    need.dedup();
    let databases: std::collections::BTreeSet<(&str, &str, &str)> = need.iter().map(|n| (n.scope.as_str(), n.app.as_str(), n.database.as_str())).collect();
    (!need.is_empty() && databases.len() <= MOST_NEEDED && need.iter().all(|n| may_hand(route, n))).then_some(need)
}

/// How long a client waits, each time, to ask again an app runtime that doesn't list it yet (403 `not_let_in`) while the relay
/// still lets it in: an app runtime may not have read its approval yet. After the last, it says so.
pub const BEHIND: [u64; 2] = [2_000, 4_000];

/// What sign-in said of a device's request to be let in (`GET /api/auth/devices/requests/<key>`: its status, 0 if it
/// didn't answer, and its JSON), which every client acts on the same way (`key/crypto.js`'s `waited`).
#[derive(Debug, PartialEq)]
pub enum Waited {
    /// Its sign-in is no more: it signs in again.
    SignedOut,
    /// Approved, in the network whose root (hex) the device that approved it named, if it named one: the only
    /// one it then takes; and once who's who is written as steps, where that device read them to, which this one
    /// starts from (`database::steps::Welcome`), if it said. It's in once the relay lets it.
    Approved { root: Option<String>, welcome: Option<String> },
    /// Gone from auth (approved and cleared, turned down, or waited too long): it's in if the relay lets it, else it
    /// asks again.
    Gone,
    /// Still waiting, or auth didn't answer.
    Waiting,
}

pub fn waited(status: u16, said: &Value) -> Waited {
    match status {
        401 => Waited::SignedOut,
        404 => Waited::Gone,
        200..=299 if said["approved"] == true => Waited::Approved {
            root: said["root"].as_str().filter(|g| g.len() == 64 && g.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).map(str::to_string),
            welcome: said["welcome"].as_str().filter(|w| scope::steps::Welcome::parse(w).is_some()).map(str::to_string),
        },
        _ => Waited::Waiting,
    }
}

impl std::fmt::Display for Needs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list: Vec<String> = self.need.iter().map(|n| format!("{}'s {}/{} in epoch {}", n.scope, n.app, n.database, n.epoch)).collect();
        write!(f, "it needs keys it wasn't handed: {}", list.join(", "))
    }
}

impl std::error::Error for Needs {}

impl Needs {
    /// As the API says it: `{ error, need, to, seal }`.
    pub fn json(&self) -> Value {
        json!({ "error": self.to_string(), "need": self.need, "to": self.to, "seal": self.seal })
    }

    /// What `json` says, if it says keys are needed.
    pub fn from_json(v: &Value) -> Option<Needs> {
        let need: Vec<Need> = serde_json::from_value(v["need"].clone()).ok().filter(|n: &Vec<Need>| !n.is_empty())?;
        let said = |k: &str| v[k].as_str().unwrap_or("").to_string();
        Some(Needs { need, to: said("to"), seal: said("seal") })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call(app: &str) -> Route {
        Route::Call(app.into())
    }

    fn at(path: &str) -> Route {
        Route::Path(path.into())
    }

    #[test]
    fn a_node_is_handed_only_the_keys_a_kind_of_request_uses() {
        let need = |app: &str, database: &str| Need { scope: "global".into(), epoch: 1, app: app.into(), database: database.into() };
        assert!(may_hand(&call("tally"), &need("tally", "main")) && !may_hand(&call("tally"), &need("notes", "main")));
        assert!(may_hand(&at("/api/steps"), &need("node", "log")) && !may_hand(&at("/api/steps"), &need("tally", "main")) && !may_hand(&at("/api/state"), &need("node", "vault")));
        assert!(may_hand(&at("/api/vault"), &need("node", "vault")) && !may_hand(&at("/api/vault"), &need("tally", "main")));
        // Apps' code, `global`'s blobs, goes to an app runtime running an app (oned on Cloudflare keeps none); a scope's own
        // blobs, its users' files, never.
        let blobs = |scope: &str| Need { scope: scope.into(), epoch: 1, app: "node".into(), database: "blobs".into() };
        assert!(may_hand(&call("tally"), &blobs("global")) && may_hand(&Route::Screens("tally".into()), &blobs("global")) && may_hand(&at("/api/badges"), &blobs("global")) && may_hand(&at("/api/widgets/show"), &blobs("global")) && may_hand(&at("/api/apps"), &blobs("global")) && may_hand(&at(crate::path::OPENAPI), &blobs("global")));
        assert!(!may_hand(&at("/api/apps"), &need("tally", "main")) && !may_hand(&at("/api/apps"), &blobs("user:ab")));
        assert!(!may_hand(&call("tally"), &blobs("user:ab")) && !may_hand(&at("/mcp"), &blobs("group:cd")) && !may_hand(&at("/api/state"), &blobs("global")));
        assert_eq!(oldest_needed("main", Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 2, "from the epoch its newest copy is under");
        assert_eq!(oldest_needed("log", Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 1, "a scope's steps are read from the first");
        assert_eq!(oldest_needed("log", None, 3), 1, "a scope's steps are read from the first, before any hint");
        assert_eq!(oldest_needed("blobs", None, 3), 1, "a blob is kept under the epoch it was put in");
    }

    #[test]
    fn a_nodes_428_asks_only_for_what_the_request_uses_from_the_node_that_answered() {
        let node = "ab".repeat(32);
        let need = |app: &str, database: &str| json!({ "scope": "global", "epoch": 1, "app": app, "database": database });
        let every = |n: usize| (0..n).map(|i| need(&format!("app{i}"), "main")).collect::<Vec<_>>();
        let twice = json!({ "to": node, "need": [need("tally", "main"), need("tally", "main"), need("node", "log")] });
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [need("tally", "main")] })).is_some());
        assert!(asked_for(&call("tally"), &node, &twice).is_some_and(|n| n.len() == 2), "a key asked for twice is handed once");
        assert!(asked_for(&call("tally"), &node, &json!({ "to": "cd".repeat(32), "need": [need("tally", "main")] })).is_none(), "only the node that answered");
        assert!(asked_for(&call("tally"), &node, &json!({ "need": [need("tally", "main")] })).is_none(), "naming itself");
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [] })).is_none() && asked_for(&call("tally"), &node, &json!({ "to": node, "need": null })).is_none());
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [need("notes", "main")] })).is_none(), "another app's");
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [need("tally", "main"), { "scope": "global", "epoch": "1", "app": "tally", "database": "main" }] })).is_none());
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [{ "scope": "global", "epoch": -1, "app": "tally", "database": "main" }] })).is_none());
        assert!(asked_for(&at("/api/badges"), &node, &json!({ "to": node, "need": every(MOST_NEEDED) })).is_some());
        assert!(asked_for(&at("/api/badges"), &node, &json!({ "to": node, "need": every(MOST_NEEDED + 1) })).is_none());
        let epochs = |n: u32| (1..=n).map(|epoch| json!({ "scope": "global", "epoch": epoch, "app": "tally", "database": "main" })).collect::<Vec<_>>();
        assert!(asked_for(&at("/api/widgets"), &node, &json!({ "to": node, "need": epochs(MOST_NEEDED as u32 + 1) })).is_some(), "a scope's epochs aren't counted against it");
    }

    #[test]
    fn a_handoff_opens_only_for_the_node_it_was_sealed_to() {
        let node = SigningKey::from_bytes(&rand::random());
        let other = SigningKey::from_bytes(&rand::random());
        let handoff = Handoff { for_: "t".into(), until: 5, keys: vec![Handed { scope: "global".into(), epoch: 1, app: "tally".into(), database: "main".into(), key: rand::random() }] };
        let sealed = seal_handoff(&Endpoint::of(&node).seal, &handoff).unwrap();
        assert_eq!(open_handoff(&seal_secret(&node), &sealed).unwrap(), handoff);
        assert!(open_handoff(&seal_secret(&other), &sealed).is_err());
    }
}
