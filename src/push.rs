//! Push: how an app, an agent or the network reaches someone who isn't looking, on every device of theirs that can get
//! notifications. Every push is a note sealed to the one device it's for, which the network's push Worker
//! (`cloudflare/push/`) carries to Apple's, Google's, Microsoft's or a browser's push service; only that device opens it.
//!
//! A note is JSON (`Note`): what to show, the app screen a tap opens, and maybe an action its device checks itself,
//! signed by the device that sent it (`keys::signed_request`, domain `one/note`, so its receiver can tell which of the
//! network's devices it's from; a note the network's own Workers send is unsigned), then sealed to the receiving device's
//! seal key (`keys::seal_to`), the key the network's signed entries list for it, never one a node or a Worker says.
//!
//! Each device that gets pushes registers its address with the Worker itself (`register_request`). A node pushes for any
//! caller (`node::push`): it seals a note to each of the caller's devices and hands them to the Worker.

use anyhow::{Result, anyhow, ensure};
use data_encoding::{BASE64URL_NOPAD as B64U, HEXLOWER};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// What a device signs its requests to the push Worker for.
pub const DOMAIN: &str = "one/push";
/// What a device signs a note for.
pub const SIGNED_NOTE: &str = "one/note";
/// The most a sealed note may be: it fits in a push, 4 KB at Apple and in Web Push, with room to spare.
pub const NOTE_BYTES: usize = 2_600;

/// What a push shows, and what tapping it opens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    /// Its id (16 hex characters), so a device shows it once.
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// The app whose screen tapping it opens, that screen and its params.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub app: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub screen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Map<String, Value>>,
    /// The number on the app's icon; 0 clears it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<i64>,
    /// Something its device does, checking it first itself: `{ "join": "<code>" }`, a device asking to join, which the
    /// Apple apps show with Approve and Not Me (`client/native/iphone/Notifications/Notes.swift`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Value>,
}

impl Note {
    /// The code of the device its action asks to let in (`{ "join": "<code>" }`), if it does.
    pub fn join(&self) -> Option<&str> {
        self.action.as_ref()?["join"].as_str().filter(|c| !c.is_empty())
    }

    /// What tapping it opens in the app, the same on every platform: a join, Settings › Devices, where it waits too;
    /// Settings' own screens (`app` "settings"), the client's, not an app's; else the app screen it names, `main` if
    /// none. None opens only the app.
    pub fn link(&self) -> Option<String> {
        let cli = crate::config::CLI;
        let enc = crate::url_encode;
        if self.join().is_some() {
            return Some(format!("{cli}://settings/devices"));
        }
        if self.app.is_empty() {
            return None;
        }
        if self.app == "settings" {
            return Some(format!("{cli}://settings/{}", enc(&self.screen)));
        }
        let screen = if self.screen.is_empty() { "main" } else { &self.screen };
        let params = self.params.as_ref().filter(|p| !p.is_empty()).map(|p| format!("&params={}", enc(&Value::Object(p.clone()).to_string()))).unwrap_or_default();
        Some(format!("{cli}://open?app={}&screen={}{params}", enc(&self.app), enc(screen)))
    }
}

/// A note, signed by `by` at `at` (ms), sealed to a device's seal key `to`: the bytes a push carries. A body too long to
/// fit is cut short.
pub fn seal_note(by: &SigningKey, to: &[u8; 32], note: &Note, at: u64) -> Result<Vec<u8>> {
    let mut note = note.clone();
    loop {
        let signed = crate::keys::signed_request(by, SIGNED_NOTE, serde_json::to_value(&note)?, at);
        let sealed = crate::keys::seal_to(to, &serde_json::to_vec(&signed)?)?;
        if sealed.len() <= NOTE_BYTES {
            return Ok(sealed);
        }
        ensure!(!note.body.is_empty(), "that note is too long to push: at most {NOTE_BYTES} bytes sealed");
        let keep = note.body.len().saturating_sub(sealed.len() - NOTE_BYTES + 3);
        let cut = (0..=keep).rev().find(|i| note.body.is_char_boundary(*i)).unwrap_or(0);
        note.body = format!("{}…", note.body[..cut].trim_end());
        if cut == 0 {
            note.body.clear();
        }
    }
}

/// Opens a note sealed to the device whose seal secret is `secret`: → the note, and the key that signed it (hex), or
/// none for a note from the network's Workers. A signature that's wrong is refused.
pub fn open_note(secret: &[u8; 32], sealed: &[u8]) -> Result<(Note, Option<String>)> {
    let plain = crate::keys::open_sealed(secret, sealed)?;
    let mut v: Value = serde_json::from_slice(&plain).map_err(|_| anyhow!("not a note"))?;
    let by = match v.as_object_mut().and_then(|o| o.remove("sig")) {
        Some(sig) => {
            let by: [u8; 32] = HEXLOWER.decode(v["by"].as_str().unwrap_or("").as_bytes())?.try_into().map_err(|_| anyhow!("not a key"))?;
            crate::keys::verify(&by, SIGNED_NOTE, &v, &B64U.decode(sig.as_str().unwrap_or("").as_bytes())?)?;
            Some(HEXLOWER.encode(&by))
        }
        None => None,
    };
    Ok((serde_json::from_value(v)?, by))
}

/// A device's address with a push service, as it registers it with the push Worker.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Address {
    /// `apns`, `fcm`, `wns` or `webpush`.
    pub via: String,
    /// Its APNs token (hex), FCM token, WNS channel URI or Web Push endpoint.
    pub address: String,
    /// APNs: `sandbox` or `production`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub env: String,
    /// APNs: the app's bundle id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub topic: String,
    /// Web Push: the subscription's `{ p256dh, auth }`, base64url.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Value>,
}

/// What a device sends the push Worker to register `address` (`POST <address>/push/register`): signed with its key,
/// naming its seal key `seal`, which only the network's own Workers seal to, and its person `user`, as its own entries
/// list it, whom sign-in pushes about their devices joining.
pub fn register_request(key: &SigningKey, seal: &[u8; 32], address: &Address, user: Option<&str>, at: u64) -> Result<Value> {
    Ok(crate::keys::signed_request(key, DOMAIN, register_fields(seal, address, user)?, at))
}

/// What `register_request` signs.
pub fn register_fields(seal: &[u8; 32], address: &Address, user: Option<&str>) -> Result<Value> {
    let mut fields = serde_json::to_value(address)?;
    fields["seal"] = json!(HEXLOWER.encode(seal));
    if let Some(user) = user {
        fields["user"] = json!(user);
    }
    fields["route"] = json!("POST /push/register");
    Ok(fields)
}

/// What a device sends the push Worker to stop getting pushes (`POST <address>/push/unregister`).
pub fn unregister_request(key: &SigningKey, at: u64) -> Value {
    crate::keys::signed_request(key, DOMAIN, json!({ "route": "POST /push/unregister" }), at)
}

#[cfg(feature = "node")]
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Send {
    pub title: String,
    pub body: String,
    pub app: String,
    pub screen: String,
    pub params: Option<Map<String, Value>>,
    pub badge: Option<i64>,
}

#[cfg(feature = "node")]
impl Send {
    /// The note it makes, from app `from` if an app sent it: what a tap opens is that app's screen, or else the one
    /// the push names.
    pub fn note(&self, from: &str) -> Result<Note> {
        ensure!(!self.title.trim().is_empty(), "give it a title");
        let first = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { a.to_string() };
        let (app, params) = (first(from, &self.app), self.params.clone());
        let screen = if app.is_empty() { String::new() } else { first(&self.screen, "main") };
        let title: String = self.title.trim().chars().take(200).collect();
        let body: String = self.body.chars().take(2000).collect();
        Ok(Note { id: HEXLOWER.encode(&rand::random::<[u8; 8]>()), title, body, params: params.filter(|p| !p.is_empty() && !app.is_empty()), app, screen, badge: self.badge, action: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_opens_only_where_it_was_sealed() {
        let by = SigningKey::from_bytes(&rand::random());
        let (secret, public) = crate::keys::twin(&rand::random());
        let note = Note { id: "0123456789abcdef".into(), title: "Hi".into(), body: "there".into(), app: "tally".into(), screen: "main".into(), ..Default::default() };
        let sealed = seal_note(&by, &public, &note, 1).unwrap();
        let (opened, from) = open_note(&secret, &sealed).unwrap();
        assert_eq!((opened, from), (note.clone(), Some(HEXLOWER.encode(by.verifying_key().as_bytes()))));
        assert!(open_note(&crate::keys::twin(&rand::random()).0, &sealed).is_err(), "another device's key opens nothing");

        let long = Note { body: "é".repeat(5000), ..note };
        let sealed = seal_note(&by, &public, &long, 1).unwrap();
        assert!(sealed.len() <= NOTE_BYTES);
        let (opened, _) = open_note(&secret, &sealed).unwrap();
        assert!(opened.body.ends_with('…') && opened.body.len() < long.body.len());
    }

    #[test]
    fn a_tapped_note_opens_the_same_link_on_every_platform() {
        let cli = crate::config::CLI;
        let note = |app: &str, screen: &str, params: Option<Value>, action: Option<Value>| Note { app: app.into(), screen: screen.into(), params: params.and_then(|p| p.as_object().cloned()), action, ..Default::default() };
        assert_eq!(note("tally", "", None, None).link(), Some(format!("{cli}://open?app=tally&screen=main")));
        assert_eq!(note("tally", "day", Some(json!({ "d": "a b" })), None).link(), Some(format!("{cli}://open?app=tally&screen=day&params=%7B%22d%22%3A%22a%20b%22%7D")));
        assert_eq!(note("settings", "devices", None, None).link(), Some(format!("{cli}://settings/devices")));
        assert_eq!(note("", "", None, Some(json!({ "join": "AB12" }))).link(), Some(format!("{cli}://settings/devices")));
        assert_eq!(note("", "", None, None).link(), None);
    }
}
