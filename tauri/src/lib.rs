//! The One App's client shell as a Tauri 2 app (a spike): it signs in, keeps this device's key in the OS's secret store,
//! lists the person's apps on its home page (the shell's own page, the only one granted its commands), and opens each
//! app's screen in a window of its own. A screen is untrusted: it's served at screen://<app>/ (http://screen.<app>/ on
//! Windows and Android) by the shell, in a window no capability names, and talks only by posting bridge.js's messages
//! to its own origin's /one, which `bridge::answer` answers as that app alone. The client is `one_client` itself.

mod bridge;
mod node;
mod secrets;
mod signin;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::http::{Response, StatusCode};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;

use bridge::{MenuItem, Route};

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What the home page shows: signing in, the code to approve, or the apps.
#[derive(Clone, Serialize, Default)]
struct State {
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    apps: Vec<Value>,
    #[serde(rename = "readyMs")]
    ready_ms: u128,
}

/// One open screen: its route, its page as the node gave it, and who waits for it to close (a sheet's opener).
struct Screen {
    route: Route,
    html: String,
    value: Mutex<Value>,
    closed: Mutex<Option<tokio::sync::oneshot::Sender<Value>>>,
}

struct Shell {
    app: AppHandle,
    address: String,
    data: PathBuf,
    started: Instant,
    auth: OnceLock<signin::Auth>,
    device: OnceLock<Arc<one_client::Device>>,
    state: Mutex<State>,
    screens: Mutex<HashMap<String, Arc<Screen>>>,
    menus: Mutex<HashMap<String, tokio::sync::oneshot::Sender<usize>>>,
    next: AtomicU64,
}

static SHELL: OnceLock<Arc<Shell>> = OnceLock::new();

fn shell() -> &'static Arc<Shell> {
    SHELL.get().expect("the shell")
}

fn log(text: impl AsRef<str>) {
    eprintln!("[one-tauri {:>6} ms] {}", SHELL.get().map_or(0, |s| s.started.elapsed().as_millis()), text.as_ref());
}

impl Shell {
    fn set(&self, f: impl FnOnce(&mut State)) {
        let s = {
            let mut st = self.state.lock().unwrap();
            f(&mut st);
            st.clone()
        };
        let _ = self.app.emit_to("home", "one-state", s);
    }

    fn fail(&self, e: impl std::fmt::Display) {
        log(format!("failed: {e}"));
        self.set(|s| {
            s.phase = "failed";
            s.error = Some(e.to_string());
        });
    }

    fn device(&self) -> Result<&Arc<one_client::Device>, String> {
        self.device.get().ok_or_else(|| "Not signed in yet.".to_string())
    }
}

#[tauri::command]
fn state() -> State {
    shell().state.lock().unwrap().clone()
}

#[tauri::command]
async fn open(app: String) -> Result<(), String> {
    open_screen(Route { app, screen: "main".into(), params: "{}".into(), ..Default::default() }, None).await.map(|_| ())
}

#[tauri::command]
async fn sign_out() -> Result<(), String> {
    do_sign_out().await
}

async fn do_sign_out() -> Result<(), String> {
    let sh = shell();
    if let Some(d) = sh.device.get() {
        d.sign_out().await;
    }
    if let Some(a) = sh.auth.get() {
        a.sign_out();
    }
    secrets::delete(&format!("device {}", sh.address));
    sh.set(|s| *s = State { phase: "failed", error: Some("Signed out. Start the app again to sign in.".into()), ..Default::default() });
    Ok(())
}

/// This device's key, kept in the secret store, made the first time.
fn device_secret(address: &str) -> Result<[u8; 32], String> {
    let key = format!("device {address}");
    if let Some(s) = secrets::get(&key).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        return Ok(s);
    }
    let s = one_client::new_secret();
    secrets::set(&key, &s)?;
    Ok(s)
}

/// An id of this computer's own, hashed, that outlives signing out (Linux's machine id; elsewhere none, in the spike).
fn on_device() -> Option<String> {
    use sha2::{Digest, Sha256};
    ["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|p| {
        let id = std::fs::read_to_string(p).ok()?;
        Some(hex(&Sha256::digest(format!("this device {}", id.trim()).as_bytes())))
    })
}

fn host_name() -> String {
    std::env::var("HOSTNAME").ok().or_else(|| std::fs::read_to_string("/etc/hostname").ok()).map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).unwrap_or_else(|| std::env::consts::OS.into())
}

/// Signs in if it must, lets this device in, then lists the apps.
async fn start() {
    let sh = shell();
    if sh.address.is_empty() {
        return sh.fail("No address: start it with ONE_ADDRESS=<your One App's address>.");
    }
    match secrets::works() {
        Ok(()) => log("secret store: works"),
        Err(e) => return sh.fail(format!("the secret store: {e}")),
    }
    let secret = match device_secret(&sh.address) {
        Ok(s) => s,
        Err(e) => return sh.fail(e),
    };
    let auth = sh.auth.get_or_init(|| signin::Auth::new(&sh.address, &format!("The One App (Tauri) on {}", host_name())));
    if !auth.signed_in() {
        sh.set(|s| s.phase = "signing-in");
        let join = one_client::join_of(secret);
        let app = sh.app.clone();
        let in_window = std::env::var("ONE_SIGN_IN_HERE").is_ok() || cfg!(mobile);
        let signed = tauri::async_runtime::spawn_blocking(move || {
            let auth = shell().auth.get().unwrap();
            let r = auth.sign_in(&join, |u| {
                if in_window {
                    let url = u.parse().map_err(|e: url::ParseError| e.to_string())?;
                    // An untrusted page with no capability: sign-in's pages, then the loopback callback.
                    WebviewWindowBuilder::new(&app, "signin", WebviewUrl::External(url)).title("Sign In").inner_size(480.0, 640.0).build().map_err(|e| e.to_string())?;
                    Ok(())
                } else {
                    app.opener().open_url(u, None::<&str>).map_err(|e| e.to_string())
                }
            });
            if let Some(w) = app.get_webview_window("signin") {
                let _ = w.destroy();
            }
            r
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        if let Err(e) = signed {
            return sh.fail(e);
        }
        log("signed in");
    }
    let token = match tauri::async_runtime::spawn_blocking(|| shell().auth.get().unwrap().token()).await.unwrap() {
        Ok(t) => t,
        Err(e) => return sh.fail(e),
    };
    use sha2::{Digest, Sha256};
    let sum = Sha256::digest(format!("{} {}", sh.address, signin::Auth::subject(&token)).as_bytes());
    let dir = sh.data.join("client").join(hex(&sum[..8]));
    let device = Arc::new(one_client::Device::new(secret, &sh.address, &dir.to_string_lossy()).keeping());
    if let Some(on) = on_device() {
        device.on(&on);
    }
    let _ = sh.device.set(device.clone());
    let in_ = match device.connect().await {
        Ok(v) => v,
        Err(e) if e.not_let_in => false,
        Err(e) => return sh.fail(e),
    };
    if !in_ {
        if let Err(e) = device.ask(&token, &host_name(), None).await {
            return sh.fail(e);
        }
        let code = device.code();
        log(format!("approve this device: {code}"));
        sh.set(|s| {
            s.phase = "approve";
            s.code = Some(code);
        });
        loop {
            let token = tauri::async_runtime::spawn_blocking(|| shell().auth.get().unwrap().token()).await.unwrap().unwrap_or_default();
            match device.wait(&token).await {
                Ok(true) => break,
                Ok(false) => tokio::time::sleep(Duration::from_secs(2)).await,
                Err(e) => return sh.fail(e),
            }
        }
    }
    let apps = match node::json(&device, "GET", "/v1/apps", None).await {
        Ok(Value::Array(a)) => a,
        Ok(_) => Vec::new(),
        Err(e) => return sh.fail(e),
    };
    log(format!("signed in and listed {} apps", apps.len()));
    let ms = sh.started.elapsed().as_millis();
    sh.set(|s| {
        s.phase = "ready";
        s.apps = apps;
        s.ready_ms = ms;
    });
    tauri::async_runtime::spawn(events());
    if let Ok(app) = std::env::var("ONE_OPEN") {
        let r = open_screen(Route { app, screen: "main".into(), params: "{}".into(), ..Default::default() }, None).await;
        if let Err(e) = r {
            log(format!("open: {e}"));
        }
    }
}

/// Forwards the node's events to every open screen, as `one.__event`.
async fn events() {
    loop {
        let Ok(d) = shell().device() else { return };
        let r = node::events(d, |e| {
            let js = format!("window.one && one.__event({e})");
            for label in shell().screens.lock().unwrap().keys() {
                if let Some(w) = shell().app.get_webview_window(label) {
                    let _ = w.eval(&js);
                }
            }
            if std::env::var("ONE_PROBE").is_ok() {
                log(format!("event: {}", e.to_string().chars().take(160).collect::<String>()));
            }
        })
        .await;
        log(format!("events: {r:?}"));
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// A screen's origin: letters, digits and dashes of its app's name, or `own`.
fn screen_host(r: &Route) -> String {
    if r.own {
        return "own".into();
    }
    let h: String = r.app.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    let h = h.trim_matches('-').to_string();
    if h.is_empty() { "app".into() } else { h }
}

/// screen://<host>/ where WebKit takes a custom scheme; WebView2 and Android's web view take them only as
/// http://screen.<host>/.
fn screen_url(host: &str) -> String {
    if cfg!(any(windows, target_os = "android")) { format!("http://screen.{host}/") } else { format!("screen://{host}/") }
}

/// The host a screen's request names, either way.
fn request_host(uri: &tauri::http::Uri) -> String {
    let h = uri.host().unwrap_or_default();
    h.strip_prefix("screen.").unwrap_or(h).to_string()
}

/// Fetches a screen's page, then shows it in a window of its own (a sheet: over `over`, its value back on close).
async fn open_screen(r: Route, over: Option<String>) -> Result<tokio::sync::oneshot::Receiver<Value>, String> {
    let sh = shell();
    let t = Instant::now();
    let html = if r.own {
        one_client::client_screen(&r.screen, &r.params)
    } else {
        node::screen(sh.device()?, &r.app, &r.screen, &r.params, Some(&r.scope)).await?
    };
    let label = format!("screen-{}", sh.next.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let host = screen_host(&r);
    sh.screens.lock().unwrap().insert(label.clone(), Arc::new(Screen { route: r.clone(), html, value: Mutex::new(Value::Null), closed: Mutex::new(Some(tx)) }));
    let url: tauri::Url = screen_url(&host).parse().unwrap();
    let own_origin = url.clone();
    let mut b = WebviewWindowBuilder::new(&sh.app, &label, WebviewUrl::CustomProtocol(url))
        .title(&r.app)
        .initialization_script(bridge::SHIM)
        // Anything that would take the screen away goes to the browser; its own page stays.
        .on_navigation(move |u| {
            if u.scheme() == own_origin.scheme() && u.host_str() == own_origin.host_str() {
                return true;
            }
            outside(u.as_str());
            false
        });
    #[cfg(desktop)]
    {
        b = b.inner_size(820.0, 680.0);
        if let Some(parent) = over.as_ref().and_then(|l| sh.app.get_webview_window(l)) {
            b = b.parent(&parent).map_err(|e| e.to_string())?.inner_size(560.0, 600.0);
        }
    }
    let _ = &over;
    let w = b.build().map_err(|e| e.to_string())?;
    log(format!("screen {}/{} window made {} ms after it was asked for", r.app, r.screen, t.elapsed().as_millis()));
    let label2 = label.clone();
    w.on_window_event(move |e| {
        if let tauri::WindowEvent::Destroyed = e {
            if let Some(s) = shell().screens.lock().unwrap().remove(&label2)
                && let Some(tx) = s.closed.lock().unwrap().take()
            {
                let _ = tx.send(s.value.lock().unwrap().clone());
            }
        }
    });
    Ok(rx)
}

fn outside(u: &str) {
    if ["https://", "http://", "mailto:", "tel:"].iter().any(|p| u.starts_with(p)) {
        let _ = shell().app.opener().open_url(u, None::<&str>);
    }
}

/// screen://<app>/: its page at /, and its bridge at /one, for the window that shows that app alone.
fn serve_screen(label: &str, req: tauri::http::Request<Vec<u8>>, respond: tauri::UriSchemeResponder) {
    let screen = shell().screens.lock().unwrap().get(label).cloned();
    let not_found = || Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()).unwrap();
    let Some(s) = screen.filter(|s| request_host(req.uri()) == screen_host(&s.route)) else {
        if std::env::var("ONE_PROBE").is_ok() {
            log(format!("refused {} {} from {label}", req.method(), req.uri()));
        }
        return respond.respond(not_found());
    };
    match (req.method().as_str(), req.uri().path()) {
        ("GET", "/") => respond.respond(Response::builder().header("content-type", "text/html; charset=utf-8").body(s.html.clone().into_bytes()).unwrap()),
        ("POST", "/one") => {
            let label = label.to_string();
            tauri::async_runtime::spawn(async move {
                let reply = match serde_json::from_slice::<Value>(req.body()) {
                    Ok(m) if m["type"] == "probe" => {
                        log(format!("probe {label}: {}", m["result"]));
                        json!({ "ok": null })
                    }
                    Ok(m) => {
                        let natives = Natives { label: label.clone(), route: s.route.clone() };
                        match shell().device() {
                            Ok(d) => match bridge::answer(d, &natives, &s.route, &shell().data.join("kept"), m).await {
                                Ok(v) => json!({ "ok": v }),
                                Err(e) => json!({ "error": e }),
                            },
                            Err(e) => json!({ "error": e }),
                        }
                    }
                    Err(e) => json!({ "error": e.to_string() }),
                };
                respond.respond(Response::builder().header("content-type", "application/json").body(serde_json::to_vec(&reply).unwrap()).unwrap());
            });
        }
        _ => respond.respond(not_found()),
    }
}

/// bridge::Natives for one screen's window: Tauri's menus, dialogs, clipboard, notifications and opener.
struct Natives {
    label: String,
    route: Route,
}

impl Natives {
    fn window(&self) -> Option<WebviewWindow> {
        shell().app.get_webview_window(&self.label)
    }
}

impl bridge::Natives for Natives {
    async fn menu(&self, items: Vec<MenuItem>, x: f64, y: f64) -> Option<usize> {
        let w = self.window()?;
        let app = &shell().app;
        let n = shell().next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        shell().menus.lock().unwrap().insert(format!("menu-{n}"), tx);
        let checks = items.iter().any(|i| i.checked);
        let menu = tauri::menu::Menu::new(app).ok()?;
        for (i, it) in items.iter().enumerate() {
            if i > 0 && (it.section || (items[i - 1].row && !it.row)) {
                let _ = menu.append(&tauri::menu::PredefinedMenuItem::separator(app).ok()?);
            }
            let id = format!("menu-{n}-{i}");
            // No destructive style in Tauri's menus (muda): a danger item is only its words.
            if checks {
                let _ = menu.append(&tauri::menu::CheckMenuItem::with_id(app, id, &it.title, true, it.checked, None::<&str>).ok()?);
            } else {
                let _ = menu.append(&tauri::menu::MenuItem::with_id(app, id, &it.title, true, None::<&str>).ok()?);
            }
        }
        let shown = {
            let w = w.clone();
            let (done, wait) = tokio::sync::oneshot::channel();
            let _ = w.clone().run_on_main_thread(move || {
                let _ = done.send(w.popup_menu_at(&menu, tauri::LogicalPosition::new(x, y)));
            });
            wait.await
        };
        if let Ok(Err(e)) = shown {
            log(format!("menu: {e}"));
        }
        // macOS and Windows show a menu until it's gone, so its click, if any, comes now or a moment after; GTK's
        // returns at once, so a menu there is waited on until chosen (the spike's limit: Escape isn't heard).
        let wait = if cfg!(target_os = "linux") { Duration::from_secs(60) } else { Duration::from_millis(300) };
        let chosen = tokio::time::timeout(wait, rx).await.ok().and_then(Result::ok);
        shell().menus.lock().unwrap().remove(&format!("menu-{n}"));
        chosen
    }

    async fn alert(&self, title: String, text: String, action: String, danger: bool) -> bool {
        let app = shell().app.clone();
        let parent = self.window();
        tauri::async_runtime::spawn_blocking(move || {
            let mut d = app.dialog().message(text).title(title);
            if let Some(p) = &parent {
                d = d.parent(p);
            }
            if action.is_empty() {
                d.buttons(MessageDialogButtons::Ok).blocking_show();
                return false;
            }
            // No destructive button in Tauri's dialogs: danger shows as a warning dialog.
            if danger {
                d = d.kind(MessageDialogKind::Warning);
            }
            d.buttons(MessageDialogButtons::OkCancelCustom(action, "Cancel".into())).blocking_show()
        })
        .await
        .unwrap_or(false)
    }

    fn copy(&self, text: &str) -> Result<(), String> {
        shell().app.clipboard().write_text(text).map_err(|e| e.to_string())
    }

    fn link(&self, url: &str) {
        outside(url)
    }

    fn title(&self, text: &str, name: &str) {
        let t = if !name.is_empty() { name } else if !text.is_empty() { text } else { &self.route.app };
        if let Some(w) = self.window() {
            let _ = w.set_title(t);
        }
    }

    async fn open(&self, to: Route, sheet: bool) -> Result<Value, String> {
        let rx = open_screen(to, sheet.then(|| self.label.clone())).await?;
        if !sheet {
            return Ok(Value::Null);
        }
        Ok(rx.await.unwrap_or(Value::Null))
    }

    fn close(&self, value: Value) {
        if let Some(s) = shell().screens.lock().unwrap().get(&self.label) {
            *s.value.lock().unwrap() = value;
        }
        if let Some(w) = self.window() {
            let _ = w.close();
        }
    }

    async fn notify(&self) -> String {
        let n = shell().app.notification();
        let state = match n.permission_state() {
            Ok(PermissionState::Granted) => PermissionState::Granted,
            _ => n.request_permission().unwrap_or(PermissionState::Denied),
        };
        match state {
            PermissionState::Granted => {
                let _ = n.builder().title("The One App").body("Notifications are on.").show();
                "granted".into()
            }
            PermissionState::Denied => "denied".into(),
            _ => "default".into(),
        }
    }

    async fn save(&self, name: String, data: Vec<u8>) -> Result<(), String> {
        let app = shell().app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let Some(path) = app.dialog().file().set_file_name(&name).blocking_save_file() else { return Ok(()) };
            let path = path.into_path().map_err(|e| e.to_string())?;
            std::fs::write(path, data).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    fn preview(&self, name: &str, data: &[u8]) -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!("one-preview-{}", shell().next.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(name);
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        shell().app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
    }

    async fn sign_out(&self) -> Result<(), String> {
        do_sign_out().await
    }

    fn device(&self) -> &'static str {
        match std::env::consts::OS {
            "macos" => "Mac",
            "windows" => "Windows",
            "ios" => "iPhone",
            "android" => "Android",
            _ => "Linux",
        }
    }
}

/// A loopback port an agent drives the spike by (`ONE_CONTROL=<port>`): `POST /eval/<label>` runs its body in that
/// window's page; `GET /windows` lists them. Never in a build people use.
fn control(port: u16) {
    use std::io::{BufRead, BufReader, Read, Write};
    let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) else { return log("control: port taken") };
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
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
            let path = line.split(' ').nth(1).unwrap_or_default().to_string();
            let out = if let Some(label) = path.strip_prefix("/eval/") {
                match shell().app.get_webview_window(label) {
                    Some(w) => w.eval(String::from_utf8_lossy(&body)).map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()),
                    None => "no such window".into(),
                }
            } else {
                serde_json::to_string(&shell().app.webview_windows().keys().collect::<Vec<_>>()).unwrap()
            };
            let mut s = &s;
            let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}", out.len());
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started = Instant::now();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![state, open, sign_out])
        .register_asynchronous_uri_scheme_protocol("screen", |ctx, req, respond| serve_screen(ctx.webview_label(), req, respond))
        .on_menu_event(|_, e| {
            let id = e.id().0.as_str();
            if let Some((menu, i)) = id.rsplit_once('-')
                && let Some(tx) = shell().menus.lock().unwrap().remove(menu)
            {
                let _ = tx.send(i.parse().unwrap_or(0));
            }
        })
        .setup(move |app| {
            let address = std::env::var("ONE_ADDRESS").unwrap_or_else(|_| option_env!("ONE_ADDRESS").unwrap_or_default().to_string());
            let data = app.path().app_data_dir()?.join("one");
            let sh = Arc::new(Shell {
                app: app.handle().clone(),
                address: address.trim_end_matches('/').into(),
                data,
                started,
                auth: OnceLock::new(),
                device: OnceLock::new(),
                state: Mutex::new(State { phase: "starting", ..Default::default() }),
                screens: Mutex::default(),
                menus: Mutex::default(),
                next: AtomicU64::new(1),
            });
            let _ = SHELL.set(sh);
            if std::env::args().any(|a| a == "--forget") {
                let a = shell().address.clone();
                for k in [format!("device {a}"), format!("{a} refresh"), format!("{a} client")] {
                    secrets::delete(&k);
                }
                log("forgot this address's device key and sign-in");
                std::process::exit(0);
            }
            if let Some(p) = std::env::var("ONE_CONTROL").ok().and_then(|p| p.parse().ok()) {
                control(p);
            }
            let home = WebviewWindowBuilder::new(app, "home", WebviewUrl::App("index.html".into())).title("The One App");
            #[cfg(desktop)]
            let home = home.inner_size(900.0, 640.0);
            home.build()?;
            log("home window made");
            #[cfg(desktop)]
            tray(app)?;
            tauri::async_runtime::spawn(start());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the shell");
}

/// The menu bar item (macOS) or tray icon: Open, Quit.
#[cfg(desktop)]
fn tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    let open = MenuItem::with_id(app, "tray-open", "Open The One App", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    tauri::tray::TrayIconBuilder::with_id("one")
        .icon(app.default_window_icon().unwrap().clone())
        .icon_as_template(true)
        .tooltip("The One App")
        .menu(&menu)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "tray-open" => {
                if let Some(w) = app.get_webview_window("home") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    log("tray item made");
    Ok(())
}
