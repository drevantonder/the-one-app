//! The native app: one window, which is the web client's own page (cloudflare/web/web.js, its files from one::web,
//! built in), drawn and run as it is in a browser, but on this device's client (one_client) rather than a browser's.
//! The page finds the app as `window.__oneNative` (src/native.js), which only it has: screens run in sandboxed frames
//! in it with no origin of their own, which reach none of the app's commands, and ask the page, as they do in a
//! browser. What the app adds is what a page can't do: this device's key in the OS's secret store, signing in through
//! the system's browser, notifications, the clipboard, files, and links outside.

#[macro_use]
mod commands;
mod headless;
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

/// What the person calls this device, as their other devices list it.
fn device_name() -> String {
    platform::name().unwrap_or_else(|| platform::LABEL.into())
}

/// The platform, as people say it (in a debug build, `PLATFORM` for a test: a phone's look on a computer).
fn label() -> &'static str {
    #[cfg(debug_assertions)]
    if let Some(p) = env("PLATFORM") {
        return if p == "iPhone" { "iPhone" } else if p == "Android" { "Android" } else { "Linux" };
    }
    platform::LABEL
}

/// A setting for a test from its environment, named by the CLI's name (`ONE_ADDRESS`): `ADDRESS`, the network's;
/// `SIGN_IN_PRINT`, sign in by printing the link rather than opening a browser (`one throwaway client linux` signs in
/// there); `CONTROL`, in a debug build, a loopback port an agent drives the window by.
fn env(name: &str) -> Option<String> {
    std::env::var(format!("{}_{name}", one::config::CLI.to_uppercase())).ok().filter(|v| !v.is_empty())
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
    sign_in: Mutex<Option<Arc<signin::SignIn>>>,
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

/// What the app keeps in `data` as `name`, if anything.
fn kept_in(data: &std::path::Path, name: &str) -> Option<String> {
    std::fs::read_to_string(data.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The network's address: `ADDRESS` for a test, the custom build's if it has one, else as the person gave it.
fn address_in(data: &std::path::Path) -> Option<String> {
    env("ADDRESS").or_else(|| Some(one::config::ADDRESS.to_string()).filter(|a| !a.is_empty())).or_else(|| kept_in(data, "address")).map(|a| a.trim_end_matches('/').to_string())
}

/// This device's client for `id` (`<address> <user>`), its folder in `data` named by it.
fn client(data: &std::path::Path, address: &str, id: &str, secret: [u8; 32]) -> one_client::Device {
    one_client::Device::new(secret, address, &data.join("client").join(secrets::folder(id)).to_string_lossy()).keeping().logging(data)
}

/// A device's code, as typed or pushed: its letters and digits.
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
        let a = Arc::new(signin::SignIn::new(&address, &format!("{} on {}", one::config::NAME, device_name()), self.data.clone()));
        *kept = Some(a.clone());
        Ok(a)
    }

    fn device(&self) -> Result<Arc<one_client::Device>, String> {
        self.device.lock().unwrap().clone().ok_or_else(|| "This device hasn't been approved yet.".into())
    }
}

/// What a node answers `method` at `path` (with `body`, as JSON), as JSON: its `error`, or a status that isn't a success,
/// an Err.
async fn node(device: &one_client::Device, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let (headers, body) = match body {
        Some(b) => (vec![("content-type".into(), "application/json".into())], serde_json::to_vec(&b).map_err(|e| e.to_string())?),
        None => (Vec::new(), Vec::new()),
    };
    let r = device.fetch(method, path, headers, body).await.map_err(|e| e.to_string())?;
    let status = r.status;
    let out: Value = serde_json::from_slice(&r.body.bytes().await.map_err(|e| e.to_string())?).unwrap_or_default();
    match out["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None if !(200..300).contains(&status) => Err("That didn’t work. Try again.".into()),
        None => Ok(out),
    }
}

/// The widgets kept for the platform's own again, and drawn again where the app runs.
async fn widgets_changed(device: &one_client::Device) {
    if let Err(e) = widgets::refresh(device, &platform::widgets_dir()).await {
        one::logs::say(&format!("widgets: {e}"));
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

/// Let in, now or as it starts: the platform's own widgets kept, and its pushes registered.
fn let_in() {
    keep_widgets();
    if let Ok(device) = app().device() {
        platform::let_in(device);
    }
}

/// The panel, drawn before this device was in (signed out), drawn again now it is.
fn redraw_panel() {
    if let Some(p) = app().handle.get_webview_window("panel") {
        let _ = p.eval("location.reload()");
    }
}

/// Something changed on the person's devices (the page's events say): the widgets kept for the platform's own again.
#[tauri::command]
fn changed() {
    let Ok(device) = app().device() else { return };
    tauri::async_runtime::spawn(async move { widgets_changed(&device).await });
}

/// What waits for the person (every app's badge on every page, and Settings'), counted on the app's icon; none at 0.
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

impl From<one_client::sign_in::Failed> for Failure {
    fn from(f: one_client::sign_in::Failed) -> Failure {
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
    signin::subject(token).ok_or_else(|| one_client::sign_in::signed_out("Sign in again: that sign-in names no one.").into())
}

// MARK: What the page asks

/// Who and where this is: the custom build's name and CLI, the address (if one's given), whether someone's signed in
/// here and who.
#[tauri::command]
fn native() -> Value {
    let a = app();
    let address = a.address();
    let signed_in = address.as_ref().is_some_and(|_| a.sign_in().is_ok_and(|s| s.signed_in()));
    a.page.store(true, Ordering::Relaxed);
    let opened = a.opened.lock().unwrap().take();
    let local = local_state();
    let unpaired = platform::unpaired(local.as_ref());
    platform::menus(signed_in, unpaired);
    json!({
        "name": one::config::NAME, "yours": one::config::YOURS, "cli": one::config::CLI, "platform": label(), "device": device_name(), "client": platform::CLIENT,
        "address": address, "signedIn": signed_in, "user": if signed_in { a.kept("user") } else { None },
        "opened": opened,
        // This computer's own node, where one runs here: "This device" in the panel.
        "node": local.as_ref().and_then(|s| s["device"]["key"].as_str().map(str::to_string)),
        // A Mac whose own node is in no network yet joins one with a pairing code (`pair`), as the Mac app did.
        "unpaired": unpaired,
        // Open at login: whether it's on, where the app switches it (none where it can't).
        "login": platform::login(),
        // Let Agents Use This Mac, on a Mac (Computer Use's helper reads it).
        "computerUse": platform::computer_use(),
        // A Mac's widgets hidden from the menu bar (their own items), which its panel shows again.
        "menubar": platform::menubar_hidden(),
        // The document camera, where there's one (an iPhone's, Android's scanner).
        "scan": platform::can_scan(),
        // An iPhone's touch and hold lifting what's held with its menu (`menu`).
        "lifts": cfg!(target_os = "ios"),
        // How much bigger than usual the person has made text (a phone's: Android's font size, an iPhone's Dynamic Type),
        // for the window to be zoomed by.
        "textScale": platform::text_scale(),
    })
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
    platform::claim(a.address());
    Ok(origin)
}

/// Signs in, in the system's browser, which comes back to this device; naming the device it's for, so the person's
/// devices can let it in by this sign-in.
#[tauri::command]
async fn sign_in() -> Result<(), Failure> {
    let a = app();
    a.address().ok_or("Enter your address first.")?;
    let join = one_client::join_of(secrets::signing_in()?);
    let signing = a.sign_in()?;
    let print = env("SIGN_IN_PRINT").is_some();
    tauri::async_runtime::spawn_blocking(move || platform::sign_in(&signing, &join, print)).await.map_err(|e| e.to_string())??;
    let token = token().await?;
    a.keep("user", Some(&subject(&token)?));
    // Back in front of the browser it signed in with.
    platform::show();
    Ok(())
}

/// Makes this device its person's: one already, or asking to be let in. → `{ in: true }`, or the code to approve
/// it by (`{ code }`), after which `wait`.
#[tauri::command]
async fn join() -> Result<Value, Failure> {
    let a = app();
    let address = a.address().ok_or("Enter your address first.")?;
    let token = token().await?;
    let user = subject(&token)?;
    a.keep("user", Some(&user));
    // One key for each person at each address, as the Mac and iPhone apps keep them, its folder named by it too.
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
    // A computer is "<name> on <computer>", as the Mac app's was; a phone, its own name.
    let name = if cfg!(desktop) { format!("{} on {}", one::config::NAME, device_name()) } else { device_name() };
    device.ask(&token, &name, invite.as_deref()).await.map_err(|e| e.to_string())?;
    println!("Code: {}", device.code());
    platform::approve_here(device.code());
    Ok(json!({ "code": device.code(), "invited": invite.is_some() }))
}

/// Waits until this device is let in, as auth says it was approved and the relay lets it on.
#[tauri::command]
async fn wait() -> Result<(), Failure> {
    let device = app().device()?;
    loop {
        let token = token().await?;
        if device.wait(&token).await.map_err(|e| e.to_string())? {
            app().keep("invite", None);
            let_in();
            redraw_panel();
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

/// What `app`'s `tool` answered last for `input` here, if it only reads (`Device::kept`): `{ answer, at }`, or null.
#[tauri::command]
async fn kept(app: String, tool: String, input: Value) -> Option<Value> {
    let (answer, at) = self::app().device().ok()?.kept(&app, &tool, input.to_string().as_bytes()).await?;
    Some(json!({ "answer": serde_json::from_slice::<Value>(&answer).ok()?, "at": at }))
}

/// Renews its person's app schedules of `app`, which they just used (`Device::renew_app_schedules`).
#[tauri::command]
async fn renew(app: String) {
    if let Ok(device) = self::app().device() {
        device.renew_app_schedules(&app).await;
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
    if let Ok(signing) = a.sign_in() {
        let _ = tauri::async_runtime::spawn_blocking(move || signing.sign_out()).await;
    }
    if let (Some(address), Some(user)) = (a.address(), a.kept("user")) {
        secrets::forget_device(&format!("{address} {user}"));
    }
    a.keep("user", None);
    widgets::forget(&platform::widgets_dir());
    platform::redraw_widgets();
    badge(0);
    Ok(())
}

/// Whether this device shows notifications, asking the person first if `ask` → "granted", "denied" or "default"; none
/// where a phone's build has no pushes to show (no Firebase project in its config.toml), so Settings has no row for it.
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

/// A file a screen gives (`data`, base64): saved where the person chooses, or with `preview`, opened in the app for it.
#[tauri::command]
async fn save(name: String, data: String, preview: bool) -> Result<(), String> {
    let data = data_encoding::BASE64.decode(data.as_bytes()).map_err(|e| e.to_string())?;
    let name: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
    let name = if name.trim().is_empty() { "file".to_string() } else { name };
    platform::save(name, data, preview).await
}

/// What the page says went wrong (an error it didn't catch, a file that didn't load), for this device's log, which a person's
/// agent reads through a node (`one logs <device>`). The log's own caps and filters keep it small and free of what's secret.
#[tauri::command]
fn log(text: String) {
    one::logs::record(&format!("page: {text}"));
}

/// A phone's haptic: "selection" (a press), "medium" (a long press), "success" or "error"; elsewhere, nothing.
#[tauri::command]
fn haptic(style: String) {
    platform::haptic(style);
}

/// The document camera, where there's one (an iPhone's, or Android's scanner): each page found, straightened and cropped, as a JPEG
/// `{ name, type, size, base64 }`; none if the person cancels.
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

/// Light, dark or automatic, as the person chose: the app starts in it from its first frame, and a phone's system bars
/// follow it.
#[tauri::command]
fn appearance(value: String) {
    platform::appearance(value);
}

/// A widget's own menu bar item shown again, or hidden (a Mac's) → the kinds hidden now.
#[tauri::command]
fn menubar(kind: String, show: bool) -> Vec<String> {
    platform::menubar(kind, show)
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

/// This computer's own node joins a network with a pairing code another of its person's computers made (as `<cli>
/// join` does) → its address, which the app signs in to next.
#[tauri::command]
async fn pair(code: String) -> Result<String, String> {
    if !one::home().join("local-token").exists() {
        return Err("This Mac's node isn't running.".into());
    }
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let said = tauri::async_runtime::spawn_blocking(move || local("/v1/join", Some(json!({ "code": code })), 60)).await.map_err(|e| e.to_string())?;
    // An answer from the node says why by its code; none came when its own answer didn't (it was too slow: the device that made the code is off).
    let failed = match said {
        Ok((true, _)) => None,
        Ok((false, out)) => Some(out["code"].as_str().unwrap_or_default().to_string()),
        Err(_) => Some(one::protocol::code::UNREACHABLE.to_string()),
    };
    if let Some(why) = failed {
        use one::protocol::code;
        return Err(match why.as_str() {
            code::PAIRING_USED => "That pairing code was used or has expired. Make a new one on your other device.".into(),
            code::ALREADY_IN_NETWORK => format!("This device is already in another {}. Remove it there first.", one::config::NAME.trim_start_matches("The ")),
            code::BAD_PAIRING_CODE => "That isn’t a whole pairing code. Copy it again, all of it.".into(),
            code::UNREACHABLE => "Couldn’t reach the device that made the code. Check both are online, then try again.".into(),
            _ => "Couldn’t join. Make a new pairing code on your other device and try again.".into(),
        });
    }
    // Its network's address, which the person signs in to here.
    let address = local_state().and_then(|s| s["network"]["settings"]["address"].as_str().map(str::to_string)).ok_or("Joined. Now type your network’s address.")?;
    address_of(&address)
}

/// Let Agents Use This Mac, switched → whether it's on.
#[tauri::command]
fn computer_use(on: bool) -> Option<bool> {
    platform::set_computer_use(on);
    #[cfg(desktop)]
    tray_menu(&app().handle);
    platform::computer_use()
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

/// The panel's size, as the Mac app's was wide.
#[cfg(desktop)]
const PANEL: (f64, f64) = (340.0, 560.0);

#[cfg(desktop)]
fn toggle_panel(at: Option<tauri::Rect>) {
    if let Some(p) = app().handle.get_webview_window("panel") {
        toggle_under(&p, at);
    }
}

/// A window like the panel's, shown where this computer shows its tray's flyouts, or hidden if it shows: by the item at
/// `at` (its place and size on the screen, physical) if given, else in the corner where the tray is.
#[cfg(desktop)]
fn toggle_under(p: &tauri::WebviewWindow, at: Option<tauri::Rect>) {
    if p.is_visible().unwrap_or(false) {
        let _ = p.hide();
        return;
    }
    let Some(m) = p.current_monitor().ok().flatten() else { return };
    let scale = m.scale_factor();
    // Its size as last fitted to its screen; a window never shown yet says none, so as made.
    let size = p.outer_size().ok().filter(|s| s.width > 0 && s.height > 0).unwrap_or(tauri::PhysicalSize::new((PANEL.0 * scale) as u32, (PANEL.1 * scale) as u32));
    // What shows of it is what's placed: Windows' window reaches past it, invisibly.
    let (left, top, right, bottom) = platform::unseen(p);
    let seen = ((size.width as i32 - left - right).max(1) as u32, (size.height as i32 - top - bottom).max(1) as u32);
    let item = at.map(|at| (at.position.to_physical::<f64>(scale), at.size.to_physical::<f64>(scale)));
    let place = flyout(m.work_area(), (m.position().x, m.position().y, m.size().width, m.size().height), seen, item.map(|(p, s)| (p.x, p.y, s.width, s.height)), scale);
    let _ = p.set_position(tauri::PhysicalPosition::new(place.0 - left, place.1 - top));
    let _ = p.show();
    let _ = p.set_focus();
}

/// The bar the tray is on: which side of the screen it takes from the work area.
#[cfg(desktop)]
#[derive(Clone, Copy, PartialEq)]
enum Bar {
    Top,
    Bottom,
    Left,
    Right,
}

/// Where a flyout of `size` goes, all physical: inside the monitor's work area (`work`; the taskbar's or panel's room
/// left out of `screen`), a gap from the bar and the edges as the OS keeps its own (Windows 11's 12 px, a Mac's popover
/// just under the menu bar), centred on its tray `item` if it has one and that's on the bar, else in the bar's corner:
/// the end where the tray is (the right, or the bottom of a side bar). An item in Windows' overflow, above its taskbar,
/// is off the bar, so it goes in the corner, as Windows' own flyouts do.
#[cfg(desktop)]
fn flyout(work: &tauri::PhysicalRect<i32, u32>, screen: (i32, i32, u32, u32), size: (u32, u32), item: Option<(f64, f64, f64, f64)>, scale: f64) -> (i32, i32) {
    let gap = scale * platform::FLYOUT_GAP;
    let (wx, wy, ww, wh) = (work.position.x as f64, work.position.y as f64, work.size.width as f64, work.size.height as f64);
    let (sx, sy, sw, sh) = (screen.0 as f64, screen.1 as f64, screen.2 as f64, screen.3 as f64);
    let (w, h) = (size.0 as f64, size.1 as f64);
    // The side the work area gives up, else the item's side of the screen, else where this OS keeps it.
    let bar = if wy > sy {
        Bar::Top
    } else if wy + wh < sy + sh {
        Bar::Bottom
    } else if wx > sx {
        Bar::Left
    } else if wx + ww < sx + sw {
        Bar::Right
    } else if let Some((_, iy, _, _)) = item {
        if iy < sy + sh / 2.0 { Bar::Top } else { Bar::Bottom }
    } else if platform::TRAY_AT_BOTTOM {
        Bar::Bottom
    } else {
        Bar::Top
    };
    let on_bar = item.filter(|&(ix, iy, iw, ih)| match bar {
        Bar::Top => iy < wy,
        Bar::Bottom => iy + ih > wy + wh,
        Bar::Left => ix < wx,
        Bar::Right => ix + iw > wx + ww,
    });
    let (min_x, max_x) = (wx + gap, (wx + ww - w - gap).max(wx + gap));
    let (min_y, max_y) = (wy + gap, (wy + wh - h - gap).max(wy + gap));
    let (x, y) = match (bar, on_bar) {
        (Bar::Top, Some((ix, _, iw, _))) => (ix + iw / 2.0 - w / 2.0, min_y),
        (Bar::Bottom, Some((ix, _, iw, _))) => (ix + iw / 2.0 - w / 2.0, max_y),
        (Bar::Left, Some((_, iy, _, ih))) => (min_x, iy + ih / 2.0 - h / 2.0),
        (Bar::Right, Some((_, iy, _, ih))) => (max_x, iy + ih / 2.0 - h / 2.0),
        (Bar::Top, None) => (max_x, min_y),
        (Bar::Bottom, None) => (max_x, max_y),
        (Bar::Left, None) => (min_x, max_y),
        (Bar::Right, None) => (max_x, max_y),
    };
    (x.clamp(min_x, max_x).round() as i32, y.clamp(min_y, max_y).round() as i32)
}

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
        // Windows' system shadow, as its flyouts have.
        .shadow(true)
        .build()?;
    platform::appear(&window);
    platform::round(&window);
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
        platform::set_login(true);
        let _ = std::fs::create_dir_all(first.parent().unwrap());
        let _ = std::fs::write(&first, "");
    }
    let tray = tauri::tray::TrayIconBuilder::with_id("main")
        .tooltip(one::config::NAME)
        // A click shows the panel; the menu is a right click's (Linux's tray items show only their menu).
        .show_menu_on_left_click(cfg!(target_os = "linux"))
        .on_tray_icon_event(|_, e| {
            if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, rect, .. } = e {
                toggle_panel(Some(rect));
            }
        })
        .on_menu_event(move |app, e| match e.id().as_ref() {
            "open" => platform::show(),
            "login" => {
                platform::set_login(!platform::login().unwrap_or(true));
                tray_menu(app);
            }
            "computer-use" => drop(computer_use(!platform::computer_use().unwrap_or(true))),
            "quit" => app.exit(0),
            // A widget's own menu bar item shown again (a Mac's).
            id => {
                if let Some(kind) = id.strip_prefix("show ") {
                    platform::menubar(kind.into(), true);
                }
            }
        });
    platform::tray_icon(app, tray)?.build(app)?;
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
            Box::new(CheckMenuItem::with_id(app, "login", "Open at Login", true, platform::login().unwrap_or(false), None::<&str>)?),
        ];
        if let Some(on) = platform::computer_use() {
            items.push(Box::new(CheckMenuItem::with_id(app, "computer-use", "Let Agents Use This Mac", true, on, None::<&str>)?));
        }
        let hidden = platform::hidden_widgets();
        if !hidden.is_empty() {
            items.push(Box::new(PredefinedMenuItem::separator(app)?));
            for (kind, title) in hidden {
                items.push(Box::new(MenuItem::with_id(app, format!("show {kind}"), format!("Show {title} in the Menu Bar"), true, None::<&str>)?));
            }
        }
        items.push(Box::new(PredefinedMenuItem::separator(app)?));
        items.push(Box::new(MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?));
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|i| i.as_ref()).collect();
        Menu::with_items(app, &refs)
    };
    match made() {
        Ok(menu) => drop(tray.set_menu(Some(menu))),
        Err(e) => one::logs::say(&format!("the tray's menu: {e}")),
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
    platform::show();
    match a.handle.get_webview_window("main").filter(|_| a.page.load(Ordering::Relaxed)) {
        Some(w) => drop(w.eval(format!("window.__oneNative?.opened({})", json!(url)))),
        None => *a.opened.lock().unwrap() = Some(url.to_string()),
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
            one::logs::keep_in(&data, one::logs::CLIENT);
            let earlier = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                one::logs::say(&format!("panic: {info}"));
                earlier(info)
            }));
            one::logs::say(&format!("{} {} started on {}", one::config::NAME, env!("CARGO_PKG_VERSION"), device_name()));
            #[cfg(debug_assertions)]
            if let Some(port) = env("CONTROL").and_then(|p| p.parse().ok()) {
                control(port);
            }
            let _ = APP.set(App {
                handle: app.handle().clone(),
                data,
                sign_in: Mutex::default(),
                device: Mutex::default(),
                bodies: Mutex::default(),
                next: AtomicU64::new(1),
                opened: Mutex::default(),
                page: Default::default(),
            });
            platform::start(app);
            platform::claim(crate::app().address());
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                for url in app.deep_link().get_current().ok().flatten().unwrap_or_default() {
                    opened(url.as_str());
                }
                app.deep_link().on_open_url(|e| e.urls().iter().for_each(|u| opened(u.as_str())));
            }
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(include_str!("native.js"))
                .use_https_scheme(true)
                .on_page_load(|_, load| {
                    let event = match load.event() {
                        tauri::webview::PageLoadEvent::Started => "started",
                        tauri::webview::PageLoadEvent::Finished => "finished",
                    };
                    one::logs::record(&format!("page {event} loading {}", load.url()));
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
            let window = window.title(one::config::NAME).inner_size(1000.0, 700.0).min_inner_size(360.0, 480.0);
            #[cfg_attr(mobile, allow(unused_variables))]
            let window = window.build()?;
            #[cfg(desktop)]
            platform::appear(&window);
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
                // The panel: this page in panel mode, hidden until the tray item's clicked, gone when it loses focus.
                popup(app.handle(), "panel", "index.html?panel")?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the app");
}
