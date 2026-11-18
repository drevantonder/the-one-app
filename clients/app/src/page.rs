//! The window's page (`page/`): the copy built into the app, or a newer one its network has, as Expo Updates, CodePush
//! and Capacitor's live updates give an app's web half. The network's edge serves the page at `<address>/page/`, with
//! what its `update.json` says of it (`core/edge/edge.js`): its version, the lowest build of the app it runs on
//! (`native`, from `page/update.json`) and each file's SHA-256. As the app opens it asks for that, and keeps a newer
//! one it runs on in its home (`update`), which it loads from its next start (`load`), as Expo does by default. It
//! loads the copy built into it while it keeps none newer that it runs on, so with no network too.
//!
//! As Tauri's updater and Sparkle do, it takes a page only with the platform's Ed25519 signature of its `update.json`
//! (`update.json.sig`, made at the edge's deploy), checked with the public half built in here, `KEY`, as it's
//! downloaded and again each time a kept one is loaded. That page can reach everything the app can, the recovery key
//! among it, so neither whoever runs the network's edge nor anything in front of it may choose it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, bail};
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

// A computer's built-in copy, which is this program's own files (`build.rs`), and the app's name. A phone's is
// Tauri's.
#[cfg(desktop)]
include!(concat!(env!("OUT_DIR"), "/page.rs"));

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

/// The kept page this run loads, by its files' names, once it's chosen (`load`).
static KEPT: OnceLock<Option<BTreeMap<String, Vec<u8>>>> = OnceLock::new();

/// Chooses the page this run loads, from the copies kept in `home`: the newest this build runs on that's newer than
/// the one built into it, whole, or none. It's read once, as the app starts, and held, so a newer one kept
/// meanwhile waits for the next start.
pub fn load(home: &Path) {
    KEPT.get_or_init(|| {
        let mut kept = copies(home).filter(|(update, _)| update.newer(&build())).collect::<Vec<_>>();
        kept.sort_by(|(a, _), (b, _)| b.version.cmp(&a.version));
        kept.into_iter().find_map(|(update, folder)| match read(&update, &folder) {
            Ok(files) => {
                eprintln!("the window's page is the network's {}", update.version);
                Some(files)
            }
            Err(error) => {
                eprintln!("the page kept in {} isn't whole: {error:#}", folder.display());
                None
            }
        })
    });
}

/// One of the kept page's files this run loads, by the path it's served at, if it loads one (`load`).
pub fn kept(path: &str) -> Option<&'static [u8]> {
    KEPT.get()?.as_ref()?.get(path.trim_start_matches('/')).map(Vec::as_slice)
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
/// and every one kept, for the next start. Older ones kept go, since each run holds the page it loads.
pub async fn update(client: &::client::Client, home: &Path) -> anyhow::Result<()> {
    let Some(address) = client.address() else { return Ok(()) };
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
        return Ok(());
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
    Ok(())
}

/// One of the page's files as the web view's answer for it, with what it may load: the kept page's, or the one
/// built in.
#[cfg(desktop)]
pub fn respond(path: &str) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    use wry::http::{Response, StatusCode, header};
    let file = match KEPT.get() {
        Some(Some(_)) => kept(path),
        _ => FILES.iter().find(|(served, _)| *served == path).map(|(_, file)| *file),
    };
    let response = match file {
        Some(file) => Response::builder().header(header::CONTENT_TYPE, kind(path)).header(header::CONTENT_SECURITY_POLICY, CSP).body(file.into()),
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new().into()),
    };
    response.expect("these headers are valid")
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
