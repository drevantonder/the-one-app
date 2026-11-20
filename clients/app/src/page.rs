//! The window's page (`page/`): the copy built into the app, or a newer one its network has, as Expo Updates, CodePush
//! and Capacitor's live updates give an app's web half. The network's edge serves the page at `<address>/page/`, with
//! what its `update.json` says of it (`core/edge/edge.js`): its version, the lowest build of the app it runs on
//! (`native`, from `page/update.json`) and each file's SHA-256. As the app opens, and as its network says its edge is
//! another build (`updater`), it asks for that, and keeps a newer one it runs on in its home (`update`), which it
//! loads as it next starts or reloads (`load`): a computer's app starts again on it by itself, and a phone's reloads
//! it once it's off screen. It loads the copy built into it while it keeps none newer that it runs on, so with no
//! network too.
//!
//! As Tauri's updater and Sparkle do, it takes a page only with the platform's Ed25519 signature of its `update.json`
//! (`update.json.sig`, made at the edge's deploy), checked with the public half built in here, `KEY`, as it's
//! downloaded and again each time a kept one is loaded. That page can reach everything the app can, the recovery key
//! among it, so neither whoever runs the network's edge nor anything in front of it may choose it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::{Context, bail};
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The public half of the key the platform signs the page's `update.json` with, PAGE_SIGNING_KEY at the edge's
/// deploy (in 1Password, "The One App page signing key").
const KEY: &str = "58a090e1ddf4e3dc9bec1cfe5ba83fe987c54a8ed05455d871ae3514e33fc4ef";

/// What a network says of its page, and what a page kept here says of itself: its `update.json`.
#[derive(Deserialize)]
struct Update {
    version: Version,
    native: Version,
    /// Each file's SHA-256, by its name.
    files: BTreeMap<String, String>,
}

impl Update {
    /// Whether this build of the app runs it, and it's newer than `than`.
    fn newer(&self, than: &Version) -> bool {
        self.version > *than && self.native <= build()
    }
}

/// This build of the app, whose page is the one built into it.
fn build() -> Version {
    Version::parse(::client::BUILD).expect("a build is its version")
}

/// The kept page the window loads, by its files' names, once it's chosen (`load`), or none: the one built in.
static KEPT: RwLock<Option<Arc<BTreeMap<String, Vec<u8>>>>> = RwLock::new(None);

/// Chooses the page the window loads, from the copies kept in `home`: the newest this build runs on that's newer than
/// the one built into it, whole, or none. It's read as the app starts, and again before its window reloads, and held,
/// so a newer one kept meanwhile waits for that.
pub fn load(home: &Path) {
    let mut kept = copies(home).filter(|(update, _)| update.newer(&build())).collect::<Vec<_>>();
    kept.sort_by(|(a, _), (b, _)| b.version.cmp(&a.version));
    let chosen = kept.into_iter().find_map(|(update, folder)| match read(&update, &folder) {
        Ok(files) => {
            eprintln!("the window's page is the network's {}", update.version);
            Some(Arc::new(files))
        }
        Err(error) => {
            eprintln!("the page kept in {} isn't whole: {error:#}", folder.display());
            None
        }
    });
    *KEPT.write().expect("nothing panics holding it") = chosen;
}

/// One of the kept page's files the window loads, by the path it's served at, if it loads one (`load`).
#[cfg(mobile)]
pub fn kept(path: &str) -> Option<Vec<u8>> {
    KEPT.read().expect("nothing panics holding it").as_ref()?.get(path.trim_start_matches('/')).cloned()
}

/// The pages kept in `home` whose `update.json` is signed, each with its folder. One still being downloaded
/// (`.<version>`) isn't one.
fn copies(home: &Path) -> impl Iterator<Item = (Update, std::path::PathBuf)> {
    let folders = std::fs::read_dir(home.join("page")).into_iter().flatten().flatten();
    let folders = folders.filter(|entry| !entry.file_name().to_string_lossy().starts_with('.')).map(|entry| entry.path());
    folders.filter_map(|folder| {
        let read = |name| std::fs::read(folder.join(name)).with_context(|| format!("{name} isn't there"));
        match read("update.json").and_then(|said| signed(&said, &read("update.json.sig")?)) {
            Ok(update) => Some((update, folder)),
            Err(error) => {
                eprintln!("the page kept in {} isn't the platform's: {error:#}", folder.display());
                None
            }
        }
    })
}

/// What `update.json` says, if `sig` is the platform's signature of it.
fn signed(said: &[u8], sig: &[u8]) -> anyhow::Result<Update> {
    let sig = data_encoding::BASE64.decode(sig.trim_ascii()).context("update.json.sig isn't base64")?;
    let sig = Signature::from_slice(&sig).context("update.json.sig isn't a signature")?;
    let key = data_encoding::HEXLOWER.decode(KEY.as_bytes()).ok().and_then(|key| VerifyingKey::try_from(key.as_slice()).ok());
    key.expect("KEY is a public key").verify_strict(said, &sig).context("update.json isn't signed by the platform")?;
    serde_json::from_slice(said).context("update.json")
}

/// A kept page's files, each as its `update.json` says it is.
fn read(update: &Update, folder: &Path) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for (name, sha256) in &update.files {
        let file = std::fs::read(folder.join(name)).with_context(|| format!("{name} isn't there"))?;
        if hex(&file) != *sha256 {
            bail!("{name} isn't what update.json says");
        }
        files.insert(name.clone(), file);
    }
    Ok(files)
}

fn hex(file: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(file))
}

/// Asks the network for its page, and keeps it in `home` if this build runs it and it's newer than the one built in
/// and every one kept, for the next start. Older ones kept go, since each run holds the page it loads. → whether it
/// kept one.
pub async fn update(client: &::client::Client, home: &Path) -> anyhow::Result<bool> {
    let Some(address) = client.address() else { return Ok(false) };
    let at = format!("{}/page/", address.trim_end_matches('/'));
    let web = ::client::web()?;
    let get = async |name: &str| -> anyhow::Result<Vec<u8>> {
        Ok(web.get(format!("{at}{name}")).send().await?.error_for_status()?.bytes().await?.to_vec())
    };
    let said = get("update.json").await?;
    let sig = get("update.json.sig").await?;
    let update = signed(&said, &sig).context("the network's page")?;
    let newest = copies(home).map(|(kept, _)| kept.version).chain([build()]).max().expect("there's the build");
    if !update.newer(&newest) {
        return Ok(false);
    }
    let folder = home.join("page");
    let partial = folder.join(format!(".{}", update.version));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)?;
    for (name, sha256) in &update.files {
        if name.contains(['/', '\\']) || name.starts_with('.') {
            bail!("the network's page names a file {name}, which isn't a name");
        }
        let file = get(name).await?;
        if hex(&file) != *sha256 {
            bail!("the network's {name} isn't what its update.json says");
        }
        std::fs::write(partial.join(name), file)?;
    }
    std::fs::write(partial.join("update.json"), said)?;
    std::fs::write(partial.join("update.json.sig"), sig)?;
    // One of this version that isn't the platform's may be there.
    let whole = folder.join(update.version.to_string());
    let _ = std::fs::remove_dir_all(&whole);
    std::fs::rename(&partial, &whole)?;
    for older in std::fs::read_dir(&folder)?.flatten().map(|entry| entry.path()).filter(|path| *path != whole) {
        let _ = std::fs::remove_dir_all(older);
    }
    eprintln!("kept the network's page {}, for the next start", update.version);
    Ok(true)
}

/// What kind of file one of the page's own is.
pub fn kind(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "text/plain",
    }
}
