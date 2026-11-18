//! A computer's app windows: each app opens in a window of its own, in a process of its own (`host`), as an app
//! the system lists by its own name and icon, and the launcher (`computer`) starts them. They have no client of
//! their own: each asks through this one, over a local channel that only this user on this machine reaches, a Unix
//! socket (`address`), or on Windows a named pipe, so there's one connection to the network, one sign-in
//! and one set of what's kept. What one asks is what the launcher's page asks (`invoke`), by the same commands
//! (`command`), and it hears the same events.
//!
//! Each line on the channel is a JSON message. An app's window says first which app it is (`hello`), then asks (`id`,
//! `command`, `args`) and loads files (`id`, `file`); this side answers each (`id`, `result` or `error`), and tells it
//! what's said (`event`, `payload`). One app has one window: a second start of it brings the first forward and ends.
//! A second start of the app itself says only `again`, with the `one:` link it was started with, if any, and ends.
//! Ending this program ends the channel, and every app's window with it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use data_encoding::BASE64;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::computer;

/// The events the launcher's page hears that an app's window hears too. A `one:` link (`open`) is told only to the
/// launcher, which opens the app's window.
pub const EVENTS: [&str; 6] = ["changed", "connection", "behind", "stale", "session", "theme"];

/// The app windows that are open or starting, by the app they show (`key`), as the launcher's page names an open
/// app: `<subject>|<entrypoint>`.
static HOSTS: LazyLock<Hosts> = LazyLock::new(Hosts::default);

#[derive(Default)]
struct Hosts {
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

/// Tells the app already running for `home`, if there is one, that it was started again, with the `one:` link it was
/// started with: the Dock's or taskbar's icon clicked, or a link opened. → whether it was told, so this start ends.
pub fn again(home: &std::path::Path, link: Option<&str>) -> bool {
    use std::io::Write;
    let line = format!("{}\n", json!({"again": {"link": link}}));
    #[cfg(unix)]
    let connected = std::os::unix::net::UnixStream::connect(address(home));
    #[cfg(windows)]
    let connected = std::fs::OpenOptions::new().read(true).write(true).open(address(home));
    connected.and_then(|mut running| running.write_all(line.as_bytes())).is_ok()
}

/// Serves the channel for as long as this program runs, on its runtime.
pub fn serve(home: &std::path::Path) -> anyhow::Result<()> {
    let address = address(home);
    let _inside = computer::runtime().enter();
    #[cfg(unix)]
    {
        // A socket left by a program that ended without removing it is in the way of a new one.
        let _ = std::fs::remove_file(&address);
        let listener = tokio::net::UnixListener::bind(&address)?;
        // Only this user reaches it.
        std::fs::set_permissions(&address, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        crate::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                crate::spawn(talk(stream));
            }
        });
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        // A named pipe takes one client per instance, so another waits for the next as each connects. Its default
        // security lets only this user, the system and administrators write to it, and no other machine reaches it.
        let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&address)?;
        crate::spawn(async move {
            while server.connect().await.is_ok() {
                let Ok(next) = ServerOptions::new().reject_remote_clients(true).create(&address) else { return };
                crate::spawn(talk(std::mem::replace(&mut server, next)));
            }
        });
    }
    Ok(())
}

/// Tells every app's window `message`.
pub fn tell_all(message: Value) {
    for host in HOSTS.open.lock().expect("no host panics").values() {
        if let Some(to) = &host.to {
            let _ = to.send(message.clone());
        }
    }
}

/// Has every app's window end, as the device's session starts again, or the app does.
pub fn close_all() {
    tell_all(json!({"quit": true}));
}

/// The apps whose windows are open, each as `open_window` opens it: its entrypoint and who it's for.
pub fn open_now() -> Vec<(String, Option<String>)> {
    let open = HOSTS.open.lock().expect("no host panics");
    open.keys()
        .filter_map(|key| key.split_once('|'))
        .map(|(subject, entrypoint)| (entrypoint.to_owned(), (!subject.is_empty()).then(|| subject.to_owned())))
        .collect()
}

/// Opens an app in its own window, at `link` (`<app>/<page>[#<fragment>]`, as a `one:` link says it), or brings its
/// window forward there if it's open.
pub fn open_window(entrypoint: String, subject: Option<String>, link: String) -> Result<(), String> {
    let key = format!("{}|{entrypoint}", subject.as_deref().unwrap_or_default());
    #[cfg(target_os = "macos")]
    {
        let id = crate::shortcuts::app_id(&entrypoint, subject.as_deref());
        computer::on_main(move || yield_activation(&id));
    }
    let mut open = HOSTS.open.lock().expect("no host panics");
    if let Some(host) = open.get_mut(&key) {
        match &host.to {
            Some(to) => drop(to.send(json!({"event": "open", "payload": link}))),
            None => host.link = Some(link),
        }
        return Ok(());
    }
    let mut command = std::process::Command::new(crate::shortcuts::program(&entrypoint).map_err(|error| error.to_string())?);
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
    std::thread::spawn(move || {
        let _ = child.wait();
        let mut open = HOSTS.open.lock().expect("no host panics");
        if open.get(&key).is_some_and(|host| host.to.is_none()) {
            open.remove(&key);
        }
    });
    Ok(())
}

/// Lets the app `id` come in front of this one, as a person opened it from here: since macOS 14 an app comes in front
/// only where the one in front lets it (cooperative activation), so an app's window would otherwise open behind the
/// Home Screen's, and the Home Screen's behind an app's. AppKit's, on the main thread.
#[cfg(target_os = "macos")]
pub(crate) fn yield_activation(id: &str) {
    use objc2::runtime::{AnyClass, AnyObject};
    let (Some(application), Some(id)) = (AnyClass::get(c"NSApplication"), ns_string(id)) else { return };
    unsafe {
        let shared: *mut AnyObject = objc2::msg_send![application, sharedApplication];
        let yields: bool = objc2::msg_send![shared, respondsToSelector: objc2::sel!(yieldActivationToApplicationWithBundleIdentifier:)];
        if yields {
            let () = objc2::msg_send![shared, yieldActivationToApplicationWithBundleIdentifier: id];
        }
    }
}

/// The window's title bar was double-clicked: does what that does on a Mac, as its person set it in System Settings (Desktop & Dock,
/// "Double-click a window's title bar to"): zoom, which is what's set until they change it, minimize, or nothing. A
/// window with no title band is double-clicked at its page's top (`title_bar_double_click`). AppKit's, on the main
/// thread.
#[cfg(target_os = "macos")]
pub(crate) fn double_clicked(window: *mut std::ffi::c_void) {
    use objc2::runtime::{AnyClass, AnyObject};
    let (Some(defaults), Some(key)) = (AnyClass::get(c"NSUserDefaults"), ns_string("AppleActionOnDoubleClick")) else { return };
    let window = window.cast::<AnyObject>();
    let nothing: *mut AnyObject = std::ptr::null_mut();
    unsafe {
        let defaults: *mut AnyObject = objc2::msg_send![defaults, standardUserDefaults];
        let action: *mut AnyObject = objc2::msg_send![defaults, stringForKey: key];
        let action: *const std::ffi::c_char = if action.is_null() { std::ptr::null() } else { objc2::msg_send![action, UTF8String] };
        match (!action.is_null()).then(|| std::ffi::CStr::from_ptr(action).to_bytes()) {
            Some(b"Minimize") => objc2::msg_send![window, performMiniaturize: nothing],
            Some(b"None") => {}
            _ => objc2::msg_send![window, performZoom: nothing],
        }
    }
}

/// `text` as an `NSString`.
#[cfg(target_os = "macos")]
fn ns_string(text: &str) -> Option<*mut objc2::runtime::AnyObject> {
    let (string, text) = (objc2::runtime::AnyClass::get(c"NSString")?, std::ffi::CString::new(text).ok()?);
    Some(unsafe { objc2::msg_send![string, stringWithUTF8String: text.as_ptr()] })
}

/// Talks with one app's window until it ends, or with a second start of the app (`again`).
async fn talk(stream: impl AsyncRead + AsyncWrite + Send + 'static) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let (to, mut out) = unbounded_channel::<Value>();
    crate::spawn(async move {
        while let Some(message) = out.recv().await {
            let mut line = message.to_string();
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                return;
            }
        }
    });
    let label = format!("host-{}", HOSTS.next.fetch_add(1, Ordering::Relaxed));
    let mut key = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(again) = message.get("again") {
            computer::again(again["link"].as_str().map(str::to_owned));
            return;
        }
        if let Some(hello) = message.get("hello") {
            let entrypoint = hello["entrypoint"].as_str().unwrap_or_default();
            let subject = hello["subject"].as_str();
            let at = format!("{}|{entrypoint}", subject.unwrap_or_default());
            let mut open = HOSTS.open.lock().expect("no host panics");
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
        let (to, label) = (to.clone(), label.clone());
        crate::spawn(async move {
            let answer = match message.get("file").and_then(Value::as_str) {
                Some(address) => file(address).await,
                None => command(&label, message["command"].as_str().unwrap_or_default(), message["args"].clone()).await,
            };
            let _ = to.send(match answer {
                Ok(result) => json!({"id": id, "result": result}),
                Err(error) => json!({"id": id, "error": error}),
            });
        });
    }
    if let Some(key) = key {
        HOSTS.open.lock().expect("no host panics").remove(&key);
    }
    if let Some(views) = crate::app().views.get() {
        views.shown(&label, false);
    }
}

/// One file of an app's, for an app's window, as the launcher's page loads one (`Views::file`).
async fn file(address: &str) -> Result<Value, String> {
    let (kind, file) = crate::views()?.file(address).await.map_err(|error| format!("{error:#}"))?;
    Ok(json!({"kind": kind, "body": BASE64.encode(&file)}))
}

/// What the launcher's page (`main`) or an app's window (`label`) asks, by the page's commands (`invoke`).
pub(crate) async fn command(label: &str, command: &str, args: Value) -> Result<Value, String> {
    use crate::*;
    let text = |name: &str| args[name].as_str().map(str::to_owned);
    let launcher = label == "main";
    let done = |()| Value::Null;
    match command {
        // Where the device is, and for an app's window the name of its app, as its shortcut has it.
        "start" => {
            let mut started = start().await?;
            if !launcher {
                started["name"] = json!(text("entrypoint").and_then(|entrypoint| crate::shortcuts::name(&entrypoint)));
            }
            Ok(started)
        }
        // An app opened from the launcher comes in front, and the launcher goes.
        "open_window" => {
            open_window(text("entrypoint").unwrap_or_default(), text("subject"), text("link").unwrap_or_default())?;
            if launcher {
                computer::hide(false);
            }
            Ok(Value::Null)
        }
        // An app's Home, or Window → Home Screen.
        "front" => Ok(done(computer::summon())),
        // Escape on the launcher, or its page done with it (`leave`).
        "hide" => Ok(done(computer::hide(true))),
        "theme" if launcher => Ok(done(theme(serde_json::from_value(args["theme"].clone()).unwrap_or_default()))),
        "theme" => Ok(done(chose(serde_json::from_value(args["theme"].clone()).unwrap_or_default()))),
        "open_url" => {
            let url = text("url").filter(|url| url.starts_with("https://") || url.starts_with("http://")).ok_or("that isn't a web address")?;
            app().shell.open_url(&url).map(done)
        }
        "retry" => retry().await.map(done),
        "apps" => apps(text("subject")).await,
        "request" => request(text("view").unwrap_or_default(), text("method").unwrap_or_default(), args["params"].clone()).await,
        "refresh" => refresh(text("view").unwrap_or_default()).await.map(done),
        "client" => client(),
        "serve" => serve(args["on"].as_bool() == Some(true)).await.map(Value::from),
        "update" => update().await.map(Value::from),
        "visible" => {
            let views = views()?;
            views.shown(label, args["onScreen"].as_bool() == Some(true));
            if args["onScreen"].as_bool() == Some(true) {
                views.caught_up().await
            }
            Ok(Value::Null)
        }
        "upload" => {
            let body = BASE64.decode(args["body"].as_str().unwrap_or_default().as_bytes()).map_err(|error| error.to_string())?;
            upload_file(args["app"].as_str().unwrap_or_default(), body).await
        }
        "recovery_key" => recovery_key(text("subject")).await.map(Value::from),
        "recover" => recover(text("key").unwrap_or_default(), text("subject")).await.map(done),
        "close" => close(text("view").unwrap_or_default()).map(done),
        "reconnect" => reconnect().map(done),
        "sign_out" => sign_out().await.map(done),
        "push" => push().await,
        "listen" => listen().map(done),
        "opened" => opened(text("view").unwrap_or_default(), args["ms"].as_u64().unwrap_or_default() as u32).map(done),
        "report" => report(args["record"].clone()).map(done),
        // A phone's status bar and haptics, and the web view's own back and forward, which the launcher has none of.
        "status_bar" | "navigation_gestures" => Ok(Value::Null),
        command if command.starts_with("plugin:") => Ok(Value::Null),
        _ => Err(format!("The app has no {command}")),
    }
}
