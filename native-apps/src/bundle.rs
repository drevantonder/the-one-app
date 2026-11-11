//! What the window shows: the web client the release key signed and this device last kept, newest first, or the one
//! built in. Both reach the window through the app's own origin, so a bundle changes nothing about what the window's
//! page may ask of the app. A bundle is checked (signature, not older than this has, shell new enough) before it's
//! kept and again every time the app starts, and only ever comes from `one-dev update web`: the place it travels through
//! (the network's address) is a carrier. The page finding a problem with what it got falls back, not to nothing, but
//! to what came before.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use ui::bundle::{self, Bundle};
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{Assets, Manager, Wry};

/// A bundle in the memory it's served from, once checked.
struct Served {
    commit: String,
    committed: u64,
    files: HashMap<String, Vec<u8>>,
}

impl From<Bundle> for Served {
    fn from(b: Bundle) -> Served {
        Served { commit: b.commit, committed: b.committed, files: b.files.into_iter().map(|(path, bytes)| (format!("/{path}"), bytes)).collect() }
    }
}

/// What the window shows now, none for the built-in; and what's waiting for it to next open.
static SERVED: RwLock<Option<Arc<Served>>> = RwLock::new(None);
static WAITING: Mutex<Option<Arc<Served>>> = Mutex::new(None);
/// The page asked the app something since it last loaded, which a bundle whose scripts don't run never does.
static STARTED: AtomicBool = AtomicBool::new(false);

/// How long a bundle's page has to start before the app gives up on that bundle.
const WATCH: Duration = Duration::from_secs(20);

/// The folder bundles are kept in, each as `<commit>.bundle`; `<commit>.bad` is a bundle whose page didn't
/// start, kept away for good; `behind` the shell a bundle needs that this app is too old for.
fn folder(data: &Path) -> PathBuf {
    data.join("web")
}

/// The key a bundle must be signed with: in a debug build, `WEB_KEY` for a test's.
fn release_key() -> String {
    #[cfg(debug_assertions)]
    if let Some(key) = crate::env("WEB_KEY") {
        return key;
    }
    custom_build::config::RELEASE_KEY.to_string()
}

/// `file` is a bundle the release key signed and this app can open. Not itself checked against what's kept: `kept` does that.
fn checked(file: &[u8]) -> Result<Bundle, String> {
    let key = release_key();
    if key.is_empty() {
        return Err("this build has no release key".into());
    }
    bundle::open(&key, file).map_err(|e| e.to_string())
}

/// Bundles kept here, newest first, that still check out; none with a `.bad` beside them.
fn kept(data: &Path) -> Vec<Bundle> {
    let dir = folder(data);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<Bundle> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "bundle" || path.with_extension("bad").exists() {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            match checked(&bytes) {
                Ok(b) => Some(b),
                Err(e) => {
                    device_logs::say(&format!("web bundle {}: {e}; removing it", path.display()));
                    let _ = std::fs::remove_file(&path);
                    None
                }
            }
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.committed));
    out
}

/// What the built-in bundle is, by when its commit was made.
fn built_in() -> u64 {
    custom_build::config::COMMITTED
}

/// Whether this app should run `b`: newer than the copy built into it, and a shell it has.
fn takes(b: &Bundle) -> bool {
    b.committed > built_in() && b.shell <= ui::web::SHELL
}

/// The newest bundle kept here this app can run, or none for the built-in, and how much of a shell the newest needed if
/// it couldn't run that.
fn best(data: &Path) -> (Option<Bundle>, Option<u32>) {
    let mut behind = None;
    for b in kept(data) {
        if b.committed <= built_in() {
            continue;
        }
        if takes(&b) {
            return (Some(b), behind);
        }
        behind = behind.max(Some(b.shell));
    }
    (None, behind)
}

/// What the app shows as it starts, from what it keeps. Never fails: at worst, the built-in.
pub fn start(data: &Path) {
    let (b, behind) = best(data);
    match &b {
        Some(b) => device_logs::say(&format!("web: serving the bundle of {}", b.commit)),
        None => device_logs::say("web: serving the built-in bundle"),
    }
    keep_behind(data, behind);
    *SERVED.write().unwrap() = b.map(|b| Arc::new(Served::from(b)));
}

fn keep_behind(data: &Path, behind: Option<u32>) {
    let path = folder(data).join("behind");
    match behind {
        Some(shell) => {
            let _ = std::fs::create_dir_all(folder(data));
            let _ = std::fs::write(path, shell.to_string());
        }
        None => drop(std::fs::remove_file(path)),
    }
}

/// Whether this app is behind: the newest bundle needs a newer app than this one, or the network reads more of who's who than
/// this app's own client does (`own_client_behind`). Either way it stays on what it has, and says it needs updating.
pub fn behind(data: &Path) -> bool {
    folder(data).join("behind").exists() || folder(data).join("reads-behind").exists()
}

/// Whether what runs with no window (widgets, and what a notification asks) just learned the network reads more than this app's own
/// client does: `refused` is what it was told, if anything. Those things keep what they last had and the page says to update.
pub fn own_client_behind(data: &Path, refused: Option<&str>) {
    let path = folder(data).join("reads-behind");
    if refused.is_some_and(|e| e.contains(&custom_build::protocol::update_refusal(custom_build::config::YOURS))) {
        let _ = std::fs::create_dir_all(folder(data));
        let _ = std::fs::write(path, "");
    } else if refused.is_none() {
        let _ = std::fs::remove_file(path);
    }
}

/// What the network's address says is newest: its commit, when it was made, and the file's name there.
struct Published {
    commit: String,
    committed: u64,
    sha256: String,
}

/// Where bundles are published: the network's own address.
fn base(address: &str) -> String {
    format!("{address}/updates")
}

/// A GET of the network's address, waiting at most `secs` in all.
fn get(secs: u64, url: String) -> Result<ureq::http::Response<ureq::Body>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(secs))).http_status_as_error(false).build().into();
    let request = ureq::http::Request::get(url).body(Vec::new()).map_err(|e| e.to_string())?;
    platform::hedge::run(&agent, "updates", request, platform::hedge::Twice::Safe).map(|(r, _)| r).map_err(|e| e.to_string())
}

/// What the place says is newest, of the web client: its `web` line of `updates/newest` (`web <commit> <committed> <sha256> <size> <config>`).
/// That file is signed, but it's only a pointer: `checked` is what decides, so this reads it as text and nothing rests on it.
fn latest(address: &str) -> Result<Option<Published>, String> {
    let mut r = get(15, format!("{}/newest", base(address)))?;
    if !r.status().is_success() {
        return Ok(None);
    }
    let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(text.lines().find_map(|line| {
        let mut f = line.split_whitespace();
        (f.next()? == "web").then_some(())?;
        Some(Published { commit: f.next()?.to_string(), committed: f.next()?.parse().ok()?, sha256: f.next()?.to_string() })
    }))
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut r = get(120, url.to_string())?;
    if !r.status().is_success() {
        return Err(format!("{url} answered {}", r.status()));
    }
    r.body_mut().with_config().limit(1 << 28).read_to_vec().map_err(|e| e.to_string())
}

/// Keeps `file`, the bundle `b`, written whole or not at all; drops all but it and the ones before it `watch` needs.
fn keep(data: &Path, file: &[u8], b: &Bundle) -> Result<(), String> {
    let dir = folder(data);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{}.part", b.commit));
    std::fs::write(&tmp, file).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(format!("{}.bundle", b.commit))).map_err(|e| e.to_string())?;
    // Two this app can run (the one before is what `watch` falls back to), and the newest, which may need a newer app.
    let all = kept(data);
    let mut runnable = 0;
    for (i, old) in all.iter().enumerate() {
        let run = takes(old);
        runnable += run as usize;
        if i != 0 && (!run || runnable > 2) {
            let _ = std::fs::remove_file(dir.join(format!("{}.bundle", old.commit)));
        }
    }
    Ok(())
}

/// Looks for a newer bundle at the network's address; a good one is kept for the next time the window opens. Nothing here
/// ever reaches the window before that, and any failure leaves things as they are: it's only logged.
pub fn update(data: PathBuf, address: String) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = fetched(&data, &address) {
            device_logs::say(&format!("web: no new bundle: {e}"));
        }
    });
}

fn fetched(data: &Path, address: &str) -> Result<(), String> {
    let Some(published) = latest(address)? else { return Ok(()) };
    let has = SERVED.read().unwrap().as_ref().map_or(built_in(), |s| s.committed).max(WAITING.lock().unwrap().as_ref().map_or(0, |s| s.committed));
    if published.committed <= has || folder(data).join(format!("{}.bad", published.commit)).exists() {
        return Ok(());
    }
    let bytes = download(&format!("{}/{}", base(address), published.sha256))?;
    let b = checked(&bytes).map_err(|why| format!("refused {}: {why}", published.commit))?;
    if b.committed <= has {
        return Err(format!("refused {}: it's older than what this has", b.commit));
    }
    if !b.commit.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("refused a bundle whose commit isn't one".into());
    }
    keep(data, &bytes, &b)?;
    if b.shell > ui::web::SHELL {
        device_logs::say(&format!("web: bundle {} needs a newer app (shell {})", b.commit, b.shell));
        keep_behind(data, Some(b.shell));
        return Ok(());
    }
    keep_behind(data, None);
    device_logs::say(&format!("web: bundle {} kept for the next time the window opens", b.commit));
    *WAITING.lock().unwrap() = Some(Arc::new(Served::from(b)));
    Ok(())
}

/// The window opened again (a computer's, shown after it was closed, or a phone's, back from the background): a bundle waiting
/// for it goes in, as nothing's running under it.
pub fn reopened(app: &tauri::AppHandle) {
    static CHECKED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let mut last = CHECKED.lock().unwrap();
        if last.is_none_or(|t| t.elapsed() > Duration::from_secs(1800))
            && let Some(address) = crate::app().address()
        {
            *last = Some(std::time::Instant::now());
            update(crate::app().data.clone(), address);
        }
    }
    let Some(next) = WAITING.lock().unwrap().take() else { return };
    device_logs::say(&format!("web: serving the bundle of {}", next.commit));
    *SERVED.write().unwrap() = Some(next);
    STARTED.store(false, Ordering::Relaxed);
    crate::app().page.store(false, Ordering::Relaxed);
    watch(app.clone());
    for label in ["main", "panel"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.eval("location.reload()");
        }
    }
}

/// The page ran far enough to ask the app something.
pub fn started() {
    STARTED.store(true, Ordering::Relaxed);
}

/// Whether the page from a bundle started in time: if not, that bundle is put away for good and the page loads what came
/// before it, so a bad one is never the last thing a user sees. The built-in has nothing to fall back on.
pub fn watch(app: tauri::AppHandle) {
    let data = crate::app().data.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(WATCH).await;
        if STARTED.load(Ordering::Relaxed) {
            return;
        }
        let Some(failed) = SERVED.read().unwrap().clone() else { return };
        device_logs::say(&format!("web: the page of bundle {} didn't start; going back", failed.commit));
        let _ = std::fs::write(folder(&data).join(format!("{}.bad", failed.commit)), "");
        let _ = std::fs::remove_file(folder(&data).join(format!("{}.bundle", failed.commit)));
        let (b, _) = best(&data);
        *SERVED.write().unwrap() = b.map(|b| Arc::new(Served::from(b)));
        watch(app.clone());
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.eval("location.reload()");
        }
    });
}

/// The page's policy, so the page may reach the app (Tauri's `ipc:`) and, for its own client, the network's address and
/// its relay, wherever that is: which the page's own file can't say, as an address is a user's to give.
fn policy(html: &[u8]) -> Vec<u8> {
    let address = crate::APP.get().and_then(|a| a.address()).map(|a| format!(" {a} {}", a.replacen("http", "ws", 1))).unwrap_or_default();
    String::from_utf8_lossy(html).replacen("connect-src 'self'", &format!("connect-src 'self' ipc: http://ipc.localhost{address}"), 1).into_bytes()
}

/// What Tauri asks for the window's files, in place of the built-in ones: the bundle being served, else those.
struct Web(Box<dyn Assets<Wry>>);

impl Assets<Wry> for Web {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        let served = SERVED.read().unwrap().clone();
        let bytes = match served {
            Some(s) => Cow::Owned(s.files.get(key.as_ref())?.clone()),
            None => self.0.get(key)?,
        };
        Some(if key.as_ref() == "/index.html" { Cow::Owned(policy(&bytes)) } else { bytes })
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.0.iter()
    }

    fn csp_hashes(&self, html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        self.0.csp_hashes(html_path)
    }
}

/// The app's context, its window's files coming from here.
pub fn serving(mut context: tauri::Context<Wry>) -> tauri::Context<Wry> {
    struct Nothing;
    impl Assets<Wry> for Nothing {
        fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
            None
        }
        fn iter(&self) -> Box<AssetsIter<'_>> {
            Box::new(std::iter::empty())
        }
        fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
            Box::new(std::iter::empty())
        }
    }
    let built_in = context.set_assets(Box::new(Nothing));
    context.set_assets(Box::new(Web(built_in)));
    context
}
