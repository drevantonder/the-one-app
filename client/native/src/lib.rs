//! The native app: one window, which is the web client's own page (cloudflare/web/web.js, its files from one::web,
//! built in), drawn and run as it is in a browser, but on this device's client (one_client) rather than a browser's.
//! The page finds the app as `window.__oneNative` (src/native.js), which only it has: screens run in sandboxed frames
//! in it with no origin of their own, which reach none of the app's commands, and ask the page, as they do in a
//! browser. What the app adds is what a page can't do: this device's key in the OS's secret store, signing in through
//! the system's browser, notifications, the clipboard, files, and links outside.

#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
mod headless;
#[cfg(target_os = "ios")]
mod iphone;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod menubar;
mod secrets;
mod signin;
mod widgets;
#[cfg(windows)]
mod windows;

pub use headless::answer as headless;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
#[cfg(desktop)]
use tauri_plugin_dialog::DialogExt;
#[cfg(desktop)]
use tauri_plugin_notification::{NotificationExt, PermissionState};
#[cfg(not(target_os = "android"))]
use tauri_plugin_opener::OpenerExt;

/// What a call says it came from: the platform, an iPhone as "phone", as the iPhone app's did.
fn client_name() -> String {
    std::env::consts::OS.replace("macos", "mac").replace("ios", "phone")
}

/// The app's id, as build.rs names it for this platform: `app.<cli>.<platform>` (a throwaway's, `….throwaway`).
fn identifier() -> &'static str {
    env!("IDENTIFIER")
}

/// What the person calls this device, as their other devices list it.
fn device_name() -> String {
    // A Mac, as it's named in its settings ("Andre's MacBook Air"), as the Mac app named it.
    #[cfg(target_os = "macos")]
    if let Some(name) = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|n| !n.is_empty()) {
        return name;
    }
    #[cfg(target_os = "ios")]
    if let Some(name) = iphone::name() {
        return name;
    }
    #[cfg(target_os = "android")]
    if let Some(name) = android::name() {
        return name;
    }
    // Windows names it in COMPUTERNAME.
    let host = std::env::var("HOSTNAME").ok().or_else(|| std::env::var("COMPUTERNAME").ok()).or_else(|| std::fs::read_to_string("/etc/hostname").ok()).map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    host.unwrap_or_else(|| platform().into())
}

/// The platform, as people say it (in a debug build, `PLATFORM` for a test: a phone's look on a computer).
fn platform() -> &'static str {
    #[cfg(debug_assertions)]
    if let Some(p) = env("PLATFORM") {
        return if p == "iPhone" { "iPhone" } else if p == "Android" { "Android" } else { "Linux" };
    }
    match std::env::consts::OS {
        "macos" => "Mac",
        "windows" => "Windows",
        "ios" => "iPhone",
        "android" => "Android",
        _ => "Linux",
    }
}

/// A setting for a test from its environment, named by the CLI's name (`ONE_ADDRESS`): `ADDRESS`, the network's;
/// `SIGN_IN_PRINT`, sign in by printing the link rather than opening a browser (`one throwaway client linux` signs in
/// there); `CONTROL`, in a debug build, a loopback port an agent drives the window by.
fn env(name: &str) -> Option<String> {
    std::env::var(format!("{}_{name}", one::config::CLI.to_uppercase())).ok().filter(|v| !v.is_empty())
}

/// The device this app is on, which signing in again replaces its sign-in under: the computer's own node's key, where
/// one runs here (as the Mac app's was); else the machine's own id, hashed: Linux's machine-id, or Windows' MachineGuid,
/// which outlives reinstalling Windows' apps (as the Windows app's was); on an iPhone, an id of its own in its Keychain,
/// which outlives signing out (as the iPhone app's was); on an Android phone, its Android ID for this app.
fn this_device() -> Option<String> {
    #[cfg(not(target_os = "android"))]
    use sha2::{Digest, Sha256};
    #[cfg(target_os = "ios")]
    return secrets::this_device().ok().map(|id| data_encoding::HEXLOWER.encode(&Sha256::digest(id)));
    // An Android phone's own id, which outlives reinstalling (as the Kotlin app's was).
    #[cfg(target_os = "android")]
    return android::this_device();
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    local_node().or_else(|| {
        #[cfg(windows)]
        let id = windows::machine_guid()?;
        #[cfg(not(windows))]
        let id = ["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|p| std::fs::read_to_string(p).ok())?;
        Some(data_encoding::HEXLOWER.encode(&Sha256::digest(format!("this device {}", id.trim()).as_bytes())))
    })
}

/// This computer's own node's key, where one runs here.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn local_node() -> Option<String> {
    local_state()?["device"]["key"].as_str().map(str::to_string)
}

/// What this computer's own node says of itself, where one runs here.
fn local_state() -> Option<Value> {
    local("/v1/state", None, 2).ok().filter(|(ok, _)| *ok).map(|(_, out)| out)
}

/// Asks this computer's own node as one of its programs (by its local token): `body` posted, or a GET, waiting at
/// most `secs` → whether it did, and what it said.
fn local(path: &str, body: Option<Value>, secs: u64) -> Result<(bool, Value), String> {
    let token = std::fs::read_to_string(one::home().join("local-token")).map_err(|_| "This computer's node isn't running.".to_string())?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(secs))).http_status_as_error(false).build().into();
    let url = format!("http://127.0.0.1:{}{path}", one::config::PORT);
    let mut r = match body {
        Some(body) => agent.post(url).header("x-one-local-token", token.trim()).send_json(body),
        None => agent.get(url).header("x-one-local-token", token.trim()).call(),
    }
    .map_err(|e| e.to_string())?;
    Ok((r.status().is_success(), r.body_mut().read_json().unwrap_or_default()))
}

/// A response's body the page reads, and what stops a read waiting on it once the page cancels.
struct Reading {
    body: tokio::sync::Mutex<one_client::Body>,
    stop: tokio::sync::Notify,
}

struct App {
    handle: AppHandle,
    /// Where it keeps what isn't secret: the address, who's signed in, and the client's own folder per person.
    data: PathBuf,
    auth: Mutex<Option<Arc<signin::Auth>>>,
    device: Mutex<Option<Arc<one_client::Device>>>,
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

impl App {
    fn kept(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.data.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
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

    /// The network's address: `ADDRESS` for a test, the custom build's if it has one, else as the person gave it.
    fn address(&self) -> Option<String> {
        env("ADDRESS").or_else(|| Some(one::config::ADDRESS.to_string()).filter(|a| !a.is_empty())).or_else(|| self.kept("address")).map(|a| a.trim_end_matches('/').to_string())
    }

    fn auth(&self) -> Result<Arc<signin::Auth>, String> {
        let address = self.address().ok_or("Enter your address first.")?;
        let mut auth = self.auth.lock().unwrap();
        if let Some(a) = auth.as_ref().filter(|a| a.address == address) {
            return Ok(a.clone());
        }
        let a = Arc::new(signin::Auth::new(&address, &format!("{} on {}", one::config::NAME, device_name()), self.data.clone()));
        *auth = Some(a.clone());
        Ok(a)
    }

    fn device(&self) -> Result<Arc<one_client::Device>, String> {
        self.device.lock().unwrap().clone().ok_or_else(|| "This device hasn't been approved yet.".into())
    }
}

/// What a node answers at `path`, as JSON.
async fn node_json(device: &one_client::Device, path: &str) -> Result<Value, String> {
    let r = device.fetch("GET", path, Vec::new(), Vec::new()).await.map_err(|e| e.to_string())?;
    let status = r.status;
    let body = r.body.bytes().await.map_err(|e| e.to_string())?;
    let out: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    if !(200..300).contains(&status) {
        return Err(out["error"].as_str().unwrap_or("That didn’t work.").to_string());
    }
    Ok(out)
}

/// Where the platform's own widgets read the pages' widgets (`widgets`): on an iPhone, the App Group they share.
fn widgets_dir() -> PathBuf {
    #[cfg(target_os = "ios")]
    if let Some(group) = iphone::group() {
        return group;
    }
    app().data.join("widgets")
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

/// Let in, now or as it starts: the platform's own widgets kept, and its pushes registered: on Windows its channel (by
/// the package's helper, which has the Windows App SDK), since a channel lasts 30 days; on an iPhone or Android phone its
/// token.
fn let_in() {
    keep_widgets();
    #[cfg(target_os = "ios")]
    if let Ok(device) = app().device() {
        iphone::push(device);
    }
    // An Android phone asks the person, once, whether it may show notifications (in a build with pushes).
    #[cfg(target_os = "android")]
    if let Ok(device) = app().device() {
        android::push(device);
        android::tell("notify", json!({ "ask": true, "once": true }));
    }
    // The panel, drawn before this device was in (signed out), draws again.
    if let Some(p) = app().handle.get_webview_window("panel") {
        let _ = p.eval("location.reload()");
    }
    #[cfg(windows)]
    windows::register_push();
}

/// Something changed on the person's devices (the page's events say): the widgets kept for the platform's own again.
#[tauri::command]
fn changed() {
    let Ok(device) = app().device() else { return };
    tauri::async_runtime::spawn(async move {
        if let Err(e) = widgets::refresh(&device, &widgets_dir()).await {
            eprintln!("widgets: {e}");
        }
        redraw_widgets();
    });
}

/// The platform's own widgets drawn again from what's kept for them: a Mac's menu bar items, an iPhone's or Android
/// phone's home-screen widgets.
fn redraw_widgets() {
    #[cfg(target_os = "macos")]
    menubar::update();
    #[cfg(target_os = "ios")]
    iphone::tell("reload", json!({}));
    #[cfg(target_os = "android")]
    let _ = android::host("reload", json!({}));
}

/// What waits for the person (every app's badge on every page, and Settings'), counted on the app's icon: Windows'
/// taskbar button (BadgeUpdateManager, by the package's identity), elsewhere what Tauri sets (the dock's, the
/// launcher's); none at 0.
#[tauri::command]
fn badge(count: u32) {
    #[cfg(windows)]
    windows::badge(count);
    #[cfg(all(desktop, not(windows)))]
    if let Some(w) = app().handle.get_webview_window("main") {
        let _ = w.set_badge_count((count > 0).then_some(count as i64));
    }
    #[cfg(target_os = "ios")]
    iphone::tell("badge", json!({ "count": count }));
    // An Android launcher counts what notifications it shows, not what the app says.
    #[cfg(target_os = "android")]
    let _ = count;
}

/// An access token, from the refresh token in the secret store (a request to sign-in, so off the async threads).
async fn token() -> Result<String, String> {
    let auth = app().auth()?;
    tauri::async_runtime::spawn_blocking(move || auth.token()).await.map_err(|e| e.to_string())?
}

// MARK: What the page asks

/// Who and where this is: the custom build's name and CLI, the address (if one's given), whether someone's signed in
/// here and who.
#[tauri::command]
fn native() -> Value {
    let a = app();
    let address = a.address();
    let signed_in = address.as_ref().is_some_and(|_| a.auth().is_ok_and(|auth| auth.signed_in()));
    a.page.store(true, Ordering::Relaxed);
    let opened = a.opened.lock().unwrap().take();
    let local = local_state();
    let unpaired = cfg!(target_os = "macos") && local.as_ref().is_some_and(|s| s["network"].is_null());
    #[cfg(target_os = "macos")]
    mac::menu_now(signed_in, unpaired);
    json!({
        "name": one::config::NAME, "yours": one::config::YOURS, "cli": one::config::CLI, "platform": platform(), "device": device_name(), "client": client_name(),
        "address": address, "signedIn": signed_in, "user": if signed_in { a.kept("user") } else { None },
        "opened": opened,
        // This computer's own node, where one runs here: "This device" in the panel.
        "node": local.as_ref().and_then(|s| s["device"]["key"].as_str().map(str::to_string)),
        // A Mac whose own node is in no network yet joins one with a pairing code (`pair`), as the Mac app did.
        "unpaired": unpaired,
        // Open at login: whether it's on, where the app switches it (none where it can't yet).
        "login": login_now(&a.handle),
        // Let Agents Use This Mac, on a Mac (Computer Use's helper reads it).
        "computerUse": computer_use_now(),
        // A Mac's widgets hidden from the menu bar (their own items), which its panel shows again.
        "menubar": menubar_hidden(),
        // An iPhone's: the document camera, and touch and hold lifting what's held with its menu (`menu`).
        "scan": cfg!(target_os = "ios") || can_scan(),
        "lifts": cfg!(target_os = "ios"),
        // How much bigger than usual the person has made text (a phone's: Android's font size, an iPhone's Dynamic Type), for the
        // window to be zoomed by.
        "textScale": text_scale(),
    })
}

fn can_scan() -> bool {
    #[cfg(target_os = "android")]
    return android::can_scan();
    #[cfg(not(target_os = "android"))]
    false
}

fn text_scale() -> f64 {
    #[cfg(target_os = "android")]
    return android::text_scale();
    #[cfg(target_os = "ios")]
    return iphone::text_scale();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    1.0
}

/// Links to the network's address open this app (Windows' app links), as of the address it signs in to.
fn claim() {
    #[cfg(windows)]
    {
        let address = app().address();
        std::thread::spawn(move || windows::claim(address.as_deref()));
    }
}

/// Takes what the person typed: an address, or an invite link (`<address>/invite#…`), whose secret goes with this
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
    claim();
    Ok(origin)
}

/// Signs in, in the system's browser, which comes back to this device; naming the device it's for, so the person's
/// devices can let it in by this sign-in.
#[tauri::command]
async fn sign_in() -> Result<(), String> {
    let a = app();
    a.address().ok_or("Enter your address first.")?;
    let join = one_client::join_of(secrets::signing_in()?);
    let auth = a.auth()?;
    #[cfg(not(target_os = "android"))]
    let handle = a.handle.clone();
    let print = env("SIGN_IN_PRINT").is_some();
    tauri::async_runtime::spawn_blocking(move || {
        // An iPhone signs in in the system's sheet, which comes back to the app's own link; a test's prints the link.
        #[cfg(target_os = "ios")]
        if !print {
            return auth.sign_in_here(&join, iphone::sign_in);
        }
        // An Android phone signs in in a Chrome Custom Tab, which comes back to the app's own link; a test's prints the
        // link and waits for it to be opened.
        #[cfg(target_os = "android")]
        return auth.sign_in_here(&join, |u| {
            if print {
                println!("Sign in: {u}");
            }
            android::sign_in(u, !print)
        });
        #[cfg(not(target_os = "android"))]
        auth.sign_in(&join, |u| {
            if print {
                println!("Sign in: {u}");
                return Ok(());
            }
            handle.opener().open_url(u, None::<&str>).map_err(|e| e.to_string())
        })
    })
        .await
        .map_err(|e| e.to_string())??;
    let token = token().await?;
    a.keep("user", Some(&signin::Auth::subject(&token)));
    // Back in front of the browser it signed in with.
    #[cfg(desktop)]
    if let Some(w) = a.handle.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    Ok(())
}

/// Makes this device its person's: one already, or asking to be let in. → `{ in: true }`, or the code to approve
/// it by (`{ code }`), after which `wait`.
#[tauri::command]
async fn join() -> Result<Value, String> {
    let a = app();
    let address = a.address().ok_or("Enter your address first.")?;
    let token = token().await?;
    let user = signin::Auth::subject(&token);
    a.keep("user", Some(&user));
    // One key for each person at each address, as the Mac and iPhone apps keep them, its folder named by it too.
    let id = format!("{address} {user}");
    let secret = secrets::device_made(&id)?;
    let device = {
        let mut d = a.device.lock().unwrap();
        match d.as_ref() {
            Some(d) => d.clone(),
            None => {
                let dir = a.data.join("client").join(secrets::folder(&id));
                let made = Arc::new(one_client::Device::new(secret, &address, &dir.to_string_lossy()).keeping());
                if let Some(on) = this_device() {
                    made.on(&on);
                }
                *d = Some(made.clone());
                made
            }
        }
    };
    match device.connect().await {
        Ok(true) => {
            let_in();
            println!("Let in.");
            return Ok(json!({ "in": true }));
        }
        Ok(false) => {}
        Err(e) if e.not_let_in => {}
        Err(e) => return Err(e.to_string()),
    }
    let invite = a.kept("invite");
    // A computer is "<name> on <computer>", as the Mac app's was; a phone, its own name.
    let name = if cfg!(desktop) { format!("{} on {}", one::config::NAME, device_name()) } else { device_name() };
    device.ask(&token, &name, invite.as_deref()).await.map_err(|e| e.to_string())?;
    println!("Code: {}", device.code());
    approve_here(device.code());
    Ok(json!({ "code": device.code(), "invited": invite.is_some() }))
}

/// Waits until this device is let in, as sign-in says it was approved and the relay lets it on.
#[tauri::command]
async fn wait() -> Result<(), String> {
    let device = app().device()?;
    loop {
        let token = token().await?;
        if device.wait(&token).await.map_err(|e| e.to_string())? {
            app().keep("invite", None);
            let_in();
            println!("Let in.");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// A request to the person's nodes → its status and headers, and an id its body is read by (`read`), part by part.
#[tauri::command]
async fn fetch(method: String, path: String, headers: Vec<(String, String)>, body: Option<String>) -> Result<Value, String> {
    let device = app().device()?;
    let body = body.map(String::into_bytes).unwrap_or_default();
    let r = device.fetch(&method, &path, headers, body).await.map_err(|e| if e.not_let_in { "not_let_in".to_string() } else { e.to_string() })?;
    let id = app().next.fetch_add(1, Ordering::Relaxed);
    app().bodies.lock().unwrap().insert(id, Arc::new(Reading { body: tokio::sync::Mutex::new(r.body), stop: tokio::sync::Notify::new() }));
    Ok(json!({ "id": id, "status": r.status, "headers": r.headers, "trace": r.trace.map(|t| t.to_string()) }))
}

/// The next part of a response's body; none once it's all read.
#[tauri::command]
async fn read(id: u64) -> Result<tauri::ipc::Response, String> {
    let reading = app().bodies.lock().unwrap().get(&id).cloned().ok_or("That response is gone.")?;
    let part = tokio::select! {
        part = async { reading.body.lock().await.chunk().await } => part.map_err(|e| e.to_string())?,
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

/// Whether this device was taken out of its person's devices, as the relay says.
#[tauri::command]
async fn removed() -> bool {
    match app().device() {
        Ok(d) => d.removed().await,
        Err(_) => false,
    }
}

/// Signs out: out of its person's devices first, then its key goes (a removed key can't be let in again), and the
/// sign-in with it.
#[tauri::command]
async fn sign_out() -> Result<(), String> {
    let a = app();
    let device = a.device.lock().unwrap().take();
    if let Some(d) = device {
        let _ = tokio::time::timeout(Duration::from_secs(20), d.sign_out()).await;
    }
    if let Ok(auth) = a.auth() {
        let _ = tauri::async_runtime::spawn_blocking(move || auth.sign_out()).await;
    }
    if let (Some(address), Some(user)) = (a.address(), a.kept("user")) {
        secrets::forget_device(&format!("{address} {user}"));
    }
    a.keep("user", None);
    widgets::forget(&widgets_dir());
    redraw_widgets();
    badge(0);
    Ok(())
}

/// Whether this device shows notifications, asking the person first if `ask` → "granted", "denied" or "default"; none
/// where a phone's build has no pushes to show (no Firebase project in its config.toml), so Settings has no row for it.
#[tauri::command]
async fn notify(ask: bool) -> Option<String> {
    #[cfg(target_os = "ios")]
    return Some(iphone::ask_off("notify", json!({ "ask": ask })).await.ok().and_then(|v| v["state"].as_str().map(str::to_string)).unwrap_or_else(|| "default".into()));
    #[cfg(target_os = "android")]
    return android::ask_off("notify", json!({ "ask": ask })).await.ok().and_then(|v| v["state"].as_str().map(str::to_string));
    #[cfg(desktop)]
    {
        let n = app().handle.notification();
        let state = match n.permission_state() {
            Ok(PermissionState::Granted) => PermissionState::Granted,
            Ok(s) if !ask => s,
            _ => n.request_permission().unwrap_or(PermissionState::Denied),
        };
        Some(match state {
            PermissionState::Granted => "granted".into(),
            PermissionState::Denied => "denied".into(),
            _ => "default".into(),
        })
    }
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
    // On Android, the network's own pages open in a Custom Tab, as signed in there as the browser is.
    #[cfg(target_os = "android")]
    return android::ask("open", json!({ "url": url, "own": app().address().is_some_and(|a| url.starts_with(&a)) })).map(drop).map_err(|_| "Couldn’t open your browser.".to_string());
    #[cfg(not(target_os = "android"))]
    app().handle.opener().open_url(url, None::<&str>).map_err(|_| "Couldn’t open your browser.".to_string())
}

/// A file a screen gives (`data`, base64): saved where the person chooses, or with `preview`, opened in the app for it.
#[tauri::command]
async fn save(name: String, data: String, preview: bool) -> Result<(), String> {
    let data = data_encoding::BASE64.decode(data.as_bytes()).map_err(|e| e.to_string())?;
    let name: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
    let name = if name.trim().is_empty() { "file".to_string() } else { name };
    // A phone shares it (the share sheet, which saves to Files too), or shows it (Quick Look; what views its type).
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        // An Android phone's, in its own folder, where its share sheet is let read (android/.../res/xml/files.xml).
        #[cfg(target_os = "android")]
        let base = app().data.join("given");
        #[cfg(not(target_os = "android"))]
        let base = std::env::temp_dir().join("screen-files");
        let dir = base.join(app().next.fetch_add(1, Ordering::Relaxed).to_string());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(&name);
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        #[cfg(target_os = "ios")]
        return iphone::ask_off("give", json!({ "path": path, "preview": preview })).await.map(drop);
        #[cfg(target_os = "android")]
        return android::ask_off("give", json!({ "path": path, "preview": preview })).await.map(drop);
    }
    #[cfg(desktop)]
    if preview {
        let dir = std::env::temp_dir().join(format!("{}-{}", one::config::CLI, app().next.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(&name);
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        return app().handle.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string());
    }
    #[cfg(desktop)]
    let handle = app().handle.clone();
    #[cfg(desktop)]
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = handle.dialog().file().set_file_name(&name).blocking_save_file() else { return Ok(()) };
        let path = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(path, data).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A phone's haptic: "selection" (a press), "medium" (a long press), "success" or "error"; elsewhere, nothing.
#[tauri::command]
fn haptic(style: String) {
    #[cfg(target_os = "ios")]
    iphone::tell("haptic", json!({ "style": style }));
    #[cfg(target_os = "android")]
    android::tell("haptic", json!({ "style": style }));
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let _ = style;
}

/// The document camera, where there's one (an iPhone's, or Android's scanner): each page found, straightened and cropped, as a JPEG
/// `{ name, type, size, base64 }`; none if the person cancels.
#[tauri::command]
async fn scan() -> Result<Value, String> {
    #[cfg(target_os = "ios")]
    return iphone::ask_off("scan", json!({})).await.map(|v| v["files"].clone());
    #[cfg(target_os = "android")]
    return android::ask_off("scan", json!({})).await.map(|v| v["files"].clone());
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    Err("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.".into())
}

/// A screen's menu while a touch and hold has lifted what it's for out of the page (an iPhone's): shown with it, as the
/// Home Screen's → `{ index }` (null if none was chosen); else `{ lifted: false }`, and the page shows its own.
#[tauri::command]
async fn menu(items: Value) -> Value {
    #[cfg(target_os = "ios")]
    return iphone::ask_off("menu", json!({ "items": items })).await.unwrap_or_else(|_| json!({ "lifted": false }));
    #[cfg(not(target_os = "ios"))]
    {
        let _ = items;
        json!({ "lifted": false })
    }
}

/// Light, dark or automatic, as the person chose: kept by the app too, so its next start draws in it from its first
/// frame, and a phone's system bars follow it.
#[tauri::command]
fn appearance(value: String) {
    #[cfg(target_os = "ios")]
    iphone::tell("appearance", json!({ "value": value }));
    #[cfg(target_os = "android")]
    android::tell("appearance", json!({ "value": value }));
    #[cfg(desktop)]
    {
        app().keep("appearance", Some(&value).filter(|v| *v != "automatic").map(String::as_str));
        app().handle.webview_windows().values().for_each(appear);
    }
}

/// A window in the person's light or dark (the system's, for automatic): its title bar, and the colour it shows before
/// its page draws, so a start in dark has no white frame.
#[cfg(desktop)]
fn appear(window: &tauri::WebviewWindow) {
    let theme = match app().kept("appearance").as_deref() {
        Some("dark") => Some(tauri::Theme::Dark),
        Some("light") => Some(tauri::Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(theme);
    // @one/ui's --background.
    let dark = theme.or_else(|| window.theme().ok()) == Some(tauri::Theme::Dark);
    let _ = window.set_background_color(Some(if dark { tauri::window::Color(10, 10, 10, 255) } else { tauri::window::Color(255, 255, 255, 255) }));
}

/// A widget's own menu bar item shown again, or hidden (a Mac's) → the kinds hidden now.
#[tauri::command]
fn menubar(kind: String, show: bool) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        menubar::set_hidden(&kind, !show);
        tray_menu(&app().handle);
        return menubar::hidden();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (kind, show);
        Vec::new()
    }
}

// MARK: The menu bar panel

#[cfg(desktop)]
fn login_now(app: &AppHandle) -> Option<bool> {
    Some(login::on(app))
}

#[cfg(not(desktop))]
fn login_now(_: &AppHandle) -> Option<bool> {
    None
}

/// The window, in front, the panel gone, and at `link` if given (`<cli>://…`).
#[tauri::command]
fn main(link: Option<String>) {
    if let Some(p) = app().handle.get_webview_window("panel") {
        let _ = p.hide();
    }
    show();
    if let Some(link) = link {
        opened(&link);
    }
}

/// Open at login, switched → whether it's on.
#[tauri::command]
fn login(on: bool) -> Option<bool> {
    #[cfg(desktop)]
    login::set(&app().handle, on);
    let _ = on;
    login_now(&app().handle)
}

fn menubar_hidden() -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    return Some(menubar::hidden());
    #[cfg(not(target_os = "macos"))]
    None
}

fn computer_use_now() -> Option<bool> {
    #[cfg(target_os = "macos")]
    return Some(mac::computer_use());
    #[cfg(not(target_os = "macos"))]
    None
}

/// This computer's own node joins a network with a pairing code another of its person's computers made (as `<cli>
/// join` does) → its address, which the app signs in to next.
#[tauri::command]
async fn pair(code: String) -> Result<String, String> {
    if !one::home().join("local-token").exists() {
        return Err("This Mac's node isn't running.".into());
    }
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let said = tauri::async_runtime::spawn_blocking(move || local("/v1/join", Some(json!({ "code": code })), 60)).await.map_err(|e| e.to_string())?;
    let failed = match said {
        Ok((true, _)) => None,
        Ok((false, out)) => Some(out["error"].as_str().unwrap_or_default().to_lowercase()),
        Err(e) => Some(e.to_lowercase()),
    };
    if let Some(e) = failed {
        let has = |words: &[&str]| words.iter().any(|w| e.contains(w));
        return Err(if has(&["already used", "expired", "refused"]) {
            "That pairing code was used or has expired. Make a new one on your other device.".into()
        } else if has(&["already belongs"]) {
            format!("This device is already in another {}. Remove it there first.", one::config::NAME.trim_start_matches("The "))
        } else if has(&["decode", "invalid", "base", "parse", "bad"]) {
            "That isn’t a whole pairing code. Copy it again, all of it.".into()
        } else if has(&["timed out", "timeout", "connect", "unreachable", "no route", "network"]) {
            "Couldn’t reach the device that made the code. Check both are online, then try again.".into()
        } else {
            "Couldn’t join. Make a new pairing code on your other device and try again.".into()
        });
    }
    // Its network's address, which the person signs in to here.
    let address = local_state().and_then(|s| s["network"]["settings"]["address"].as_str().map(str::to_string)).ok_or("Joined. Now type your network’s address.")?;
    address_of(&address)
}

/// Let Agents Use This Mac, switched → whether it's on.
#[tauri::command]
fn computer_use(on: bool) -> Option<bool> {
    #[cfg(target_os = "macos")]
    mac::set_computer_use(on);
    let _ = on;
    #[cfg(desktop)]
    tray_menu(&app().handle);
    computer_use_now()
}

/// On a Mac, its own node approves the window by its code, as one of the Mac's own programs, if it's a device of the
/// same person (as the Mac app's did): else the code waits for another of theirs.
fn approve_here(code: String) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    std::thread::spawn(move || local("/v1/devices/approve", Some(json!({ "code": code, "confirm": true })), 30));
}

/// The panel this tall, as its screen is: under its item it grows down, over one (a taskbar) up.
#[tauri::command]
fn fit(window: tauri::WebviewWindow, height: f64) {
    #[cfg(desktop)]
    if ["panel", "widget"].contains(&window.label()) {
        let p = window;
        let height = height.clamp(160.0, 900.0);
        let scale = p.scale_factor().unwrap_or(1.0);
        let was = p.outer_size().map(|s| s.height as f64).unwrap_or(PANEL.1 * scale);
        let _ = p.set_size(tauri::LogicalSize::new(PANEL.0, height));
        if let (Ok(at), Some(m)) = (p.outer_position(), p.current_monitor().ok().flatten())
            && (at.y as f64) > m.position().y as f64 + m.size().height as f64 / 2.0
        {
            let _ = p.set_position(tauri::PhysicalPosition::new(at.x, at.y + (was - height * scale) as i32));
        }
    }
    #[cfg(not(desktop))]
    let _ = (window, height);
}

#[tauri::command]
fn quit() {
    app().handle.exit(0);
}

/// The panel, shown under the menu bar or tray item at `at` (its place and size on the screen, physical), or hidden if
/// it shows; with none, at the top right.
#[tauri::command]
fn panel() {
    toggle_panel(None);
}

/// The panel's size, as the Mac app's was wide.
#[cfg(desktop)]
const PANEL: (f64, f64) = (340.0, 560.0);

#[cfg(desktop)]
fn toggle_panel(at: Option<tauri::Rect>) {
    if let Some(p) = app().handle.get_webview_window("panel") {
        toggle_under(&p, at);
    }
}

/// A window like the panel's, shown under the menu bar or tray item at `at` (its place and size on the screen,
/// physical), or hidden if it shows; with none, at the top right.
#[cfg(desktop)]
fn toggle_under(p: &tauri::WebviewWindow, at: Option<tauri::Rect>) {
    if p.is_visible().unwrap_or(false) {
        let _ = p.hide();
        return;
    }
    // Its size as made: a window never shown yet says none.
    let scale = p.scale_factor().unwrap_or(1.0);
    let size = tauri::PhysicalSize::new((PANEL.0 * scale) as u32, (PANEL.1 * scale) as u32);
    if let Some(at) = at {
        let (pos, item) = (at.position.to_physical::<f64>(scale), at.size.to_physical::<f64>(scale));
        let x = pos.x + item.width / 2.0 - size.width as f64 / 2.0;
        // Under an item at the top of the screen (a Mac's menu bar), over one at the bottom (a Windows taskbar).
        let top = p.current_monitor().ok().flatten().map(|m| (m.position().y as f64 + m.size().height as f64 / 2.0) > pos.y).unwrap_or(true);
        let y = if top { pos.y + item.height } else { pos.y - size.height as f64 };
        let _ = p.set_position(tauri::PhysicalPosition::new(x.max(0.0), y.max(0.0)));
    } else if let Some(m) = p.current_monitor().ok().flatten() {
        // With no item to sit under (opened by its command), at the screen's top right, as a menu bar item's would.
        let margin = (8.0 * m.scale_factor()) as i32;
        let _ = p.set_position(tauri::PhysicalPosition::new(m.position().x + m.size().width as i32 - size.width as i32 - margin, m.position().y + margin));
    }
    let _ = p.show();
    let _ = p.set_focus();
}

#[cfg(not(desktop))]
fn toggle_panel(_: Option<()>) {}

// MARK: Starting

/// A window like a menu bar item's popover, of this page at `page`: hidden until its item's clicked, gone when it loses
/// focus.
#[cfg(desktop)]
fn popup(handle: &AppHandle, label: &str, page: &str) -> tauri::Result<tauri::WebviewWindow> {
    let window = WebviewWindowBuilder::new(handle, label, WebviewUrl::App(page.into()))
        .title(one::config::NAME)
        .initialization_script(include_str!("native.js"))
        .use_https_scheme(true)
        .inner_size(PANEL.0, PANEL.1)
        // Sized by its screen (`fit`), which GTK refuses a window that isn't resizable; with no edges to drag, a person
        // can't.
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    appear(&window);
    let hide = window.clone();
    window.on_window_event(move |e| {
        if let tauri::WindowEvent::Focused(false) = e {
            let _ = hide.hide();
        }
    });
    Ok(window)
}

/// The tray item (the menu bar's, on a Mac): a click shows the panel; its menu (a right click's) has Open, Open at Login,
/// on a Mac Let Agents Use This Mac and the widgets hidden from the menu bar, and Quit.
#[cfg(desktop)]
fn tray(app: &tauri::App) -> tauri::Result<()> {
    // Open at login from the first start, as the Mac app was; the person's choice after that. Never a test's build.
    let first = app.path().app_data_dir()?.join("opened-at-login");
    if !first.exists() && !identifier().ends_with(".throwaway") {
        login::set(app.handle(), true);
        let _ = std::fs::create_dir_all(first.parent().unwrap());
        let _ = std::fs::write(&first, "");
    }
    let mut tray = tauri::tray::TrayIconBuilder::with_id("main")
        .tooltip(one::config::NAME)
        // A click shows the panel; the menu is a right click's (Linux's tray items show only their menu).
        .show_menu_on_left_click(cfg!(target_os = "linux"))
        .on_tray_icon_event(|_, e| {
            if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, rect, .. } = e {
                toggle_panel(Some(rect));
            }
        })
        .on_menu_event(move |app, e| match e.id().as_ref() {
            "open" => show(),
            "login" => {
                login::set(app, !login::on(app));
                tray_menu(app);
            }
            "computer-use" => drop(computer_use(!computer_use_now().unwrap_or(true))),
            "quit" => app.exit(0),
            #[cfg(target_os = "macos")]
            id => {
                if let Some(kind) = id.strip_prefix("show ") {
                    menubar::set_hidden(kind, false);
                }
            }
            #[cfg(not(target_os = "macos"))]
            _ => {}
        });
    // The Mac's menu bar mark: the custom build's (build.rs), a template macOS tints for the menu bar.
    #[cfg(target_os = "macos")]
    {
        tray = tray.icon(tauri::image::Image::from_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/menubar.png")))?).icon_as_template(true);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    tray_menu(app.handle());
    Ok(())
}

/// The tray item's menu, as things are now.
#[cfg(desktop)]
fn tray_menu(app: &AppHandle) {
    use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
    let Some(tray) = app.tray_by_id("main") else { return };
    let made = || -> tauri::Result<Menu<tauri::Wry>> {
        let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![
            Box::new(MenuItem::with_id(app, "open", format!("Open {}", one::config::NAME), true, None::<&str>)?),
            Box::new(CheckMenuItem::with_id(app, "login", "Open at Login", true, login::on(app), None::<&str>)?),
        ];
        if let Some(on) = computer_use_now() {
            items.push(Box::new(CheckMenuItem::with_id(app, "computer-use", "Let Agents Use This Mac", true, on, None::<&str>)?));
        }
        #[cfg(target_os = "macos")]
        {
            let hidden = menubar::hidden();
            let widgets: Vec<_> = menubar::widgets().into_iter().filter(|w| hidden.contains(&w.0)).collect();
            if !widgets.is_empty() {
                items.push(Box::new(PredefinedMenuItem::separator(app)?));
                for (kind, title, ..) in widgets {
                    items.push(Box::new(MenuItem::with_id(app, format!("show {kind}"), format!("Show {title} in the Menu Bar"), true, None::<&str>)?));
                }
            }
        }
        items.push(Box::new(PredefinedMenuItem::separator(app)?));
        items.push(Box::new(MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?));
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|i| i.as_ref()).collect();
        Menu::with_items(app, &refs)
    };
    match made() {
        Ok(menu) => drop(tray.set_menu(Some(menu))),
        Err(e) => eprintln!("the tray's menu: {e}"),
    }
}

/// A widget's menu bar item clicked: the widget under it (the panel's page, showing only that widget, in a window of
/// its own), or gone if it shows.
#[cfg(target_os = "macos")]
fn toggle_widget(kind: &str, at: tauri::Rect) {
    let handle = &app().handle;
    let page = format!("index.html?panel&widget={}", url::form_urlencoded::byte_serialize(kind.as_bytes()).collect::<String>());
    let window = match handle.get_webview_window("widget") {
        Some(w) => {
            if !w.url().is_ok_and(|u| u.as_str().ends_with(&page)) {
                let _ = w.hide();
                let _ = w.navigate(tauri::Url::parse(&format!("https://tauri.localhost/{page}")).unwrap());
            }
            w
        }
        None => match popup(handle, "widget", &page) {
            Ok(w) => w,
            Err(e) => return eprintln!("the widget's window: {e}"),
        },
    };
    toggle_under(&window, Some(at));
}

/// One of the app menu's commands, to the window's page, which does it: "sidebar" (fold it or unfold it), "join".
#[cfg(target_os = "macos")]
fn command(name: &str) {
    if let Some(w) = app().handle.get_webview_window("main") {
        let _ = w.eval(format!("window.__oneNative?.command({})", json!(name)));
    }
}

/// A link the app was opened with (`<cli>://…`, or one to the network's address, which Windows hands it): to the page,
/// which goes there, or kept for it until it asks.
fn opened(url: &str) {
    let a = app();
    let own = url.starts_with(&format!("{}://", one::config::CLI))
        || a.address().is_some_and(|address| url::Url::parse(url).is_ok_and(|u| u.origin().ascii_serialization() == address));
    if !own {
        return;
    }
    show();
    match a.handle.get_webview_window("main").filter(|_| a.page.load(Ordering::Relaxed)) {
        Some(w) => drop(w.eval(format!("window.__oneNative?.opened({})", json!(url)))),
        None => *a.opened.lock().unwrap() = Some(url.to_string()),
    }
}

/// Opening at login. Linux: an XDG autostart entry, where XDG_CONFIG_HOME says; Windows: the package's startup task; a
/// Mac: its login item (SMAppService's main app).
#[cfg(desktop)]
mod login {
    use tauri::AppHandle;
    #[cfg(target_os = "linux")]
    use tauri::Manager;

    #[cfg(target_os = "linux")]
    fn entry(app: &AppHandle) -> Option<std::path::PathBuf> {
        Some(app.path().config_dir().ok()?.join("autostart").join(format!("{}.desktop", app.config().identifier)))
    }

    pub fn on(app: &AppHandle) -> bool {
        #[cfg(target_os = "linux")]
        return entry(app).is_some_and(|e| e.exists());
        #[cfg(windows)]
        return {
            let _ = app;
            crate::windows::login()
        };
        #[cfg(target_os = "macos")]
        return {
            let _ = app;
            crate::mac::login()
        };
    }

    pub fn set(app: &AppHandle, on: bool) {
        #[cfg(target_os = "linux")]
        if let (Some(entry), Ok(exe)) = (entry(app), std::env::current_exe()) {
            if !on {
                let _ = std::fs::remove_file(entry);
                return;
            }
            let _ = std::fs::create_dir_all(entry.parent().unwrap());
            let _ = std::fs::write(entry, format!("[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"\nTerminal=false\n", one::config::NAME, exe.display()));
        }
        #[cfg(windows)]
        crate::windows::set_login(on);
        #[cfg(target_os = "macos")]
        crate::mac::set_login(on);
        let _ = (app, on);
    }
}

/// On Linux, the app in the desktop's launcher, with its icon, and as what opens `<cli>://` links: written each start,
/// so it names the program where it is now.
#[cfg(target_os = "linux")]
fn launcher(app: &tauri::App) -> Result<(), String> {
    let id = &app.config().identifier;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let data = app.path().data_dir().map_err(|e| e.to_string())?;
    let icon = data.join("icons/hicolor/1024x1024/apps").join(format!("{id}.png"));
    std::fs::create_dir_all(icon.parent().unwrap()).map_err(|e| e.to_string())?;
    if let Some(bytes) = one::web::files().into_iter().find_map(|(path, bytes)| (path == "icon.png").then_some(bytes)) {
        std::fs::write(&icon, bytes).map_err(|e| e.to_string())?;
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exe}\" %u\nIcon={id}\nTerminal=false\nCategories=Office;Network;\nMimeType=x-scheme-handler/{cli};\nStartupWMClass={cli}\n",
        name = one::config::NAME,
        exe = exe.display(),
        cli = one::config::CLI,
    );
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    std::fs::write(apps.join(format!("{id}.desktop")), entry).map_err(|e| e.to_string())?;
    // And it's what opens `<cli>://`, in mimeapps.list where XDG_CONFIG_HOME says.
    let _ = std::process::Command::new("xdg-mime").args(["default", &format!("{id}.desktop"), &format!("x-scheme-handler/{}", one::config::CLI)]).status();
    Ok(())
}

/// The window, in front.
fn show() {
    #[cfg(desktop)]
    if let Some(w) = app().handle.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// In a debug build, a loopback port an agent drives the window by (`CONTROL`): `POST /eval` runs its body in the
/// window's page (`/eval/<label>`, another window's); `POST /value` gives an expression's value, as JSON. Never in a build people use.
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
    tauri::generate_handler![native, address, sign_in, join, wait, fetch, read, cancel, removed, sign_out, notify, copy, open, save, changed, main, login, computer_use, pair, quit, panel, fit, badge, haptic, scan, menu, appearance, menubar]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // First, so a second start goes to the first before anything else runs.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|_, argv, _| {
            show();
            argv.iter().for_each(|a| opened(a));
        }))
        // The window as it was left; the panel shows only when its item is clicked.
        .plugin(tauri_plugin_window_state::Builder::new().with_denylist(&["panel"]).build());
    // A phone's notifications are its own (iphone.rs, android.rs), which the plugin's would take from it.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_notification::init());
    #[cfg(target_os = "ios")]
    let builder = builder.plugin(iphone::plugin());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android::plugin());
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_opener::init());
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler({
            let commands = commands();
            move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
                // Android's web view gives screens' frames the app's commands too: only the page's own frame has the token.
                #[cfg(target_os = "android")]
                if !android::ours(&invoke) {
                    invoke.resolver.reject("That isn’t for a screen to ask.");
                    return true;
                }
                commands(invoke)
            }
        })
        .setup(|app| {
            // Its local folder, not one a domain profile roams (Windows): the client's is this device's alone.
            #[cfg(not(target_os = "android"))]
            let data = app.path().app_local_data_dir()?;
            // The app's own folder, where what runs with no window finds it too.
            #[cfg(target_os = "android")]
            let data = android::files().ok_or("The app's folder.")?;
            // What the old Mac or iPhone app kept, taken over once.
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            apple::take_over(&data);
            // The Kotlin app's, taken over once: its sign-in and address.
            #[cfg(target_os = "android")]
            {
                android::take_over();
                // What screens gave to be shared, shown or saved is gone with the last run.
                let _ = std::fs::remove_dir_all(data.join("given"));
            }
            // This Mac's node kept running; the Mac app's menus, with its keys.
            #[cfg(target_os = "macos")]
            {
                mac::start_node();
                app.set_menu(mac::menu(app.handle())?)?;
                app.on_menu_event(|_, e| match e.id().as_ref() {
                    "sidebar" | "join" => {
                        show();
                        command(e.id().as_ref());
                    }
                    "pair" => main(Some(format!("{}://settings/devices?add=1", one::config::CLI))),
                    _ => {}
                });
            }
            #[cfg(debug_assertions)]
            if let Some(port) = env("CONTROL").and_then(|p| p.parse().ok()) {
                control(port);
            }
            let _ = APP.set(App {
                handle: app.handle().clone(),
                data,
                auth: Mutex::default(),
                device: Mutex::default(),
                bodies: Mutex::default(),
                next: AtomicU64::new(1),
                opened: Mutex::default(),
                page: Default::default(),
            });
            // What runs beside the window asks it, while it runs, rather than starting a client of its own.
            claim();
            #[cfg(desktop)]
            headless::serve(|| APP.get().and_then(|a| a.device().ok()));
            #[cfg(target_os = "linux")]
            if let Err(e) = launcher(app) {
                eprintln!("the launcher's entry: {e}");
            }
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                // Windows learns the app opens `<cli>://` as it runs (Linux by `launcher`); elsewhere its bundle says so.
                #[cfg(windows)]
                let _ = app.deep_link().register_all();
                for url in app.deep_link().get_current().ok().flatten().unwrap_or_default() {
                    opened(url.as_str());
                }
                app.deep_link().on_open_url(|e| e.urls().iter().for_each(|u| opened(u.as_str())));
            }
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(include_str!("native.js"))
                .use_https_scheme(true)
                // Anything that would take the page away goes to the browser.
                .on_navigation(|u| {
                    let own = matches!(u.scheme(), "tauri") || u.host_str() == Some("tauri.localhost");
                    if !own {
                        let _ = open(u.to_string());
                    }
                    own
                });
            #[cfg(desktop)]
            let window = window.title(one::config::NAME).inner_size(1000.0, 700.0).min_inner_size(360.0, 480.0);
            #[cfg_attr(mobile, allow(unused_variables))]
            let window = window.build()?;
            #[cfg(desktop)]
            appear(&window);
            // On a computer, closing the window leaves the app running, in the tray or menu bar, where Open shows it
            // again (or starting it again does); Quit quits.
            #[cfg(desktop)]
            {
                let hide = window.clone();
                window.on_window_event(move |e| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let _ = hide.hide();
                    }
                });
                tray(app)?;
                // The widgets' own items, as last kept, until the node's answer redraws them.
                #[cfg(target_os = "macos")]
                menubar::update();
                // The panel: this page in panel mode, hidden until the tray item's clicked, gone when it loses focus.
                popup(app.handle(), "panel", "index.html?panel")?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the app");
}
