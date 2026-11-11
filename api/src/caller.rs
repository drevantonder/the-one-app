//! Who asks: a caller, and what they came with, which says whose keys their calls open data with.

use key::Key;
use scope::scopes::Scope;

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

/// What a caller came with: this device's own keys, or the keys it was handed for one of these, kept by its runtime
/// and never this device's own. A sign-in is none of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    /// None: this device's own programs, or a device or service of its own user asking as itself. Its calls use this
    /// device's own keys, for its own user alone.
    Own,
    /// A client device's calls over iroh, by its key, proven by QUIC: never a token anyone sends, so no request reaches
    /// what a device was handed by naming it.
    Client(Key),
    /// A call another device (`node`, its endpoint key, proven by QUIC) passed on for `user`: it runs only on the keys
    /// it was handed, sealed here for that device's key.
    Passed { node: Key, user: String },
    /// App schedules' calls: `app`'s in `scope`, with the keys sealed to this device for them alone. Never passed on.
    AppSchedules { scope: Scope, app: String },
}

impl Caller {
    pub fn own(key: Key, user: impl Into<String>) -> Caller {
        Caller { key, user: user.into(), agent: None, credential: Credential::Own }
    }

    /// Crediting `agent`, when a request names one, else the agent it came with.
    pub fn for_agent(self, agent: Option<&str>) -> Caller {
        Caller { agent: agent.map(str::to_string).or(self.agent), ..self }
    }
}

/// Who asked a route: this machine's own user, or a client device of the network, whom this device acts for. Only the
/// guard puts one on a request; with none, it's this machine's own programs, as the runtime's own caller.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone)]
pub struct Me {
    pub caller: Caller,
    /// A client device's name; none for this machine's own programs.
    pub client: Option<String>,
}

#[cfg(not(target_family = "wasm"))]
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Me
where
    crate::runtime::Rt: axum::extract::FromRef<S>,
{
    type Rejection = crate::reply::Error;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, state: &S) -> Result<Me, crate::reply::Error> {
        match parts.extensions.get::<Me>() {
            Some(me) => Ok(me.clone()),
            None => Ok(Me { caller: <crate::runtime::Rt as axum::extract::FromRef<S>>::from_ref(state).caller()?, client: None }),
        }
    }
}
