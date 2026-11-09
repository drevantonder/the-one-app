//! Asking to be let in as a user's device, the same for every client (a native app's `Device`, a browser's
//! `Keyring` in its worker): what it asks auth with, and what auth says of its request while it waits, acted on. How
//! long it waits between asks, and what it shows meanwhile, are its own.

use one::keys::{self, Endpoint, Waited};
use serde_json::{Value, json};

use crate::Error;
use crate::keyring::{Address, Keep, error};

/// What a device asks auth to let it in with: its keys, what it's called (`name`) and the kinds of request it reads;
/// the device it's on, if its app says (`on`); what it's built for (`platform`); and for someone new, their invite's
/// proof.
pub fn asking(me: &Endpoint, name: &str, on: Option<&str>, platform: Option<&str>, invite: Option<&str>) -> Value {
    let mut asked = json!({ "key": keys::hex(&me.key), "seal": keys::hex(&me.seal), "name": name, "reads": one::database::READS });
    if let Some(on) = on {
        asked["device"] = json!(on);
    }
    if let Some(platform) = platform {
        asked["platform"] = json!(platform);
    }
    if let Some(invite) = invite.filter(|i| !i.is_empty()) {
        asked["proof"] = json!(keys::invite_proof(invite.as_bytes(), me));
    }
    asked
}

/// Asks auth, as the user signed in with `token`, to let it in with `asked`.
pub async fn ask(at: &impl Address, token: &str, asked: &Value) -> Result<(), Error> {
    let (status, out) = at.ask("POST", &format!("{}/api/auth/devices/requests", at.address()), Some(("authorization", format!("Bearer {token}"))), Some(asked.clone())).await?;
    if status != 200 {
        return Err(crate::unsent(error(&out).unwrap_or_else(|| format!("Couldn't ask to be let in ({status})."))));
    }
    Ok(())
}

/// What auth says now of the request of the device whose key is `key` (`keys::waited`): once approved, the network the
/// device that approved it said it's in (its root), the only one it then takes, is kept before it reads any, so none
/// it's shown first can be another; and where that device read who's who to (its welcome), which it starts from.
/// One gone from auth is in if the relay lets it, else asks again; a sign-in that's no more signs in again.
pub async fn waited(at: &(impl Address + Keep), token: &str, key: &[u8; 32]) -> Result<Waited, Error> {
    let url = format!("{}/api/auth/devices/requests/{}", at.address(), keys::hex(key));
    let (status, said) = at.ask("GET", &url, Some(("authorization", format!("Bearer {token}"))), None).await.unwrap_or((0, Value::Null));
    let waited = keys::waited(status, &said);
    if let Waited::Approved { root, welcome } = &waited {
        for (name, said) in [("root", root), ("welcome", welcome)] {
            if let Some(said) = said {
                at.put(name, said.as_bytes()).await.map_err(crate::unsent)?;
            }
        }
    }
    Ok(waited)
}
