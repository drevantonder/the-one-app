//! The native app: one window, which is the web client's own page (cloudflare/web/web.js, its files from one::web,
//! built in), drawn and run as it is in a browser, but on this device's onet rather than a browser's: on a phone, built
//! into the app, with the phone's key in the OS's secret store; on a computer, the computer's own (`here`), of which the
//! window is only a view, with no key or background program of its own.
//! The page finds the app as `window.__oneNative` (src/native.js), which only it has: screens run in sandboxed frames
//! in it with no origin of their own, which reach none of the app's commands, and ask the page, as they do in a
//! browser. What the app adds is what a page can't do: signing in through the system's browser, notifications, the
//! clipboard, files, and links outside.

#[macro_use]
mod commands;
mod bundle;
mod headless;
#[cfg(desktop)]
mod here;
mod platform;
mod secrets;
mod signin;
mod widgets;

pub use headless::answer as headless;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// The app's id, as build.rs names it for this platform: `app.<cli>.<platform>` (a throwaway's, `….throwaway`). Android
/// names nothing by it: Kotlin gives its folder.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn identifier() -> &'static str {
    env!("IDENTIFIER")
}

/// What the user calls this device, as their other devices list it.
fn device_name() -> String {
    platform::name().unwrap_or_else(|| platform::LABEL.into())
}

/// The platform, as users say it (in a debug build, `PLATFORM` for a test: a phone's look on a computer).
fn label() -> &'static str {
    #[cfg(debug_assertions)]
    if let Some(p) = env("PLATFORM") {
        return if p == "iPhone" { "iPhone" } else if p == "Android" { "Android" } else { "Linux" };
    }
    platform::LABEL
}

/// A setting for a test from its environment, named by the CLI's name (`ONE_ADDRESS`): `ADDRESS`, the network's;
/// `SIGN_IN_PRINT`, sign in by printing the link rather than opening a browser (`one-dev throwaway client linux` signs in
/// there); `CONTROL`, in a debug build, a loopback port an agent drives the window by.
fn env(name: &str) -> Option<String> {
    std::env::var(format!("{}_{name}", ::custom_build::config::CLI.to_uppercase())).ok().filter(|v| !v.is_empty())
}

/// What this app asks its user's devices through: a phone's own client, with the phone's key; or the computer's onet,
/// which the window is a view of.
#[derive(Clone)]
enum Reach {
    #[cfg(mobile)]
    Device(Arc<onet::Device>),
    #[cfg(desktop)]
    Onet,
}

/// A response's body, as it comes.
enum Body {
    #[cfg(mobile)]
    Device(onet::Body),
    #[cfg(desktop)]
    Onet(tokio::sync::mpsc::Receiver<Result<Vec<u8>, String>>),
}

impl Body {
    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self {
            #[cfg(mobile)]
            Body::Device(body) => body.chunk().await.map_err(|e| e.to_string()),
            #[cfg(desktop)]
            Body::Onet(parts) => parts.recv().await.transpose(),
        }
    }

    async fn bytes(mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        while let Some(part) = self.chunk().await? {
            out.extend_from_slice(&part);
        }
        Ok(out)
    }
}

/// A response: its status and headers, its body, and for a phone's own client, the trace of how it was reached.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Body,
    trace: Option<Value>,
}

impl Reach {
    /// `method` of `path`, with `headers` and `body`; a request a phone's key isn't let in for any more fails as `not_let_in`.
    async fn fetch(&self, method: &str, path: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<Answer, String> {
        match self {
            #[cfg(mobile)]
            Reach::Device(device) => {
                let r = device.fetch(method, path, headers, body).await.map_err(|e| if e.not_let_in { "not_let_in".to_string() } else { e.to_string() })?;
                Ok(Answer { status: r.status, headers: r.headers, body: Body::Device(r.body), trace: r.trace })
            }
            #[cfg(desktop)]
            Reach::Onet => {
                let r = here::fetch(method.to_string(), path.to_string(), headers, body).await?;
                Ok(Answer { status: r.status, headers: r.headers, body: Body::Onet(r.body), trace: None })
            }
        }
    }

    /// Registers where a push service reaches this device with the network's notifications Worker.
    async fn register_notifications(&self, address: &notifications::Address) -> Result<(), String> {
        match self {
            #[cfg(mobile)]
            Reach::Device(device) => device.register_notifications(address).await.map_err(|e| e.to_string()),
            #[cfg(desktop)]
            Reach::Onet => here::asked("/api/notifications/register", Some(json!(address)), 30).await.map(drop),
        }
    }
}

/// A response's body the page reads, and what stops a read waiting on it once the page cancels.
struct Reading {
    body: tokio::sync::Mutex<Body>,
    stop: tokio::sync::Notify,
}

struct App {
    handle: AppHandle,
    /// Where it keeps what isn't secret: the address, who's signed in, and the client's own folder per user.
    data: PathBuf,
    sign_in: Mutex<Option<Arc<signin::SignIn>>>,
    /// A phone's own client, once it's let in.
    #[cfg(mobile)]
    device: Mutex<Option<Arc<onet::Device>>>,
    /// Responses whose bodies the page reads as they come, by id.
    bodies: Mutex<HashMap<u64, Arc<Reading>>>,
    next: AtomicU64,
    /// A link the app was opened with before its page asked (`native`), and whether it has since.
    opened: Mutex<Option<String>>,
    page: std::sync::atomic::AtomicBool,
}

static APP: OnceLock<App> = OnceLock::new();

fn app() -> &'static App {
    APP.get().expect("the app")
}

/// What the app keeps in `data` as `name`, if anything.
fn kept_in(data: &std::path::Path, name: &str) -> Option<String> {
    std::fs::read_to_string(data.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The network's address: `ADDRESS` for a test, the custom build's if it has one, else as the user gave it.
fn address_in(data: &std::path::Path) -> Option<String> {
    env("ADDRESS").or_else(|| Some(::custom_build::config::ADDRESS.to_string()).filter(|a| !a.is_empty())).or_else(|| kept_in(data, "address")).map(|a| a.trim_end_matches('/').to_string())
}

/// This device's client for `id` (`<address> <user>`), its folder in `data` named by it.
fn client(data: &std::path::Path, address: &str, id: &str, secret: [u8; 32]) -> onet::Device {
    onet::Device::new(secret, address, &data.join("client").join(secrets::folder(id)).to_string_lossy()).keeping().logging(data)
}

/// A device's code, as typed or sent in a notification: its letters and digits.
fn device_code(typed: &str) -> String {
    typed.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

impl App {
    fn kept(&self, name: &str) -> Option<String> {
        kept_in(&self.data, name)
    }

    fn keep(&self, name: &str, value: Option<&str>) {
        let path = self.data.join(name);
        match value {
            Some(v) => {
                let _ = std::fs::create_dir_all(&self.data);
                let _ = std::fs::write(path, v);
            }
            None => drop(std::fs::remove_file(path)),
        }
    }

    fn address(&self) -> Option<String> {
        address_in(&self.data)
    }

    fn sign_in(&self) -> Result<Arc<signin::SignIn>, String> {
        let address = self.address().ok_or("Enter your address first.")?;
        let mut kept = self.sign_in.lock().unwrap();
        if let Some(a) = kept.as_ref().filter(|a| a.address == address) {
            return Ok(a.clone());
        }
        let a = Arc::new(signin::SignIn::new(&address, &format!("{} on {}", ::custom_build::config::NAME, device_name()), self.data.clone()));
        *kept = Some(a.clone());
        Ok(a)
    }

    #[cfg(mobile)]
    fn device(&self) -> Result<Arc<onet::Device>, String> {
        self.device.lock().unwrap().clone().ok_or_else(|| "This device hasn't been approved yet.".into())
    }

    /// What it asks its user's devices through, once it can.
    fn reach(&self) -> Result<Reach, String> {
        #[cfg(mobile)]
        return self.device().map(Reach::Device);
        #[cfg(desktop)]
        Ok(Reach::Onet)
    }
}

/// What a node answers `method` at `path` (with `body`, as JSON), as JSON: its `error`, or a status that isn't a success,
/// an Err.
async fn node(reach: &Reach, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let (headers, body) = match body {
        Some(b) => (vec![("content-type".into(), "application/json".into())], serde_json::to_vec(&b).map_err(|e| e.to_string())?),
        None => (Vec::new(), Vec::new()),
    };
    let r = reach.fetch(method, path, headers, body).await?;
    let status = r.status;
    let out: Value = serde_json::from_slice(&r.body.bytes().await?).unwrap_or_default();
    match out["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None if !(200..300).contains(&status) => Err("That didn’t work. Try again.".into()),
        None => Ok(out),
    }
}

/// The widgets kept for the platform's own again, and drawn again where the app runs.
async fn widgets_changed(reach: &Reach) {
    let said = widgets::refresh(reach, &platform::widgets_dir()).await;
    if let Err(e) = &said {
        device_logs::say(&format!("widgets: {e}"));
    }
    if let Some(a) = APP.get() {
        bundle::own_client_behind(&a.data, said.as_ref().err().map(String::as_str));
    }
    if APP.get().is_some() {
        platform::redraw_widgets();
    }
}

/// The widgets kept for the platform's own, again now, and every five minutes while the app runs (once it's let in).
fn keep_widgets() {
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::Relaxed) {
        return changed();
    }
    tauri::async_runtime::spawn(async {
        loop {
            changed();
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

/// Let in, now or as it starts: the platform's own widgets kept, and its notifications registered.
fn let_in() {
    keep_widgets();
    #[cfg(mobile)]
    if let Ok(device) = app().device() {
        platform::let_in(device);
    }
    #[cfg(desktop)]
    platform::let_in();
}

/// The panel, drawn before this device was in (signed out), drawn again now it is.
fn redraw_panel() {
    if let Some(p) = app().handle.get_webview_window("panel") {
        let _ = p.eval("location.reload()");
    }
}

/// In its network, as the window first finds it: `let_in`, and on a computer, the key the window had of its own before it
/// was a view of the computer's onet leaving (`leave_own_key`).
fn let_in_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let_in();
        #[cfg(desktop)]
        tauri::async_runtime::spawn(leave_own_key());
    });
}

/// The key this window had of its own, before it was a view of the computer's onet, leaves its user's devices
/// (`Device::sign_out`, by `Keyring::leave`, which makes no epoch) and is forgotten, with its sign-in and what was kept as
/// it: a computer is one device, with one key. Only once its onet is in, so nothing of theirs is left needing it.
#[cfg(desktop)]
async fn leave_own_key() {
    let a = app();
    secrets::forget_device(secrets::SIGNING_IN);
    let (Some(address), Some(user)) = (a.address(), a.kept("user")) else { return };
    let id = format!("{address} {user}");
    if let Some(secret) = secrets::device(&id) {
        let device = client(&a.data, &address, &id, secret);
        let _ = tokio::time::timeout(Duration::from_secs(30), device.sign_out()).await;
        secrets::forget_device(&id);
        device_logs::say("this window's own key left: the computer's is its only one");
    }
    if let Ok(signing) = a.sign_in() {
        let _ = tauri::async_runtime::spawn_blocking(move || signing.sign_out()).await;
    }
    a.keep("user", None);
    let _ = std::fs::remove_dir_all(a.data.join("client"));
}

/// Something changed on the user's devices (the page's events say): the widgets kept for the platform's own again.
#[tauri::command]
fn changed() {
    let Ok(reach) = app().reach() else { return };
    tauri::async_runtime::spawn(async move { widgets_changed(&reach).await });
}

/// What waits for the user (every app's badge on every page, and Settings'), counted on the app's icon; none at 0.
#[tauri::command]
fn badge(count: u32) {
    platform::badge(count);
}

/// Why a command didn't work, as the page reads it: what to say, and whether it's that this device is signed out (auth
/// refused its sign-in), which the page shows as signed out.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Failure {
    message: String,
    signed_out: bool,
}

impl From<String> for Failure {
    fn from(message: String) -> Failure {
        Failure { message, signed_out: false }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Failure {
        message.to_string().into()
    }
}

impl From<onet::sign_in::Failed> for Failure {
    fn from(f: onet::sign_in::Failed) -> Failure {
        Failure { message: f.message, signed_out: f.signed_out }
    }
}

/// An access token, from the refresh token in the secret store (a request to auth, so off the async threads).
async fn token() -> Result<String, Failure> {
    let signing = app().sign_in()?;
    Ok(tauri::async_runtime::spawn_blocking(move || signing.token()).await.map_err(|e| e.to_string())??)
}

/// Who an access token is for.
fn subject(token: &str) -> Result<String, Failure> {
    onet::sign_in::subject(token).ok_or_else(|| onet::sign_in::signed_out("Sign in again: that sign-in names no one.").into())
}

// MARK: What the page asks

/// Who's signed in here: on a computer in a network, the user its onet is in it as (`stands`, as onet said it); else
/// who signed in here, to be let in as. None if no one is.
fn signed_in(stands: Option<&Value>) -> Option<String> {
    if let Some(user) = stands.filter(|s| s["in"] == true).and_then(|s| s["user"].as_str()) {
        return Some(user.to_string());
    }
    let a = app();
    // Only signing out takes the kept user: a device let in by its key stays in whatever became of its refresh token,
    // which only asking to be let in needs.
    a.address().and(a.kept("user"))
}

/// Who and where this is: the custom build's name and CLI, the address (if one's given), whether someone's signed in
/// here and who. Off the main thread, as a computer's onet starting again is waited for.
#[tauri::command(async)]
fn native() -> Value {
    let a = app();
    #[cfg(desktop)]
    let stands = here::stands();
    #[cfg(mobile)]
    let stands: Option<Value> = None;
    // A computer in a network has its address, which the window then keeps, and is let in already.
    if let Some(address) = stands.as_ref().filter(|s| s["in"] == true).and_then(|s| s["address"].as_str()) {
        a.keep("address", Some(address));
        let_in_once();
    }
    let address = a.address();
    let user = signed_in(stands.as_ref());
    a.page.store(true, Ordering::Relaxed);
    bundle::started();
    let opened = a.opened.lock().unwrap().take();
    // A computer whose onet is in no network yet joins one with a pairing code (`pair`).
    let unpaired = stands.as_ref().is_some_and(|s| s["in"] == false);
    platform::menus(user.is_some(), unpaired);
    json!({
        "name": ::custom_build::config::NAME, "yours": ::custom_build::config::YOURS, "cli": ::custom_build::config::CLI, "platform": label(), "device": device_name(), "client": platform::CLIENT,
        "address": address, "signedIn": user.is_some(), "user": user,
        "opened": opened,
        // A computer's window is a view of its onet, with no key of its own: the page asks through the app.
        "view": cfg!(desktop),
        // This computer, in a network: "This device" in the panel.
        "node": stands.as_ref().filter(|s| s["in"] == true).and_then(|s| s["join"].as_str()).map(|j| j[..64].to_string()),
        "unpaired": unpaired,
        // Open at login: whether it's on, where the app switches it (none where it can't).
        "login": platform::login(),
        // The document camera, where there's one (an iPhone's, Android's scanner).
        "scan": platform::can_scan(),
        // An iPhone's touch and hold lifting what's held with its menu (`menu`).
        "lifts": cfg!(target_os = "ios"),
        // How much bigger than usual the user has made text (a phone's: Android's font size, an iPhone's Dynamic Type),
        // for the window to be zoomed by.
        "textScale": platform::text_scale(),
        // The newest bundle needs a newer app than this, which stays on what it has.
        "behind": bundle::behind(&a.data),
        // What it's called among its user's devices, and its invite: for its own client, in the page, to ask to be let in.
        "joinName": device_name(),
        "joinPlatform": onet::PLATFORM,
        "invite": a.kept("invite"),
    })
}

/// Takes what the user typed: an address, or an invite link (`<address>/invite#…`), whose secret goes with this
/// device's request to be let in.
#[tauri::command]
fn address(typed: String) -> Result<String, String> {
    address_of(&typed)
}

fn address_of(typed: &str) -> Result<String, String> {
    let mut text = typed.trim().to_string();
    if !text.contains("://") {
        text = format!("https://{text}");
    }
    let url = url::Url::parse(&text).map_err(|_| "That isn’t an address. It looks like one.example.com.".to_string())?;
    let host = url.host_str().unwrap_or_default();
    if !(url.scheme() == "https" || (url.scheme() == "http" && ["localhost", "127.0.0.1"].contains(&host))) || host.is_empty() {
        return Err("That isn’t an address. It looks like one.example.com.".into());
    }
    let origin = url.origin().ascii_serialization();
    let a = app();
    a.keep("address", Some(&origin));
    let invite = (url.path() == "/invite").then(|| url.fragment().unwrap_or_default().to_string()).filter(|s| !s.is_empty());
    a.keep("invite", invite.as_deref());
    platform::claim(a.address());
    Ok(origin)
}

/// Signs in, in the system's browser, which comes back to this device; naming the device it's for, so the user's
/// devices can check this sign-in.
#[tauri::command]
async fn sign_in() -> Result<(), Failure> {
    let a = app();
    let address = a.address().ok_or("Enter your address first.")?;
    #[cfg(mobile)]
    let join = onet::join_of(secrets::signing_in()?);
    // A computer's sign-in names the computer's own key, which its onet holds: set up first, if it isn't.
    #[cfg(desktop)]
    let join = here::ready(&address).await?["join"].as_str().ok_or("This computer's onet didn't say what it is.")?.to_string();
    let signing = a.sign_in()?;
    let print = env("SIGN_IN_PRINT").is_some();
    tauri::async_runtime::spawn_blocking(move || platform::sign_in(&signing, &join, print)).await.map_err(|e| e.to_string())??;
    let user = subject(&token().await?)?;
    a.keep("user", Some(&user));
    #[cfg(mobile)]
    start_over_if_taken_out(&address, &user).await;
    #[cfg(desktop)]
    drop(address);
    // Back in front of the browser it signed in with.
    platform::show();
    Ok(())
}

/// Signed in again with a key the relay let in once and refuses now, which can't be let in again: it goes, with what it
/// kept as that device, so the next is the new one this sign-in named (`secrets::device_made`).
#[cfg(mobile)]
async fn start_over_if_taken_out(address: &str, user: &str) {
    let id = format!("{address} {user}");
    let Some(secret) = secrets::device(&id) else { return };
    let device = client(&app().data, address, &id, secret);
    if device.removed().await {
        device.start_over().await;
        secrets::forget_device(&id);
    }
}

/// Makes this device its user's: one already, or asking to be let in. → `{ in: true }`, or the code to approve
/// it by (`{ code }`), after which `wait`. Where the page has no client of its own, this is how it asks.
#[tauri::command]
async fn join() -> Result<Value, Failure> {
    let joined = join_here().await;
    browser_goes_on(joined.as_ref().ok().and_then(|j| j["code"].as_str()));
    joined
}

/// This device has asked to join (by `code`), or needn't: the browser it signed in with goes on (`SignIn::asked`).
fn browser_goes_on(code: Option<&str>) {
    if let Ok(signing) = app().sign_in() {
        signing.asked(code.map(device_code).as_deref());
    }
}

/// A computer, in through its onet already, or asking to be let in as the user signed in here, naming the computer's key.
#[cfg(desktop)]
async fn join_here() -> Result<Value, Failure> {
    let a = app();
    let address = a.address().ok_or("Enter your address first.")?;
    if here::ready(&address).await?["in"] == true {
        let_in_once();
        println!("Let in.");
        return Ok(json!({ "in": true }));
    }
    let asked = json!({ "address": address, "token": token().await?, "invite": a.kept("invite") });
    let asked = here::asked("/api/join/ask", Some(asked), 60).await?;
    let code = asked["code"].as_str().unwrap_or_default().to_string();
    println!("Code: {code}");
    Ok(json!({ "code": code, "invited": asked["invited"] }))
}

#[cfg(mobile)]
async fn join_here() -> Result<Value, Failure> {
    let a = app();
    let address = a.address().ok_or("Enter your address first.")?;
    let user = match a.kept("user") {
        Some(user) => user,
        None => {
            let user = subject(&token().await?)?;
            a.keep("user", Some(&user));
            user
        }
    };
    // One key for each user at each address, as the Mac and iPhone apps keep them, its folder named by it too.
    let id = format!("{address} {user}");
    let secret = secrets::device_made(&id)?;
    let mut made = false;
    let device = {
        let mut d = a.device.lock().unwrap();
        match d.as_ref() {
            Some(d) => d.clone(),
            None => {
                let new = Arc::new(client(&a.data, &address, &id, secret));
                if let Some(on) = platform::this_device() {
                    new.on(&on);
                }
                *d = Some(new.clone());
                made = true;
                new
            }
        }
    };
    match device.connect().await {
        Ok(true) => {
            let_in();
            // Only the join that made it: every page joins as it starts, the panel's own too.
            if made {
                redraw_panel();
            }
            println!("Let in.");
            return Ok(json!({ "in": true }));
        }
        Ok(false) => {}
        Err(e) if e.not_let_in => {}
        Err(e) => return Err(e.to_string().into()),
    }
    let invite = a.kept("invite");
    let token = token().await?;
    let invited = device.ask(&token, &device_name(), invite.as_deref()).await.map_err(|e| e.to_string())?;
    println!("Code: {}", device.code());
    Ok(json!({ "code": device.code(), "invited": invited }))
}

/// This device's own id here: `<address> <user>`, signed in. A computer's window has none: the computer's key is its onet's.
fn id_here() -> Result<String, Failure> {
    if cfg!(desktop) {
        return Err("This window has no key: this computer's is its onet's.".into());
    }
    let a = app();
    Ok(format!("{} {}", a.address().ok_or("Enter your address first.")?, a.kept("user").ok_or("Signed out.")?))
}

/// The key of the user signed in here, made the first time.
fn secret_here() -> Result<[u8; 32], Failure> {
    Ok(secrets::device_made(&id_here()?)?)
}

fn unbase64(s: &str) -> Result<Vec<u8>, Failure> {
    Ok(data_encoding::BASE64.decode(s.as_bytes()).map_err(|e| e.to_string())?)
}

/// What names this device: its key and seal key, hex. Its own client in the page signs and opens with them through the app.
#[tauri::command]
fn device_keys() -> Result<Value, Failure> {
    let join = onet::join_of(secret_here()?);
    Ok(json!({ "key": &join[..64], "seal": &join[64..] }))
}

/// This device's signature of `bytes` (base64), for its own client in the page, which never sees the key.
#[tauri::command]
fn device_sign(bytes: String) -> Result<String, Failure> {
    Ok(data_encoding::BASE64.encode(&key::sign_with(&secret_here()?, &unbase64(&bytes)?)))
}

/// What this device's seal key agrees with `share` (base64) on, for its own client in the page, which never sees the key.
#[tauri::command]
fn device_shared(share: String) -> Result<String, Failure> {
    let share: [u8; 32] = unbase64(&share)?.try_into().map_err(|_| "not a key")?;
    Ok(data_encoding::BASE64.encode(&key::agree_as(&secret_here()?, &share)))
}

/// The folder this device keeps what its keyring keeps (the same one its own client in Rust reads, so a sign-in carries over).
fn keep_dir() -> Result<PathBuf, Failure> {
    Ok(app().data.join("client").join(secrets::folder(&id_here()?)))
}

/// A name the keyring keeps something by: nothing that could reach outside its folder.
fn keep_name(name: &str) -> Result<&str, Failure> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) || name.starts_with('.') {
        return Err("That isn't a name to keep something by.".into());
    }
    Ok(name)
}

#[tauri::command]
fn keep_get(name: String) -> Result<Option<String>, Failure> {
    Ok(std::fs::read(keep_dir()?.join(keep_name(&name)?)).ok().map(|b| data_encoding::BASE64.encode(&b)))
}

#[tauri::command]
fn keep_put(name: String, bytes: String) -> Result<(), Failure> {
    let dir = keep_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{}.part", keep_name(&name)?));
    std::fs::write(&tmp, unbase64(&bytes)?).map_err(|e| e.to_string())?;
    Ok(std::fs::rename(&tmp, dir.join(keep_name(&name)?)).map_err(|e| e.to_string())?)
}

#[tauri::command]
fn keep_delete(name: String) -> Result<(), Failure> {
    let _ = std::fs::remove_file(keep_dir()?.join(keep_name(&name)?));
    Ok(())
}

/// An access token for its own client in the page, from the refresh token the app keeps.
#[tauri::command]
async fn access_token() -> Result<String, Failure> {
    token().await
}

/// The code its own client in the page is waiting to be approved by. The browser it signed in with goes on with it.
#[tauri::command]
fn waiting(code: String) {
    println!("Code: {code}");
    browser_goes_on(Some(&code));
}

/// The page's own client was let in: the app's own (Rust) client starts beside it, for what runs with no window (widgets,
/// notifications). It may be behind what the network reads; the page isn't held up by it. Its invite is used, so it goes. The
/// browser it signed in with, if it's held, is let go: this device needed no one's yes.
#[tauri::command]
fn joined() {
    app().keep("invite", None);
    browser_goes_on(None);
    #[cfg(mobile)]
    tauri::async_runtime::spawn(async {
        let a = app();
        let (Some(address), Some(user)) = (a.address(), a.kept("user")) else { return };
        let id = format!("{address} {user}");
        let Ok(secret) = secrets::device_made(&id) else { return };
        // Every page joins as it starts, the panel's own too: only the first starts the app's own client.
        let device = {
            let mut d = a.device.lock().unwrap();
            if d.is_some() {
                println!("Let in.");
                return;
            }
            let new = Arc::new(client(&a.data, &address, &id, secret));
            if let Some(on) = platform::this_device() {
                new.on(&on);
            }
            *d = Some(new.clone());
            new
        };
        println!("Let in.");
        match device.connect().await {
            Ok(_) => {
                let_in();
                redraw_panel();
            }
            Err(e) => {
                device_logs::say(&format!("this app's own client didn't start: {e}"));
                bundle::own_client_behind(&a.data, Some(&e.to_string()));
            }
        }
    });
}

/// Waits until this device is let in, as auth says it was approved and the relay lets it on.
#[tauri::command]
async fn wait() -> Result<(), Failure> {
    #[cfg(mobile)]
    let device = app().device()?;
    loop {
        let token = token().await?;
        #[cfg(mobile)]
        let let_in_now = device.wait(&token).await.map_err(|e| e.to_string())?;
        #[cfg(desktop)]
        let let_in_now = here::asked("/api/join/wait", Some(json!({ "token": token })), 60).await?["in"] == true;
        if let_in_now {
            app().keep("invite", None);
            // A computer's onet is in by its key: the window needs its sign-in no more.
            #[cfg(desktop)]
            if let Ok(signing) = app().sign_in() {
                let _ = tauri::async_runtime::spawn_blocking(move || signing.sign_out()).await;
            }
            let_in();
            redraw_panel();
            println!("Let in.");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// A request to the user's nodes → its status and headers, and an id its body is read by (`read`), part by part.
#[tauri::command]
async fn fetch(method: String, path: String, headers: Vec<(String, String)>, body: Option<String>) -> Result<Value, String> {
    let body = body.map(String::into_bytes).unwrap_or_default();
    let r = app().reach()?.fetch(&method, &path, headers, body).await?;
    let id = app().next.fetch_add(1, Ordering::Relaxed);
    app().bodies.lock().unwrap().insert(id, Arc::new(Reading { body: tokio::sync::Mutex::new(r.body), stop: tokio::sync::Notify::new() }));
    Ok(json!({ "id": id, "status": r.status, "headers": r.headers, "trace": r.trace.map(|t| t.to_string()) }))
}

/// The next part of a response's body; none once it's all read.
#[tauri::command]
async fn read(id: u64) -> Result<tauri::ipc::Response, String> {
    let reading = app().bodies.lock().unwrap().get(&id).cloned().ok_or("That response is gone.")?;
    let part = tokio::select! {
        part = async { reading.body.lock().await.chunk().await } => part?,
        _ = reading.stop.notified() => None,
    };
    if part.is_none() {
        app().bodies.lock().unwrap().remove(&id);
    }
    Ok(tauri::ipc::Response::new(part.unwrap_or_default()))
}

/// A response the page no longer reads.
#[tauri::command]
fn cancel(id: u64) {
    if let Some(reading) = app().bodies.lock().unwrap().remove(&id) {
        reading.stop.notify_one();
    }
}

/// What `app`'s `tool` answered last for `input` here, if it only reads (`Device::kept`): `{ answer, at }`, or null. A
/// computer's onet keeps them, for every window of it (`/api/kept`).
#[tauri::command]
async fn kept(app: String, tool: String, input: Value) -> Option<Value> {
    #[cfg(mobile)]
    {
        let (answer, at) = self::app().device().ok()?.kept(&app, &tool, input.to_string().as_bytes()).await?;
        Some(json!({ "answer": serde_json::from_slice::<Value>(&answer).ok()?, "at": at }))
    }
    #[cfg(desktop)]
    here::asked("/api/kept", Some(json!({ "app": app, "tool": tool, "input": input })), 5).await.ok().filter(|kept| !kept.is_null())
}

/// Renews its user's app schedules of `app`, which they just used (`Device::renew_app_schedules`). A computer's onet
/// renews them itself, as any of its programs calls the app.
#[tauri::command]
async fn renew(app: String) {
    #[cfg(mobile)]
    if let Ok(device) = self::app().device() {
        device.renew_app_schedules(&app).await;
    }
    #[cfg(desktop)]
    let _ = app;
}

/// Whether this device was taken out of its user's devices, as the relay says: a computer's onet says so itself.
#[tauri::command]
async fn removed() -> bool {
    #[cfg(mobile)]
    return match app().device() {
        Ok(d) => d.removed().await,
        Err(_) => false,
    };
    #[cfg(desktop)]
    false
}

/// Signs out: out of its user's devices first, then its key goes (a removed key can't be let in again), and the
/// sign-in with it. Taken out, as the relay says, only the sign-in goes: the relay's word takes no key, and signing in
/// again, where it still refuses this one, starts over (`start_over_if_taken_out`).
#[cfg(mobile)]
#[tauri::command]
async fn sign_out() -> Result<(), String> {
    let a = app();
    let device = a.device.lock().unwrap().take();
    let taken_out = tokio::time::timeout(Duration::from_secs(10), async {
        match &device {
            Some(d) => d.removed().await,
            None => {
                let (Some(address), Some(user)) = (a.address(), a.kept("user")) else { return false };
                let id = format!("{address} {user}");
                match secrets::device(&id) {
                    Some(secret) => client(&a.data, &address, &id, secret).removed().await,
                    None => false,
                }
            }
        }
    })
    .await
    .unwrap_or(false);
    if let Some(d) = device {
        if taken_out {
            d.disconnect().await;
        } else {
            let _ = tokio::time::timeout(Duration::from_secs(20), d.sign_out()).await;
        }
    }
    if let Ok(signing) = a.sign_in() {
        let _ = tauri::async_runtime::spawn_blocking(move || signing.sign_out()).await;
    }
    if !taken_out && let (Some(address), Some(user)) = (a.address(), a.kept("user")) {
        secrets::forget_device(&format!("{address} {user}"));
    }
    a.keep("user", None);
    widgets::forget(&platform::widgets_dir());
    platform::redraw_widgets();
    badge(0);
    Ok(())
}

/// A computer's window signs out of only its sign-in, which asking to be let in needed: the computer is in its network by
/// its own key, which only removing it there takes out. What the window drew goes.
#[cfg(desktop)]
#[tauri::command]
async fn sign_out() -> Result<(), String> {
    let a = app();
    if let Ok(signing) = a.sign_in() {
        let _ = tauri::async_runtime::spawn_blocking(move || signing.sign_out()).await;
    }
    a.keep("user", None);
    widgets::forget(&platform::widgets_dir());
    badge(0);
    Ok(())
}

/// Whether this device shows notifications, asking the user first if `ask` → "granted", "denied" or "default"; none
/// where a phone's build has no notifications to show (no Firebase project in its config.toml), so Settings has no row for it.
#[tauri::command]
async fn notify(ask: bool) -> Option<String> {
    platform::notify(ask).await
}

#[tauri::command]
fn copy(text: String) -> Result<(), String> {
    app().handle.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// A link a page can't show itself, in the system's browser or the app for it.
#[tauri::command]
fn open(url: String) -> Result<(), String> {
    if !["https://", "http://", "mailto:", "tel:"].iter().any(|p| url.starts_with(p)) {
        return Err("That isn’t a link this can open.".into());
    }
    platform::open(url)
}

/// A file a screen gives (`data`, base64): saved where the user chooses, or with `preview`, opened in the app for it.
#[tauri::command]
async fn save(name: String, data: String, preview: bool) -> Result<(), String> {
    let data = data_encoding::BASE64.decode(data.as_bytes()).map_err(|e| e.to_string())?;
    let name: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
    let name = if name.trim().is_empty() { "file".to_string() } else { name };
    platform::save(name, data, preview).await
}

/// What the page says went wrong (an error it didn't catch, a file that didn't load), for this device's log, which a user's
/// agent reads through a node (`one logs <device>`). The log's own caps and filters keep it small and free of what's secret.
#[tauri::command]
fn log(text: String) {
    device_logs::record(&format!("page: {text}"));
}

/// A phone's haptic: "selection" (a press), "medium" (a long press), "success" or "error"; elsewhere, nothing.
#[tauri::command]
fn haptic(style: String) {
    platform::haptic(style);
}

/// The document camera, where there's one (an iPhone's, or Android's scanner): each page found, straightened and cropped, as a JPEG
/// `{ name, type, size, base64 }`; none if the user cancels.
#[tauri::command]
async fn scan() -> Result<Value, String> {
    platform::scan().await
}

/// A screen's menu, while a touch and hold has lifted what it's for out of the page (an iPhone's) → `{ index }` (null if
/// none was chosen); else `{ lifted: false }`, and the page shows its own.
#[tauri::command]
async fn menu(items: Value) -> Value {
    platform::menu(items).await
}

/// Light, dark or automatic, as the user chose: the app starts in it from its first frame, and a phone's system bars
/// follow it.
#[tauri::command]
fn appearance(value: String) {
    platform::appearance(value);
}

// MARK: The menu bar panel

/// The window, in front, the panel gone, and at `link` if given (`<cli>://…`).
#[tauri::command]
fn main(link: Option<String>) {
    if let Some(p) = app().handle.get_webview_window("panel") {
        let _ = p.hide();
    }
    platform::show();
    if let Some(link) = link {
        opened(&link);
    }
}

/// Open at login, switched → whether it's on.
#[tauri::command]
fn login(on: bool) -> Option<bool> {
    platform::set_login(on);
    platform::login()
}

/// This computer's onet joins a network with a pairing code another of its user's computers made (as `<cli> join`
/// does) → its address, as the window keeps it.
#[cfg(desktop)]
#[tauri::command]
async fn pair(code: String) -> Result<String, String> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let said = tauri::async_runtime::spawn_blocking(move || here::ask("/api/join", Some(json!({ "code": code })), 60)).await.map_err(|e| e.to_string())?;
    // An answer from onet says why by its code; none came when its own answer didn't (it was too slow: the device that made the code is off).
    let failed = match said {
        Ok((200, _)) => None,
        Ok((_, out)) => Some(out["code"].as_str().unwrap_or_default().to_string()),
        Err(_) => Some(::custom_build::protocol::code::UNREACHABLE.to_string()),
    };
    if let Some(why) = failed {
        use ::custom_build::protocol::code;
        return Err(match why.as_str() {
            code::PAIRING_USED => "That pairing code was used or has expired. Make a new one on your other device.".into(),
            code::ALREADY_IN_NETWORK => format!("This device is already in another {}. Remove it there first.", ::custom_build::config::NAME.trim_start_matches("The ")),
            code::BAD_PAIRING_CODE => "That isn’t a whole pairing code. Copy it again, all of it.".into(),
            code::UNREACHABLE => "Couldn’t reach the device that made the code. Check both are online, then try again.".into(),
            _ => "Couldn’t join. Make a new pairing code on your other device and try again.".into(),
        });
    }
    // Its network's address, read once its keys are sealed to it, a moment after it's let in.
    let stands = tauri::async_runtime::spawn_blocking(here::stands).await.map_err(|e| e.to_string())?;
    if let Some(address) = stands.as_ref().and_then(|s| s["address"].as_str()) {
        app().keep("address", Some(address));
    }
    let_in_once();
    Ok(app().address().unwrap_or_default())
}

/// A phone joins with a sign-in: a pairing code is a computer's.
#[cfg(mobile)]
#[tauri::command]
async fn pair(code: String) -> Result<String, String> {
    let _ = code;
    Err("A phone joins by signing in.".into())
}

/// The window's empty top double-clicked (a Mac's): what System Settings says that does.
#[tauri::command]
fn title_bar_double_clicked(window: tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    platform::title_bar_double_clicked(&window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// The panel this tall, as its screen is (a Mac's menu bar is AppKit's, with no page to fit).
#[tauri::command]
fn fit(window: tauri::WebviewWindow, height: f64) {
    #[cfg(any(target_os = "linux", windows))]
    platform::fit(&window, height);
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = (window, height);
}

#[tauri::command]
fn quit() {
    app().handle.exit(0);
}

// MARK: Starting

/// Open at login from the first start, as the Mac app was; the user's choice after that. Never a test's build.
#[cfg(desktop)]
fn open_at_login_once(app: &tauri::App) -> tauri::Result<()> {
    let first = app.path().app_data_dir()?.join("opened-at-login");
    if !first.exists() && !identifier().ends_with(".throwaway") {
        platform::set_login(true);
        let _ = std::fs::create_dir_all(first.parent().unwrap());
        let _ = std::fs::write(&first, "");
    }
    Ok(())
}

/// A link the app was opened with (`<cli>://…`, or one to the network's address, which Windows hands it): to the page,
/// which goes there, or kept for it until it asks.
fn opened(url: &str) {
    let a = app();
    let own = url.starts_with(&format!("{}://", ::custom_build::config::CLI))
        || a.address().is_some_and(|address| url::Url::parse(url).is_ok_and(|u| u.origin().ascii_serialization() == address));
    if !own {
        return;
    }
    platform::show();
    match a.handle.get_webview_window("main").filter(|_| a.page.load(Ordering::Relaxed)) {
        Some(w) => drop(w.eval(format!("window.__oneNative?.opened({})", json!(url)))),
        None => *a.opened.lock().unwrap() = Some(url.to_string()),
    }
}

/// In a debug build, a loopback port an agent drives the window by (`CONTROL`): `POST /eval` runs its body in the
/// window's page (`/eval/<label>`, another window's); `POST /value` gives an expression's value, as JSON. Never in a build users use.
#[cfg(debug_assertions)]
fn control(port: u16) {
    use std::io::{BufRead, BufReader, Read, Write};
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) else { return eprintln!("control: port {port} is taken") };
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut r = BufReader::new(&s);
            let mut line = String::new();
            r.read_line(&mut line).ok();
            let mut len = 0;
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).ok();
            // `/eval` the window's page, `/eval/<label>` another's (the panel's).
            let label = line.split(' ').nth(1).and_then(|p| p.strip_prefix("/eval/").or(p.strip_prefix("/value/"))).unwrap_or("main").to_string();
            let out = match app().handle.get_webview_window(&label) {
                Some(w) if line.starts_with("POST /eval") => w.eval(String::from_utf8_lossy(&body)).map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()),
                // An expression's value, as JSON (what a promise gives, left in a global by /eval first, read back).
                Some(w) if line.starts_with("POST /value") => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let _ = w.eval_with_callback(String::from_utf8_lossy(&body), move |v| drop(tx.send(v)));
                    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "no answer".into())
                }
                _ => "POST /eval or /value, or /eval/<window> or /value/<window>".into(),
            };
            let mut s = &s;
            let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}", out.len());
        }
    });
}

/// Every command a page may ask the app.
fn commands() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    macro_rules! handler {
        ($($command:ident),*) => {
            tauri::generate_handler![$($command),*]
        };
    }
    commands!(handler)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    platform::plugins(tauri::Builder::default())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler({
            let commands = commands();
            move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
                if !platform::ours(&invoke) {
                    invoke.resolver.reject("That isn’t for a screen to ask.");
                    return true;
                }
                commands(invoke)
            }
        })
        .setup(|app| {
            let data = platform::data(app)?;
            device_logs::keep_in(&data, device_logs::CLIENT);
            let earlier = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                device_logs::say(&format!("panic: {info}"));
                earlier(info)
            }));
            device_logs::say(&format!("{} {} started on {}", ::custom_build::config::NAME, env!("CARGO_PKG_VERSION"), device_name()));
            #[cfg(debug_assertions)]
            if let Some(port) = env("CONTROL").and_then(|p| p.parse().ok()) {
                control(port);
            }
            let _ = APP.set(App {
                handle: app.handle().clone(),
                data,
                sign_in: Mutex::default(),
                #[cfg(mobile)]
                device: Mutex::default(),
                bodies: Mutex::default(),
                next: AtomicU64::new(1),
                opened: Mutex::default(),
                page: Default::default(),
            });
            bundle::start(&crate::app().data);
            bundle::watch(app.handle().clone());
            platform::start(app);
            platform::claim(crate::app().address());
            if let Some(address) = crate::app().address() {
                bundle::update(crate::app().data.clone(), address);
            }
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                for url in app.deep_link().get_current().ok().flatten().unwrap_or_default() {
                    opened(url.as_str());
                }
                app.deep_link().on_open_url(|e| e.urls().iter().for_each(|u| opened(u.as_str())));
            }
            #[cfg(desktop)]
            let shown = std::sync::atomic::AtomicBool::new(false);
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(include_str!("native.js"))
                .use_https_scheme(true)
                .on_page_load(move |#[cfg_attr(mobile, allow(unused_variables))] window, load| {
                    #[cfg(target_os = "macos")]
                    if let tauri::webview::PageLoadEvent::Finished = load.event() {
                        let _ = window.eval(platform::title_bar_area(window.is_fullscreen().unwrap_or(false)));
                    }
                    // Once, as its page starts: where it was left, which the window-state plugin has moved it to by now.
                    #[cfg(desktop)]
                    if matches!(load.event(), tauri::webview::PageLoadEvent::Started) && !shown.swap(true, Ordering::Relaxed) {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    let event = match load.event() {
                        tauri::webview::PageLoadEvent::Started => "started",
                        tauri::webview::PageLoadEvent::Finished => "finished",
                    };
                    device_logs::record(&format!("page {event} loading {}", load.url()));
                })
                // Anything that would take the page away goes to the browser.
                .on_navigation(|u| {
                    let own = matches!(u.scheme(), "tauri") || u.host_str() == Some("tauri.localhost");
                    if !own {
                        let _ = open(u.to_string());
                    }
                    own
                });
            #[cfg(desktop)]
            let window = window.title(::custom_build::config::NAME).inner_size(1000.0, 700.0).min_inner_size(360.0, 480.0).visible(false);
            // The page runs up to the window's top, under its buttons, as the Mac's own apps do.
            #[cfg(target_os = "macos")]
            let window = window
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .traffic_light_position(tauri::LogicalPosition::new(platform::LIGHTS_AT.0, platform::LIGHTS_AT.1))
                .initialization_script(platform::title_bar_area(false))
                .allow_link_preview(false);
            #[cfg_attr(mobile, allow(unused_variables))]
            let window = window.build()?;
            #[cfg(target_os = "macos")]
            platform::watch_full_screen(&window);
            #[cfg(desktop)]
            platform::appear(&window);
            // On a computer, closing the window leaves the app running, in the tray or menu bar, where Open shows it
            // again (or starting it again does); Quit quits.
            #[cfg(desktop)]
            {
                let hide = window.clone();
                window.on_window_event(move |e| match e {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        platform::keep_place();
                        let _ = hide.hide();
                    }
                    // Automatic follows the system: its title bar and first colour too, as it changes while the app runs.
                    tauri::WindowEvent::ThemeChanged(_) => platform::appear(&hide),
                    _ => {}
                });
                open_at_login_once(app)?;
                platform::start_tray(app)?;
            }
            Ok(())
        })
        .build(bundle::serving(tauri::generate_context!()))
        .expect("the app")
        .run(|handle, event| {
            #[cfg(mobile)]
            if let tauri::RunEvent::Resumed = event {
                bundle::reopened(handle);
            }
            #[cfg(desktop)]
            let _ = (handle, event);
        });
}
