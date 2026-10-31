//! Native apps: drivers built into the node, for what only native code can reach (a shell, files, local models).
//! They run on the server they live on (`runs: here`) and only for its local user; their entries go in that user's
//! scope. The node advertises them in sync like any driver; the core never names one.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

use crate::apps::{AppEntry, Me};
use crate::entry::Content;

pub type Reply = Pin<Box<dyn Future<Output = Result<(Value, Vec<Content>)>> + Send>>;

pub struct Native {
    pub name: &'static str,
    pub describe: fn() -> Value,
    /// Whether `call` needs the app's entries (reading them all costs a pass over the log).
    pub entries: bool,
    /// Runs an operation, given every entry the app has in the local user's scope, oldest first.
    pub call: fn(op: String, input: Value, caller: Me, entries: Vec<AppEntry>) -> Reply,
    /// Brings this machine in line with the app's entries, after they change and when the node starts.
    pub sync: Option<fn(entries: &[AppEntry])>,
}
