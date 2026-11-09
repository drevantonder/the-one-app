//! Who asks: a caller, and what they came with, which says whose keys their calls open data with.

use super::*;

/// Who asked for something: this device or another, or a client, as one user.
#[derive(Clone, Debug)]
pub struct Caller {
    pub key: Key,
    pub user: String,
    /// The agent that asked through it, by name ("Claude Code"), when one did.
    pub agent: Option<String>,
    /// What it came with, which says whose keys its calls open data with.
    pub credential: Credential,
}

/// What a caller came with: this device's own keys, or the keys it was handed for one of these, kept here by it
/// (sessions.rs) and never this device's own. A sign-in is none of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    /// None: this device's own programs, or a device or service of its own user asking as itself. Its calls use this
    /// device's own keys, for its own user alone.
    Own,
    /// A client device's calls over iroh, by its key, proven by QUIC: never a token anyone sends, so no request reaches
    /// what a device was handed by naming it.
    Client(Key),
    /// A call another node (`node`, its endpoint key, proven by QUIC) passed on for `user`: it runs only
    /// on the keys it was handed, sealed here for that node's key.
    Passed { node: Key, user: String },
    /// App schedules' calls: `app`'s in `scope`, with the keys sealed to this device for them alone. Never passed on.
    AppSchedules { scope: Scope, app: String },
}

impl Caller {
    pub fn own(key: Key, user: impl Into<String>) -> Caller {
        Caller { key, user: user.into(), agent: None, credential: Credential::Own }
    }

    pub fn local(node: &Node) -> Result<Caller> {
        Ok(Caller::own(node.0.id, node.user()?))
    }
}

impl Node {
    pub fn this_machine(&self, caller: &Caller) -> bool {
        caller.credential == Credential::Own && caller.key == self.0.id
    }
}
