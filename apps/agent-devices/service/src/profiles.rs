//! Browser profiles. A profile is a Vault item of the user's: its profile folder, as a tarball without caches, is a
//! blob encrypted with a key made from the item's secret field and a salt of its own, so each save has a fresh key.
//! The app over this service names the item in its calls, and the node fills in the secret, so only this process ever
//! holds it or the profile; the app records each save on the item (its blob, salt and sites), and nothing here keeps a
//! profile once its device is gone.

use std::io::Read;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Result, anyhow, bail};
use one_service::refuse;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::linux::{Device, tarred};
use super::pool::{PROFILES, excludes, owned};
use super::{Devices, Input};

/// What a call says of the browser profile a device starts with.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub item: String,
    /// Its title, the profile's folder in the device.
    #[serde(default)]
    pub name: String,
    /// Its secret field, filled in by the node.
    #[serde(default)]
    pub key: String,
    /// Its last save; none for a new one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blob: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub salt: String,
}

pub fn profile_in(input: &Input) -> Result<Option<Profile>> {
    let Some(raw) = input.get("profile").filter(|p| !p.is_null()) else { return Ok(None) };
    let mut p: Profile = serde_json::from_value(strip_nulls(raw)).ok().filter(|p: &Profile| !p.item.is_empty()).ok_or_else(|| anyhow!("profile is a browser profile as the app gives it"))?;
    p.name = folder(&p.name);
    if p.key.contains("{{") || p.key.is_empty() {
        refuse!("the profile's key wasn't filled in: the app isn't switched on for it in the Vault");
    }
    Ok(Some(p))
}

/// An object without its null fields, as Go reads them: not given.
fn strip_nulls(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(o.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect()),
        v => v.clone(),
    }
}

/// Named as its item is, as far as the browser tools allow: letters, digits and dashes.
pub fn folder(title: &str) -> String {
    let b: Vec<u8> = title.trim().bytes().map(|c| if c.is_ascii_alphanumeric() || c == b'-' { c } else { b'-' }).collect();
    let name = String::from_utf8_lossy(&b[..b.len().min(40)]).trim_matches('-').to_string();
    if name.is_empty() { "profile".into() } else { name }
}

fn sealer(key: &str, salt: &str) -> Aes256Gcm {
    let mut k = [0u8; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(Some(salt.as_bytes()), key.as_bytes()).expand(b"browser profile", &mut k).expect("32 bytes is a valid length");
    Aes256Gcm::new(&k.into())
}

fn seal(key: &str, salt: &str, plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(sealer(key, salt).encrypt(&nonce.into(), plain).expect("encrypting in memory"));
    out
}

pub fn unseal(key: &str, salt: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < 12 {
        bail!("the saved profile is cut short");
    }
    sealer(key, salt).decrypt(sealed[..12].into(), &sealed[12..]).map_err(|_| anyhow!("the saved profile doesn't open with its key"))
}

/// What saving a profile made: its blob, salt and sites, for the app to record on its item.
#[derive(Debug, Serialize)]
pub struct Save {
    pub item: String,
    pub blob: String,
    pub salt: String,
    pub sites: String,
}

/// Reads a Chrome profile's History: each site's registrable domain (example.com, example.com.au) with the day it was
/// last used, newest first, at most 30. Python's sqlite3 is in every device, so this needs nothing new; a short list
/// of second-level labels stands in for the public suffix list.
const HISTORY: &str = r#"
import sqlite3, sys, datetime, shutil, tempfile
from urllib.parse import urlsplit
db = tempfile.mktemp()
shutil.copy(sys.argv[1], db)
second = {"com", "co", "org", "net", "gov", "edu", "ac", "id", "asn"}
last = {}
for url, t in sqlite3.connect(db).execute("select url, last_visit_time from urls where url like 'http%'"):
    host = (urlsplit(url).hostname or "").lower()
    labels = [l for l in host.split(".") if l]
    if len(labels) < 2 or host.replace(".", "").isdigit():
        continue
    n = 3 if len(labels) > 2 and len(labels[-1]) == 2 and labels[-2] in second else 2
    site = ".".join(labels[-n:])
    last[site] = max(last.get(site, 0), t)
day = lambda t: (datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=t)).strftime("%Y-%m-%d")
print(", ".join(f"{s} {day(t)}" for s, t in sorted(last.items(), key=lambda x: -x[1])[:30]))
"#;

impl Devices {
    pub(super) fn opened(&self, p: &Profile) -> Result<Vec<u8>> {
        let mut sealed = Vec::new();
        self.host.get_blob(&p.blob).and_then(|mut r| Ok(r.read_to_end(&mut sealed)?)).map_err(|err| anyhow!("fetching the {} profile: {err:#}", p.name))?;
        unseal(&p.key, &p.salt, &sealed)
    }

    pub(super) fn unpack(&self, d: &Device, p: &Profile) -> Result<()> {
        if p.blob.is_empty() {
            return self.sh(d, &owned(&p.name));
        }
        let plain = self.opened(p)?;
        let dir = format!("{PROFILES}{}", p.name);
        let script = format!(r#"rm -rf "$1" && mkdir -p "$1" && tar -C "$1" -xzf - && {}"#, owned(&p.name));
        self.exec(d, Some(&mut &plain[..]), None, &["sh", "-c", &script, "sh", &dir]).map_err(|err| anyhow!("unpacking the {} profile: {err:#}", p.name))
    }

    pub(super) fn pack(&self, d: &Device, p: &Profile) -> Result<Save> {
        if self.sh(d, &format!("test -d {PROFILES}{}", p.name)).is_err() {
            refuse!("device {}'s browser has no {} profile", d.name, p.name);
        }
        self.close_browsers(d, true);
        let dir = format!("{PROFILES}{}", p.name);
        let mut args = vec!["tar".to_string(), "-C".into(), dir, "--warning=no-file-changed".into()];
        args.extend(excludes(".", &[]));
        args.extend(["-czf".into(), "-".into(), ".".into()]);
        let mut tarball = Vec::new();
        let packed = self.exec(d, None, Some(&mut tarball), &args.iter().map(String::as_str).collect::<Vec<_>>());
        if !tarred(&packed) {
            packed?;
        }
        let salt = super::random_hex(16);
        let sealed = seal(&p.key, &salt, &tarball);
        let size = sealed.len();
        let (blob, _) = self.host.put_blob(std::io::Cursor::new(sealed))?;
        tracing::info!("saved the {} profile from {}: {size} bytes", p.name, d.name);
        Ok(Save { item: p.item.clone(), blob, salt, sites: self.sites(d, &p.name) })
    }

    pub(super) fn saved(&self, s: &Save) -> Result<()> {
        self.call_app("_saved", &serde_json::to_value(s)?).map_err(|err| anyhow!("recording the save: {err:#}"))?;
        Ok(())
    }

    fn sites(&self, d: &Device, name: &str) -> String {
        let file = format!("{PROFILES}{name}/Default/History");
        let mut out = Vec::new();
        match self.exec(d, Some(&mut HISTORY.as_bytes()), Some(&mut out), &["sh", "-c", r#"test -f "$1" && python3 - "$1" || true"#, "sh", &file]) {
            Ok(()) => String::from_utf8_lossy(&out).trim().to_string(),
            Err(err) => {
                tracing::warn!("reading the {name} profile's sites: {err:#}");
                String::new()
            }
        }
    }

    /// Closes the browsers the device's tools launched, and with sign_ins the one sign_in opened too, marking their
    /// profiles as closed normally, or they'd offer to restore their pages. One still closing after 10s is killed: a
    /// profile's copied once this returns, and a copy of files Chrome is writing can be broken where a killed Chrome's
    /// never is. With sign_ins nothing of any browser's is left running, its helpers included.
    pub(super) fn close_browsers(&self, d: &Device, sign_ins: bool) {
        let keep = if sign_ins { "--matches-no-browser" } else { "--class=one-sign-in" };
        let script = format!(
            r#"profiles={PROFILES} signIns={sign_ins}
main() {{ # each browser's own process, not its helpers
	for p in $(pgrep -f -- "[-]-user-data-dir=$profiles"); do
		c=$(tr '\0' ' ' 2>/dev/null </proc/$p/cmdline) || continue
		case "$c" in *--type=*|*{keep}*) ;; *) echo $p ;; esac
	done
}}
pids=$(main)
[ -n "$pids" ] && kill $pids
for i in $(seq 100); do [ -z "$(main)" ] && break; sleep 0.1; done
pids=$(main)
[ -n "$pids" ] && kill -9 $pids
if $signIns; then
	for i in $(seq 50); do pgrep -f -- "[-]-user-data-dir=$profiles" >/dev/null || break; sleep 0.1; done
	pkill -9 -f -- "[-]-user-data-dir=$profiles"
fi
sed -i 's/"exit_type":"Crashed"/"exit_type":"Normal"/' $profiles*/Default/Preferences 2>/dev/null
true"#
        );
        if let Err(err) = self.sh(d, &script) {
            tracing::warn!("closing {}'s browsers: {err:#}", d.name);
        }
    }
}

#[cfg(test)]
mod tests {
    /// A profile the Go service saved opens here: the same key from the item's secret and the save's salt.
    #[test]
    fn a_go_save_opens() {
        let sealed = data_encoding::HEXLOWER.decode(b"1ddbb5f54b9c374317f66861cf0507101d9bc1d035d9a1c42066999d00151df0c99b4df1baea845ce451eefa9b1587").unwrap();
        assert_eq!(super::unseal("the item's secret", "0011aabb", &sealed).unwrap(), b"a profile's tarball");
        let again = super::seal("k", "s", b"x");
        assert_eq!(super::unseal("k", "s", &again).unwrap(), b"x");
        assert!(super::unseal("k", "t", &again).is_err());
    }
}
