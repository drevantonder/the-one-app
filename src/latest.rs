//! The signed "latest" record: one small file, `<address>/updates/newest`, that says what's newest of everything a device
//! takes from `one-dev publish`, so a device takes its own updates with nothing else of the network working: not the protocol,
//! READS, the relay or a node. Each thing it names has its file beside it, `<address>/updates/<sha256>`, unsealed.
//!
//! THIS FORMAT MUST NEVER BREAK. A device too old to speak anything else still reads it, and only by reading it does it
//! get out of being too old; everything old in the field is on the other end of this. Readers ignore a line they don't
//! understand, so new kinds of things are new lines (`tests` holds the format; `client/native/mac/pull.sh` reads the same
//! lines, in its own language). The text:
//!
//! ```text
//! <name> <commit> <committed> <sha256> <size> <config>
//! …
//! -----BEGIN SSH SIGNATURE-----
//! …
//! -----END SSH SIGNATURE-----
//! ```
//!
//! `name` is `node/<target>`, `app/<target>` (a Mac's zip), `service/<name>/<target>`, `web` (the web client's signed
//! bundle), or `source` (what a Mac with nothing published builds itself, whose `sha256` is `-`); `committed` is seconds
//! since 1970; `config` is the custom build's config commit, `-` if it has none. The signature (the release key's, as
//! `ssh-keygen -Y sign -n release-latest` makes it) is over everything before its first line.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};

/// The folder of the bucket, and the path at the network's address, updates are read from. (`src/protocol.rs` gives
/// the Workers the same word.)
pub const FOLDER: &str = crate::protocol::UPDATES;
/// The object replaced by each publish: `<folder>/newest`.
pub fn newest_name() -> String {
    format!("{FOLDER}/{}", crate::protocol::NEWEST)
}

/// The namespace the record is signed in, so nothing else the release key signs passes for one.
pub const NAMESPACE: &str = "release-latest";
const BEGIN: &str = "-----BEGIN SSH SIGNATURE-----";

const FETCH: Duration = Duration::from_secs(30);
const DOWNLOAD: Duration = Duration::from_secs(30 * 60);

/// One thing the record names.
#[derive(Clone, Debug, PartialEq)]
pub struct Thing {
    pub name: String,
    pub commit: String,
    /// When the commit was made, in seconds since 1970.
    pub committed: u64,
    /// The SHA-256 of its file, in hex; `-` for what's built, not downloaded (`source`).
    pub sha256: String,
    pub size: u64,
    /// The custom build's config commit; `-` if none.
    pub config: String,
}

impl Thing {
    pub fn node_name(target: &str) -> String {
        format!("node/{target}")
    }

    pub fn app_name(target: &str) -> String {
        format!("app/{target}")
    }

    pub fn service_name(name: &str, target: &str) -> String {
        format!("service/{name}/{target}")
    }

    fn from_line(line: &str) -> Option<Thing> {
        let mut f = line.split(' ');
        let (name, commit, committed, sha256, size, config) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
        if f.next().is_some() || name.is_empty() || commit.is_empty() {
            return None;
        }
        Some(Thing { name: name.into(), commit: commit.into(), committed: committed.parse().ok()?, sha256: sha256.into(), size: size.parse().ok()?, config: config.into() })
    }

    fn line(&self) -> String {
        format!("{} {} {} {} {} {}", self.name, self.commit, self.committed, self.sha256, self.size, self.config)
    }
}

/// What the record says, as read and checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Latest {
    pub things: Vec<Thing>,
}

impl Latest {
    /// The record's text, if it's signed with `key` (the release key's public half, OpenSSH): a bad one is a device
    /// staying on what it has, not an error anywhere but here.
    pub fn read(key: &str, text: &str) -> Result<Latest> {
        let at = text.find(BEGIN).context("the record isn't signed")?;
        let (body, signature) = text.split_at(at);
        let signature = SshSig::from_pem(signature).map_err(|e| anyhow!("its signature can't be read: {e}"))?;
        let key = PublicKey::from_openssh(key).context("this build's release key")?;
        ensure!(signature.public_key() == key.key_data() && signature.namespace() == NAMESPACE, "it isn't signed by this build's release key");
        key.verify(NAMESPACE, body.as_bytes(), &signature).map_err(|_| anyhow!("its signature doesn't match it"))?;
        Ok(Latest { things: body.lines().filter_map(Thing::from_line).collect() })
    }

    /// The text to publish, signed with the release key.
    pub fn signed(&self, key: &PrivateKey) -> Result<String> {
        let body: String = self.things.iter().map(|t| format!("{}\n", t.line())).collect();
        Ok(format!("{body}{}", key.sign(NAMESPACE, HashAlg::Sha512, body.as_bytes())?.to_pem(LineEnding::LF)?))
    }

    pub fn get(&self, name: &str) -> Option<&Thing> {
        self.things.iter().find(|t| t.name == name)
    }

    /// `new` added to what's here, replacing what has their names, unless what's here is newer than what's offered.
    pub fn merged(mut self, new: impl IntoIterator<Item = Thing>) -> Latest {
        for thing in new {
            match self.things.iter_mut().find(|t| t.name == thing.name) {
                Some(here) if here.committed > thing.committed => {}
                Some(here) => *here = thing,
                None => self.things.push(thing),
            }
        }
        self
    }

    /// What a device on `target` should run: for a Mac, what's to be built (`source`), else its node's build.
    pub fn for_target(&self, target: &str) -> Option<&Thing> {
        if crate::build::Platform::is_mac(target) { self.get("source") } else { self.get(&Thing::node_name(target)) }
    }
}

/// Whether a device running `build`, committed at `committed`, takes `thing`: a different build, and never going
/// backwards. (`taken` is what it last took and hasn't restarted onto.)
pub fn wanted(thing: &Thing, build: &str, committed: u64, taken: &str) -> bool {
    thing.commit != build && !build.starts_with(&thing.commit) && !thing.commit.starts_with(build) && thing.committed >= committed && thing.sha256 != taken
}

fn agent(limit: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(limit)).build().into()
}

fn updates_url(address: &str, file: &str) -> String {
    format!("{}/{FOLDER}/{file}", address.trim_end_matches('/'))
}

/// The record's text at `address`; none if nothing has been published there yet.
pub fn fetch(address: &str) -> Result<Option<String>> {
    match agent(FETCH).get(updates_url(address, crate::protocol::NEWEST)).call() {
        Ok(mut r) => Ok(Some(r.body_mut().with_config().limit(1 << 20).read_to_string()?)),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(err) => Err(anyhow!("couldn't reach {address}: {err}")),
    }
}

/// `thing`'s file, whole and with the hash the record names.
pub fn download(address: &str, thing: &Thing) -> Result<Vec<u8>> {
    let mut resp = agent(DOWNLOAD).get(updates_url(address, &thing.sha256)).call().with_context(|| format!("download {} {}", thing.name, thing.commit))?;
    let bytes = resp.body_mut().with_config().limit(2 << 30).read_to_vec().with_context(|| format!("download {} {}", thing.name, thing.commit))?;
    ensure!(sha256(&bytes) == thing.sha256, "{} {} isn't what the record names (its hash doesn't match)", thing.name, thing.commit);
    Ok(bytes)
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    crate::keys::hex(&digest)
}

/// The hash and size of the file at `path`.
pub fn hashed(path: &Path) -> Result<(String, u64)> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok((sha256(&bytes), bytes.len() as u64))
}

/// The shell that sets a Mac up to take its own updates: the job (`pull.sh`) and what it installs an app with
/// (`install-app.sh`) put in place and started, with nothing that's running stopped. Every value it needs is written in
/// it, since it runs with nothing else of the network. Run by the Mac's oned, and once by hand for a Mac too old to have it.
pub fn mac_job(address: &str) -> String {
    use crate::config::{CLI, CONFIG_FILE, CONFIG_REPO, NAME, RELEASE_KEY, RELEASE_SIGNER, REPO};
    let quoted = |v: &str| v.replace('\'', r"'\''");
    let job = include_str!("../client/native/mac/pull.sh")
        .replace("@CLI@", CLI)
        .replace("@NAME@", &quoted(NAME))
        .replace("@ADDRESS@", &quoted(address))
        .replace("@KEY@", &quoted(RELEASE_KEY))
        .replace("@REPO@", &quoted(REPO))
        .replace("@CONFIG_REPO@", &quoted(CONFIG_REPO))
        .replace("@CONFIG_FILE@", CONFIG_FILE)
        .replace("@SIGNER@", &quoted(RELEASE_SIGNER));
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.pull; plist="$HOME/Library/LaunchAgents/$label.plist"
mkdir -p "$c" "$HOME/Library/LaunchAgents"
put() {{ cat > "$1.new"; chmod 755 "$1.new"; mv "$1.new" "$1"; }}
put "$c/pull.sh" <<'JOB'
{job}JOB
put "$c/install-app.sh" <<'JOB'
{install}JOB
cat > "$plist.new" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>$label</string>
<key>RunAtLoad</key><true/>
<key>StartInterval</key><integer>120</integer>
<key>ProgramArguments</key><array><string>/bin/zsh</string><string>-l</string><string>$c/pull.sh</string></array>
</dict></plist>
PLIST
mv "$plist.new" "$plist"
launchctl print "gui/$(id -u)/$label" > /dev/null 2>&1 || launchctl bootstrap "gui/$(id -u)" "$plist"
"#,
        install = include_str!("../client/native/mac/install-app.sh"),
    )
}


#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::private::Ed25519Keypair;

    fn key(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    fn thing(name: &str, commit: &str, committed: u64) -> Thing {
        Thing { name: name.into(), commit: commit.into(), committed, sha256: "a".repeat(64), size: 3, config: "-".into() }
    }

    /// This text was signed once by an older build. It must read today exactly as it did: a device that can't read
    /// the record can't update itself out of whatever it's stuck on.
    #[test]
    fn a_record_published_by_an_older_build_still_reads() {
        const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIO1JKMYo0cLG6ukDOJBZlWEpWSc6XGP5NjbBRhSshzfR";
        const RECORD: &str = "node/x86_64-unknown-linux-gnu abc123 1700000000 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 3 -
not a line a reader knows
web def456 1700000001 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 7 cfg1
-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg7UkoxijRwsbq6QM4kFmVYSlZJz
pcY/k2NsFGFKyHN9EAAAAOcmVsZWFzZS1sYXRlc3QAAAAAAAAABnNoYTUxMgAAAFMAAAAL
c3NoLWVkMjU1MTkAAABAqZujUSIDQ+J7ZBO2M7ZM1TKgh5qsfSUYLjNbl2ogSzNGa7MdOj
7X50kKXGo+YszdCkBmp9Fz/Y3DShJw9xy8CA==
-----END SSH SIGNATURE-----
";
        let latest = Latest::read(KEY, RECORD).unwrap();
        assert_eq!(latest.things.len(), 2, "a line it doesn't understand is ignored");
        assert_eq!(latest.get("node/x86_64-unknown-linux-gnu").unwrap().commit, "abc123");
        assert_eq!(latest.get("web").unwrap().config, "cfg1");
        assert!(Latest::read(KEY, &RECORD.replace("abc123", "abc124")).is_err(), "a changed line doesn't pass");
    }

    #[test]
    fn only_what_the_release_key_signed_passes() {
        let (ours, theirs) = (key(1), key(2));
        let latest = Latest { things: vec![thing("web", "c1", 10)] };
        let public = ours.public_key().to_openssh().unwrap();
        assert_eq!(Latest::read(&public, &latest.signed(&ours).unwrap()).unwrap(), latest);
        assert!(Latest::read(&public, &latest.signed(&theirs).unwrap()).is_err());
        assert!(Latest::read(&public, "web c1 10 aaaa 3 -\n").is_err(), "unsigned");
        assert!(Latest::read(&public, "").is_err());
    }

    #[test]
    fn it_never_goes_backwards() {
        assert!(wanted(&thing("n", "new", 20), "old", 10, ""));
        assert!(!wanted(&thing("n", "old", 5), "new", 10, ""), "older than what it runs");
        assert!(!wanted(&thing("n", "same", 20), "same", 10, ""));
        assert!(!wanted(&Thing { sha256: "x".into(), ..thing("n", "new", 20) }, "old", 10, "x"), "already taken, waiting to restart");
        let merged = Latest { things: vec![thing("a", "newer", 20)] }.merged([thing("a", "older", 10), thing("b", "b1", 5)]);
        assert_eq!(merged.get("a").unwrap().commit, "newer", "publishing doesn't take a thing backwards either");
        assert_eq!(merged.get("b").unwrap().commit, "b1");
    }
}
