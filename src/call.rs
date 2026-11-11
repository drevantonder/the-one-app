//! What a running call's app (`ctx`, `src/app/runtime/shim.js`) and the node that runs it say to each other, on a machine
//! and on the Cloudflare node alike: what the app asks the node while it runs, and what it hands back to write when it
//! returns. The shim and the Cloudflare node's Worker read this from what `javascript` writes, so nothing is copied by hand;
//! `tests/contracts.rs` holds the shim's asks and the Cloudflare node's answers to it.

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

/// What an app asks the node for the call it runs in, by name: on a machine `POST one.internal/<name>`
/// (`src/app/outbound.rs`), on the Cloudflare node `node.ask(name, body)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Runs a tool of an app it uses, or a function of its own service (`ctx.apps`, `ctx.services`).
    Call,
    /// Reads a scope's database (`ctx.<scope>.query`).
    Query,
    /// Whether the caller may do each thing asked (`ctx.<scope>.may`).
    May,
    /// The Vault's items switched on for the app (`ctx.vault.items`).
    Vault,
    /// The item a sign-in would change rather than make (`ctx.vault.create` with `same`).
    VaultMatch,
    /// Keeps a blob (`ctx.blob.put`).
    BlobPut,
    /// Reads a blob (`ctx.blob.get`).
    BlobGet,
    /// What work a call left running (`ctx.waitUntil`) wrote, once it's done.
    Appends,
    /// That work's end.
    Done,
    /// The network's apps (`ctx.catalog`).
    Catalog,
    /// An app's code (`ctx.code`).
    Code,
    /// The caller's failing schedules (`ctx.failing`).
    Failing,
    /// A push to the caller's devices (`ctx.notify`).
    Notify,
    /// The network's devices (`ctx.devices`).
    Devices,
    /// Run priority's order (`ctx.order`).
    Order,
    /// Changes a device's run priority (`ctx.order` with a device).
    Priority,
    /// An app's request out, on the Cloudflare node, which has no outbound to give an app: a machine's node takes each
    /// as it's made (`src/app/outbound.rs`).
    Fetch,
    /// Asks an AI model, through AI Gateway (`ctx.ai.run`): it writes only the caller's own `node/ai-gateway`.
    AiRun,
}

/// Each ask: its name, whether a call that only reads (a widget's, badge's or hook's) is refused it, and whether the
/// Cloudflare node answers it yet (an app that asks one there is told to use a machine).
const ASKS: [(Ask, &str, bool, bool); 18] = [
    (Ask::Call, "call", true, true),
    (Ask::Query, "query", false, true),
    (Ask::May, "may", false, true),
    (Ask::Vault, "vault", false, true),
    (Ask::VaultMatch, "vault.match", false, false),
    (Ask::BlobPut, "blob.put", false, false),
    (Ask::BlobGet, "blob.get", false, false),
    (Ask::Appends, "appends", false, true),
    (Ask::Done, "done", false, true),
    (Ask::Catalog, "catalog", false, true),
    (Ask::Code, "code", false, false),
    (Ask::Failing, "failing", false, false),
    (Ask::Notify, "notify", true, false),
    (Ask::Devices, "devices", false, true),
    (Ask::Order, "order", false, false),
    (Ask::Priority, "priority", true, false),
    (Ask::Fetch, "fetch", false, true),
    (Ask::AiRun, "ai.run", false, false),
];

/// What a call that only reads is told when it writes, asks something that does, pushes or calls another app.
pub const READ_ONLY: &str = "this call only reads (a widget's, badge's or hook's): it can't write, push, call other apps or change settings";

impl Ask {
    pub fn parse(name: &str) -> Option<Ask> {
        ASKS.iter().find(|a| a.1 == name).map(|a| a.0)
    }

    fn row(self) -> &'static (Ask, &'static str, bool, bool) {
        ASKS.iter().find(|a| a.0 == self).expect("every ask is in ASKS")
    }

    pub fn name(self) -> &'static str {
        self.row().1
    }

    /// Whether a call that only reads is refused it.
    pub fn writes(self) -> bool {
        self.row().2
    }

    /// Every ask's name, and those the Cloudflare node answers.
    pub fn names() -> impl Iterator<Item = (&'static str, bool)> {
        ASKS.iter().map(|a| (a.1, a.3))
    }
}

/// What a call asks the node to write once it returns, as the shim hands it: `{ kind, … }`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Append {
    /// A statement that writes the app's database of a scope, which keeps the blobs it names; or with `check`, a query
    /// whose first value must be true as the write runs, else the call fails with that message.
    Exec {
        scope: String,
        sql: String,
        #[serde(default)]
        params: Vec<Value>,
        #[serde(default)]
        blobs: Vec<String>,
        #[serde(default)]
        check: Option<String>,
    },
    /// Adds (or removes) an app in a scope; added, with its service run on `node` (a device's name, or `cloudflare`).
    Add { app: String, scope: String, added: bool, node: Option<String> },
    /// Switches one of an app's schedules on or off in a scope, with the caller's own rights there.
    Switch { app: String, schedule: String, scope: String, on: bool },
    /// Says a scope no longer needs a blob it kept: once no scope does, devices delete it.
    Forget { scope: String, blob: String },
    /// Deletes an app (`deleted`), restores it (`restored`) or erases it (`erased`), with the caller's own rights.
    Retire { app: String, how: String },
    /// Sets (or with an empty link, clears) where an app's code is kept, with the caller's own rights.
    Source { app: String, link: String },
    /// Runs an app's service on a device (or stops it there), for the app added in `scope` (by default where the caller
    /// sees it added): for the device's user, or to stop it, for who may add apps where it ran.
    Service { name: String, device: String, on: bool, scope: Option<String> },
    /// Makes, changes or lets go of (`release`) a Vault item: `scope` for one it makes, `same` a plain field by
    /// which it changes an item of that type instead.
    Vault {
        id: String,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        item: Value,
        #[serde(default)]
        same: Option<String>,
        #[serde(default)]
        release: bool,
    },
}

pub fn appends(list: &Value) -> Result<Vec<Append>> {
    Ok(serde_json::from_value(if list.is_null() { json!([]) } else { list.clone() })?)
}

/// The constants above, as the shim and the Cloudflare node's Worker read them.
pub fn javascript() -> String {
    let asks: serde_json::Map<String, Value> = Ask::names().map(|(name, cloudflare)| (name.to_string(), json!({ "writes": Ask::parse(name).is_some_and(Ask::writes), "cloudflare": cloudflare }))).collect();
    format!("export const ASKS = {};\nexport const READ_ONLY = {};\n", Value::Object(asks), json!(READ_ONLY))
}

/// The shim every app runs inside, with what it reads of this module before it.
pub fn shim() -> String {
    format!(
        "// What src/call.rs says a call's app and its node say to each other, written when the shim is: never edit it.\n{}const CODE_HEADER = {};\n\n{}",
        javascript().replace("export const", "const"),
        json!(crate::protocol::header::CODE),
        include_str!("app/runtime/shim.js")
    )
}
