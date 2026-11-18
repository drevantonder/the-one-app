//! A computer's app windows: each app opens in a window of its own, in a process of its own (`host`), as an app
//! the system lists by its own name and icon, and the app's own window keeps its Home Screen. They have no client of
//! their own: each asks through this one, over a local channel that only this user on this machine reaches, a Unix
//! socket (`address`), or on Windows a named pipe, so there's one connection to the network, one sign-in
//! and one set of what's kept. What one asks is what the Home Screen's page asks (`invoke`), by the same commands, and
//! it hears the same events.
//!
//! Each line on the channel is a JSON message. An app's window says first which app it is (`hello`), then asks (`id`,
//! `command`, `args`) and loads files (`id`, `file`); this side answers each (`id`, `result` or `error`), and tells it
//! what's said (`event`, `payload`). One app has one window: a second start of it brings the first forward and ends.
//! Ending this program ends the channel, and every app's window with it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use data_encoding::BASE64;
use serde_json::{Value, json};
use tauri::{AppHandle, Listener, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// The events the Home Screen's page hears that an app's window hears too. A `one:` link (`open`) is told only to the
/// window of the app it opens.
const EVENTS: [&str; 6] = ["changed", "connection", "behind", "stale", "session", "theme"];

/// The app windows that are open or starting, by the app they show (`key`), as the Home Screen's page names an open
/// app: `<subject>|<entrypoint>`.
#[derive(Default)]
pub struct Hosts {
    open: Mutex<HashMap<String, Host>>,
    next: AtomicU64,
}

struct Host {
    /// Where to send it a message, once it's said hello; until then, the link it opens once it has.
    to: Option<UnboundedSender<Value>>,
    link: Option<String>,
}

/// Where the channel is, named after the client's folder, so a client of each folder has its own: a Unix socket in
/// this user's own folder for them (Linux's `XDG_RUNTIME_DIR`, a Mac's `TMPDIR`), as a socket's address is short, and
/// on Windows a named pipe.
pub fn address(home: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    home.hash(&mut hasher);
    let name = format!("{}-{:016x}", ::client::NAME, hasher.finish());
    #[cfg(unix)]
    {
        let folder = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
        folder.join(format!("{name}.sock")).to_string_lossy().into_owned()
    }
    #[cfg(windows)]
    format!(r"\\.\pipe\{name}")
}

/// Serves the channel for as long as this program runs, and tells every app window what the Home Screen hears.
pub fn serve(app: &AppHandle, home: &std::path::Path) -> anyhow::Result<()> {
    app.manage(Hosts::default());
    for event in EVENTS {
        let handle = app.clone();
        app.listen_any(event, move |heard| {
            let payload: Value = serde_json::from_str(heard.payload()).unwrap_or(Value::Null);
            handle.state::<Hosts>().tell_all(json!({"event": event, "payload": payload}));
        });
    }
    let address = address(home);
    // Its listener is Tauri's runtime's, which setup runs outside of.
    let runtime = tauri::async_runtime::handle();
    let _inside = runtime.inner().enter();
    #[cfg(unix)]
    {
        // A socket left by a program that ended without removing it is in the way of a new one.
        let _ = std::fs::remove_file(&address);
        let listener = tokio::net::UnixListener::bind(&address)?;
        // Only this user reaches it.
        std::fs::set_permissions(&address, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tauri::async_runtime::spawn(talk(app.clone(), stream));
            }
        });
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        // A named pipe takes one client per instance, so another waits for the next as each connects. Its default
        // security lets only this user, the system and administrators write to it, and no other machine reaches it.
        let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&address)?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while server.connect().await.is_ok() {
                let Ok(next) = ServerOptions::new().reject_remote_clients(true).create(&address) else { return };
                tauri::async_runtime::spawn(talk(app.clone(), std::mem::replace(&mut server, next)));
            }
        });
    }
    Ok(())
}

impl Hosts {
    fn tell_all(&self, message: Value) {
        for host in self.open.lock().expect("no host panics").values() {
            if let Some(to) = &host.to {
                let _ = to.send(message.clone());
            }
        }
    }

    /// Whether any app's window is open.
    pub fn any(&self) -> bool {
        !self.open.lock().expect("no host panics").is_empty()
    }

    /// Has every app's window end, as the device's session starts again.
    pub fn close_all(&self) {
        self.tell_all(json!({"quit": true}));
    }
}

/// Brings the Home Screen forward, as a link to it from an app's window does.
#[tauri::command]
pub fn front(app: AppHandle) {
    crate::front(&app)
}

/// Opens an app in its own window, at `link` (`<app>/<page>[#<fragment>]`, as a `one:` link says it), or brings its
/// window forward there if it's open.
#[tauri::command]
pub fn open_window(app: AppHandle, entrypoint: String, subject: Option<String>, link: String) -> Result<(), String> {
    let hosts = app.state::<Hosts>();
    let key = format!("{}|{entrypoint}", subject.as_deref().unwrap_or_default());
    let mut open = hosts.open.lock().expect("no host panics");
    if let Some(host) = open.get_mut(&key) {
        match &host.to {
            Some(to) => drop(to.send(json!({"event": "open", "payload": link}))),
            None => host.link = Some(link),
        }
        return Ok(());
    }
    let mut command = std::process::Command::new(crate::shortcuts::program(&app, &entrypoint).map_err(|error| error.to_string())?);
    command.env("ONE_APP", &entrypoint).env("ONE_OPEN", &link).stdin(std::process::Stdio::null());
    // A window of its own, and no console's.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    match &subject {
        Some(subject) => command.env("ONE_SUBJECT", subject),
        None => command.env_remove("ONE_SUBJECT"),
    };
    let mut child = command.spawn().map_err(|error| format!("{entrypoint} didn't open: {error}"))?;
    open.insert(key.clone(), Host { to: None, link: None });
    drop(open);
    // It's waited for, so it's gone once it ends; one that never said hello is forgotten then.
    let handle = app.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        let hosts = handle.state::<Hosts>();
        let mut open = hosts.open.lock().expect("no host panics");
        if open.get(&key).is_some_and(|host| host.to.is_none()) {
            open.remove(&key);
        }
    });
    Ok(())
}

/// Talks with one app's window until it ends.
async fn talk(app: AppHandle, stream: impl AsyncRead + AsyncWrite + Send + 'static) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let (to, mut out) = unbounded_channel::<Value>();
    tauri::async_runtime::spawn(async move {
        while let Some(message) = out.recv().await {
            let mut line = message.to_string();
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                return;
            }
        }
    });
    let hosts = app.state::<Hosts>();
    let label = format!("host-{}", hosts.next.fetch_add(1, Ordering::Relaxed));
    let mut key = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(hello) = message.get("hello") {
            let entrypoint = hello["entrypoint"].as_str().unwrap_or_default();
            let subject = hello["subject"].as_str();
            let at = format!("{}|{entrypoint}", subject.unwrap_or_default());
            let mut open = hosts.open.lock().expect("no host panics");
            match open.get_mut(&at) {
                // Its window is open already, as when its app was opened from the system while it was: that one comes
                // forward, at the link this one was opened at.
                Some(Host { to: Some(other), .. }) => {
                    let _ = other.send(json!({"event": "open", "payload": hello["link"]}));
                    let _ = to.send(json!({"quit": true}));
                    return;
                }
                Some(host) => {
                    let link = host.link.take();
                    host.to = Some(to.clone());
                    if let Some(link) = link {
                        let _ = to.send(json!({"event": "open", "payload": link}));
                    }
                }
                None => drop(open.insert(at.clone(), Host { to: Some(to.clone()), link: None })),
            }
            key = Some(at);
            continue;
        }
        let Some(id) = message.get("id").cloned() else { continue };
        let (app, to, label) = (app.clone(), to.clone(), label.clone());
        tauri::async_runtime::spawn(async move {
            let answer = match message.get("file").and_then(Value::as_str) {
                Some(address) => file(&app, address).await,
                None => command(&app, &label, message["command"].as_str().unwrap_or_default(), message["args"].clone()).await,
            };
            let _ = to.send(match answer {
                Ok(result) => json!({"id": id, "result": result}),
                Err(error) => json!({"id": id, "error": error}),
            });
        });
    }
    if let Some(key) = key {
        hosts.open.lock().expect("no host panics").remove(&key);
    }
    if let Some(views) = app.try_state::<crate::Views>() {
        views.shown(&label, false);
    }
    // The last of the app's windows gone, with its Home Screen put away, the app ends.
    if !hosts.any() && !app.get_webview_window("main").is_some_and(|window| window.is_visible().unwrap_or(false)) {
        app.exit(0)
    }
}

/// One file of an app's, for an app's window, as the Home Screen's page loads one (`Views::file`).
async fn file(app: &AppHandle, address: &str) -> Result<Value, String> {
    let views = app.try_state::<crate::Views>().ok_or("the app hasn't opened")?;
    let address: tauri::http::Uri = address.parse().map_err(|_| "that isn't an address")?;
    let (kind, file) = views.file(&address).await.map_err(|error| format!("{error:#}"))?;
    Ok(json!({"kind": kind, "body": BASE64.encode(&file)}))
}

/// What an app's window asks, by the Home Screen page's commands (`invoke`), answered as they are.
async fn command(app: &AppHandle, label: &str, command: &str, args: Value) -> Result<Value, String> {
    use crate::*;
    let text = |name: &str| args[name].as_str().map(str::to_owned);
    // Where the device is, and the name of the app the window asking is, as its shortcut has it.
    if command == "start" {
        let mut started = start(app.clone()).await.map_err(|()| "the app hasn't opened".to_owned())?;
        started["name"] = json!(text("entrypoint").and_then(|entrypoint| crate::shortcuts::name(&entrypoint)));
        return Ok(started);
    }
    if command == "retry" {
        return retry(app.clone()).await.map(|()| Value::Null);
    }
    if command == "open_window" {
        return open_window(app.clone(), text("entrypoint").unwrap_or_default(), text("subject"), text("link").unwrap_or_default()).map(|()| Value::Null);
    }
    if command == "front" {
        crate::front(app);
        return Ok(Value::Null);
    }
    if command == "theme" {
        crate::chose(app, serde_json::from_value(args["theme"].clone()).unwrap_or_default());
        return Ok(Value::Null);
    }
    if command == "open_url" {
        use tauri_plugin_opener::OpenerExt;
        let url = text("url").filter(|url| url.starts_with("https://") || url.starts_with("http://")).ok_or("that isn't a web address")?;
        return app.opener().open_url(url, None::<&str>).map(|()| Value::Null).map_err(|error| error.to_string());
    }
    let views = app.try_state::<Views>().ok_or("the app hasn't opened")?;
    let done = |()| Value::Null;
    match command {
        "apps" => apps(app.clone(), views, text("subject")).await,
        "request" => {
            request(views, text("view").unwrap_or_default(), text("method").unwrap_or_default(), args["params"].clone()).await
        }
        "refresh" => refresh(views, text("view").unwrap_or_default()).await.map(done).map_err(|()| String::new()),
        "client" => Ok(client(views)),
        "serve" => serve(views, args["on"].as_bool() == Some(true)).await.map(Value::from),
        "update" => update(app.clone(), views).await.map(Value::from),
        "visible" => {
            views.shown(label, args["onScreen"].as_bool() == Some(true));
            if args["onScreen"].as_bool() == Some(true) {
                views.caught_up().await
            }
            Ok(Value::Null)
        }
        "upload" => {
            let body = BASE64.decode(args["body"].as_str().unwrap_or_default().as_bytes()).map_err(|error| error.to_string())?;
            let upload = views.client.upload(args["app"].as_str().unwrap_or_default(), body).await;
            upload.map(|upload| json!({"upload": upload})).map_err(|error| error.to_string())
        }
        "recovery_key" => recovery_key(views, text("subject")).await.map(Value::from),
        "recover" => recover(views, text("key").unwrap_or_default(), text("subject")).await.map(done),
        "close" => Ok(done(close(views, text("view").unwrap_or_default()))),
        "reconnect" => Ok(done(reconnect(views))),
        "sign_out" => sign_out(views).await.map(done),
        "push" => push().await.map_err(|()| String::new()),
        "listen" => Ok(done(listen(app.clone(), views))),
        "opened" => Ok(done(opened(views, text("view").unwrap_or_default(), args["ms"].as_u64().unwrap_or_default() as u32))),
        "report" => Ok(done(report(views, args["record"].clone()))),
        _ => Err(format!("An app's window has no {command}")),
    }
}
