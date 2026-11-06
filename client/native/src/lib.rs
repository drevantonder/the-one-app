//! The native app: one window, which is the web client's own page (cloudflare/web/web.js, its files from one::web,
//! built in), drawn and run as it is in a browser, but on this device's client (one_client) rather than a browser's.
//! The page finds the app as `window.__oneNative` (src/native.js), which only it has: screens run in sandboxed frames
//! in it with no origin of their own, which reach none of the app's commands, and ask the page, as they do in a
//! browser. What the app adds is what a page can't do: this device's key in the OS's secret store, signing in through
//! the system's browser, notifications, the clipboard, files, and links outside.

mod secrets;
mod signin;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;

/// What a call says it came from.
fn client_name() -> String {
    std::env::consts::OS.replace("macos", "mac")
}

/// What the person calls this device, as their other devices list it.
fn device_name() -> String {
    let host = std::env::var("HOSTNAME").ok().or_else(|| std::fs::read_to_string("/etc/hostname").ok()).map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    host.unwrap_or_else(|| platform().into())
}

/// The platform, as people say it.
fn platform() -> &'static str {
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

fn hex(b: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(b)
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

    /// The network's address, as the person gave it, or `ADDRESS` for a test.
    fn address(&self) -> Option<String> {
        env("ADDRESS").or_else(|| self.kept("address")).map(|a| a.trim_end_matches('/').to_string())
    }

    fn auth(&self) -> Result<Arc<signin::Auth>, String> {
        let address = self.address().ok_or("Enter your address first.")?;
        let mut auth = self.auth.lock().unwrap();
        if let Some(a) = auth.as_ref().filter(|a| a.address == address) {
            return Ok(a.clone());
        }
        let a = Arc::new(signin::Auth::new(&address, &format!("{} on {}", one::config::NAME, device_name())));
        *auth = Some(a.clone());
        Ok(a)
    }

    fn device(&self) -> Result<Arc<one_client::Device>, String> {
        self.device.lock().unwrap().clone().ok_or_else(|| "This device hasn't been approved yet.".into())
    }
}

/// An access token, from the refresh token in the secret store (a request to sign-in, so off the async threads).
async fn token() -> Result<String, String> {
    let auth = app().auth()?;
    tauri::async_runtime::spawn_blocking(move || auth.token()).await.map_err(|e| e.to_string())?
}

/// This device's key at this address, kept in the secret store, made the first time.
fn device_secret(address: &str) -> Result<[u8; 32], String> {
    let key = format!("device {address}");
    if let Some(s) = secrets::get(&key).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        return Ok(s);
    }
    let s = one_client::new_secret();
    secrets::set(&key, &s)?;
    Ok(s)
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
    json!({
        "name": one::config::NAME, "yours": one::config::YOURS, "cli": one::config::CLI, "platform": platform(), "device": device_name(), "client": client_name(),
        "address": address, "signedIn": signed_in, "user": if signed_in { a.kept("user") } else { None },
        "opened": opened,
    })
}

/// Takes what the person typed: an address, or an invite link (`<address>/invite#…`), whose secret goes with this
/// device's request to be let in.
#[tauri::command]
fn address(typed: String) -> Result<String, String> {
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
    Ok(origin)
}

/// Signs in, in the system's browser, which comes back to this device; naming the device it's for, so the person's
/// devices can let it in by this sign-in.
#[tauri::command]
async fn sign_in() -> Result<(), String> {
    let a = app();
    let address = a.address().ok_or("Enter your address first.")?;
    let join = one_client::join_of(device_secret(&address)?);
    let auth = a.auth()?;
    let handle = a.handle.clone();
    let auth2 = auth.clone();
    let print = env("SIGN_IN_PRINT").is_some();
    tauri::async_runtime::spawn_blocking(move || {
        auth2.sign_in(&join, |u| {
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
    let secret = device_secret(&address)?;
    let device = {
        let mut d = a.device.lock().unwrap();
        match d.as_ref() {
            Some(d) => d.clone(),
            None => {
                use sha2::{Digest, Sha256};
                let sum = Sha256::digest(format!("{address} {user}").as_bytes());
                let dir = a.data.join("client").join(hex(&sum[..8]));
                let made = Arc::new(one_client::Device::new(secret, &address, &dir.to_string_lossy()).keeping());
                *d = Some(made.clone());
                made
            }
        }
    };
    match device.connect().await {
        Ok(true) => {
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
    if let Some(address) = a.address() {
        secrets::delete(&format!("device {address}"));
    }
    a.keep("user", None);
    Ok(())
}

/// Whether this device shows notifications, asking the person first if `ask` → "granted", "denied" or "default".
#[tauri::command]
fn notify(ask: bool) -> String {
    let n = app().handle.notification();
    let state = match n.permission_state() {
        Ok(PermissionState::Granted) => PermissionState::Granted,
        Ok(s) if !ask => s,
        _ => n.request_permission().unwrap_or(PermissionState::Denied),
    };
    match state {
        PermissionState::Granted => "granted".into(),
        PermissionState::Denied => "denied".into(),
        _ => "default".into(),
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
    app().handle.opener().open_url(url, None::<&str>).map_err(|_| "Couldn’t open your browser.".to_string())
}

/// A file a screen gives (`data`, base64): saved where the person chooses, or with `preview`, opened in the app for it.
#[tauri::command]
async fn save(name: String, data: String, preview: bool) -> Result<(), String> {
    let data = data_encoding::BASE64.decode(data.as_bytes()).map_err(|e| e.to_string())?;
    let name: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
    let name = if name.trim().is_empty() { "file".to_string() } else { name };
    if preview {
        let dir = std::env::temp_dir().join(format!("{}-{}", one::config::CLI, app().next.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(&name);
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        return app().handle.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string());
    }
    let handle = app().handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = handle.dialog().file().set_file_name(&name).blocking_save_file() else { return Ok(()) };
        let path = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(path, data).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// MARK: Starting

/// The tray item (the menu bar's, on a Mac): Open, Quit.
#[cfg(desktop)]
fn tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    let open = MenuItem::with_id(app, "open", format!("Open {}", one::config::NAME), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = tauri::tray::TrayIconBuilder::with_id("main").tooltip(one::config::NAME).menu(&menu).on_menu_event(|app, e| match e.id().as_ref() {
        "open" => show(),
        "quit" => app.exit(0),
        _ => {}
    });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// A link the app was opened with (`<cli>://…`): to the page, which goes there, or kept for it until it asks.
fn opened(url: &str) {
    if !url.starts_with(&format!("{}://", one::config::CLI)) {
        return;
    }
    let a = app();
    show();
    match a.handle.get_webview_window("main").filter(|_| a.page.load(Ordering::Relaxed)) {
        Some(w) => drop(w.eval(format!("window.__oneNative?.opened({})", json!(url)))),
        None => *a.opened.lock().unwrap() = Some(url.to_string()),
    }
}

/// The window, in front.
fn show() {
    if let Some(w) = app().handle.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// In a debug build, a loopback port an agent drives the window by (`CONTROL`): `POST /eval` runs its body in the
/// window's page. Never in a build people use.
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
            let out = match app().handle.get_webview_window("main") {
                Some(w) if line.starts_with("POST /eval") => w.eval(String::from_utf8_lossy(&body)).map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()),
                _ => "POST /eval".into(),
            };
            let mut s = &s;
            let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}", out.len());
        }
    });
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
        .plugin(tauri_plugin_window_state::Builder::new().build());
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![native, address, sign_in, join, wait, fetch, read, cancel, removed, sign_out, notify, copy, open, save])
        .setup(|app| {
            let data = app.path().app_data_dir()?;
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
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                // Linux and Windows learn the app opens `<cli>://` as it runs; elsewhere its bundle says so.
                #[cfg(any(windows, target_os = "linux"))]
                let _ = app.deep_link().register_all();
                for url in app.deep_link().get_current().ok().flatten().unwrap_or_default() {
                    opened(url.as_str());
                }
                app.deep_link().on_open_url(|e| e.urls().iter().for_each(|u| opened(u.as_str())));
            }
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(one::config::NAME)
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
            let window = window.inner_size(1000.0, 700.0).min_inner_size(360.0, 480.0);
            let window = window.build()?;
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
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the app");
}
