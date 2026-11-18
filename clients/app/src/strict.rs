//! An app's own window (`host`) drawn natively, where its entrypoint's page is in strict mode (`host` in
//! core/ui/src/strict/protocol.ts) and of a version this program speaks (`strict::VERSION`): its JS runs in a QuickJS
//! sandbox and GPUI draws what it sends (the `strict` crate), in place of a web view. Any other app's window is a web
//! view, as before.
//!
//! It asks the app's window what a page would, over the same channel (`hosts`): its files, and what the page asks of
//! its client, which an MCP App asks its host (JSON-RPC). What a web view's window page answers itself
//! (`respond` in page/client.js) is answered here: `ui/initialize`, `ping` and `one/client`'s Colour Mode; the rest
//! is the client's, as from the app (`request`).
//!
//! Each window serves the inspector's protocol (`strict::inspect`) on a socket of its own beside the app's window's
//! (`socket`), which the app's client offers the network as the Inspector app's tools of this device (`Inspector`), so
//! an agent anywhere in the network reads and drives a strict window as it would one of its own.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use data_encoding::BASE64;
use serde_json::{Value, json};
use strict::gpui::{self, AppContext as _, WindowAppearance};
use strict::js::{Clock, Files, Sandbox};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use url::Url;

/// What the app's window has of this app: whether its page is strict, and where its page's module is.
struct Page {
    module: Url,
    trusted: bool,
}

/// Something asked of the app's window that it hasn't answered: a file the sandbox waits for, or what the app's JS
/// asked, by its JSON-RPC id.
enum Asked {
    File(mpsc::Sender<Result<Vec<u8>, String>>),
    Message(Value),
    Start(mpsc::Sender<Value>),
}

/// What the window hears from the app's window, and from the inspector's clients, on threads of their own.
enum Heard {
    /// The answer to what the app asked: its JSON-RPC id, and the result or the error.
    Answer(Value, Result<Value, Value>),
    Event(String, Value),
    /// An inspector's request, and where its answer goes.
    Inspect(strict::inspect::Request, tokio::sync::oneshot::Sender<Value>),
    Quit,
}

/// Opens `entrypoint`'s window natively if its page is strict, and runs until it's closed. `None`: it isn't, or it
/// couldn't start, and opens in a web view.
pub fn run(started: Instant, home: &std::path::Path, entrypoint: &str, subject: Option<&str>, link: &str, class: &str) -> Option<ExitCode> {
    let address = crate::hosts::address(home);
    let page = match probe(&address, entrypoint, subject) {
        Ok(page) => page?,
        Err(error) => {
            eprintln!("{entrypoint} opens in a web view: {error:#}");
            return None;
        }
    };
    match window(started, home, &address, page, entrypoint, subject, link, class) {
        Ok(exit) => Some(exit),
        Err(error) => {
            eprintln!("{entrypoint} opens in a web view: {error:#}");
            None
        }
    }
}

/// Where an app's file is for its page: `oneapp://<app>[.<subject, last part first>].localhost/<path>`, as a web
/// view's window page loads it (`Views::file`).
fn origin(entrypoint: &str, subject: Option<&str>) -> String {
    let app = entrypoint.split('/').next().unwrap_or_default();
    let subject: Vec<&str> = subject.map(|subject| subject.split('/').rev().collect()).unwrap_or_default();
    let host = std::iter::once(app).chain(subject).collect::<Vec<_>>().join(".");
    format!("oneapp://{host}.localhost/")
}

/// Asks the app's window, on a connection of its own that it closes, whether `entrypoint`'s page is strict and of a
/// version this program speaks, and if it is, which module its page loads.
fn probe(address: &str, entrypoint: &str, subject: Option<&str>) -> anyhow::Result<Option<Page>> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    runtime.block_on(async {
        let stream = crate::host::connect(address).await?;
        let (read, mut write) = tokio::io::split(stream);
        let mut lines = BufReader::new(read).lines();
        let mut ask = async |message: Value| -> anyhow::Result<Value> {
            write.write_all(format!("{message}\n").as_bytes()).await?;
            while let Some(line) = lines.next_line().await? {
                let answer: Value = serde_json::from_str(&line).unwrap_or_default();
                if answer["id"] == message["id"] {
                    return match answer.get("error") {
                        Some(error) => Err(anyhow::anyhow!("{error}")),
                        None => Ok(answer["result"].clone()),
                    };
                }
            }
            anyhow::bail!("the One App ended")
        };
        let listed = ask(json!({"id": "apps", "command": "apps", "args": {"subject": subject}})).await?;
        let (app, path) = entrypoint.split_once('/').unwrap_or((entrypoint, ""));
        let Some(resource) = listed["resources"].as_array().into_iter().flatten().find(|resource| {
            let uri = resource["uri"].as_str().unwrap_or_default();
            uri.strip_prefix(&format!("ui://{app}/")).and_then(|rest| rest.split_once('/')).is_some_and(|(_, page)| page == path)
        }) else {
            return Ok(None);
        };
        let speaks = resource["_meta"]["host"].as_u64().is_some_and(|host| host <= u64::from(strict::VERSION));
        if !speaks {
            return Ok(None);
        }
        let trusted = resource["_meta"]["trusted"] == true;
        let page = Url::parse(&origin(entrypoint, subject))?.join(entrypoint)?;
        let html = ask(json!({"id": "page", "file": page.as_str()})).await?;
        let html = BASE64.decode(html["body"].as_str().unwrap_or_default().as_bytes())?;
        let module = module_script(&String::from_utf8_lossy(&html)).ok_or_else(|| anyhow::anyhow!("its page loads no module"))?;
        Ok(Some(Page { module: page.join(&module)?, trusted }))
    })
}

/// The `src` of a page's `<script type="module">`.
fn module_script(html: &str) -> Option<String> {
    html.split("<script").skip(1).find_map(|tag| {
        let tag = &tag[..tag.find('>')?];
        if !tag.contains("type=\"module\"") {
            return None;
        }
        let src = &tag[tag.find("src=\"")? + 5..];
        Some(src[..src.find('"')?].to_owned())
    })
}

#[allow(clippy::too_many_arguments)]
fn window(
    started: Instant,
    home: &std::path::Path,
    address: &str,
    page: Page,
    entrypoint: &str,
    subject: Option<&str>,
    link: &str,
    class: &str,
) -> anyhow::Result<ExitCode> {
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let (heard_to, heard) = unbounded_channel::<Heard>();
    let socket = socket(home, std::process::id());
    if let Err(error) = inspector(&socket, heard_to.clone()) {
        eprintln!("{entrypoint}'s inspector didn't start: {error:#}");
    }
    let to = channel(address, entrypoint, subject, link, asked.clone(), heard_to)?;
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());

    // Where the device is with its network, as a page asks as it opens: the Colour Mode chosen on it, if one was.
    let (start_to, start) = mpsc::channel();
    asked.lock().expect("nothing panics holding it").insert("start".into(), Asked::Start(start_to));
    let _ = to.send(json!({"id": "start", "command": "start", "args": {"entrypoint": entrypoint}}));
    let start = start.recv()?;
    let chosen = start["theme"].as_str().map(str::to_owned);

    // The page's files, which the app's window has, each asked for as the sandbox imports it.
    let (files_to, next) = (to.clone(), std::sync::atomic::AtomicU64::new(0));
    let files_asked = asked.clone();
    let mut files: Files = Box::new(move |url: &Url| {
        let id = format!("f{}", next.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let (answer_to, answer) = mpsc::channel();
        files_asked.lock().expect("nothing panics holding it").insert(id.clone(), Asked::File(answer_to));
        let _ = files_to.send(json!({"id": id, "file": url.as_str()}));
        let file = answer.recv()?.map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(String::from_utf8(file)?)
    });
    // The design tokens, which come with the UI library, as its pages' do.
    strict::tokens::load(&files(&page.module.join("/ui/client/ui.css")?)?);
    let cache = home.join("strict").join(::client::BUILD);
    let sandbox = Sandbox::start(&page.module, files, Some(cache), Clock::real())?;

    // Its name, as the app's window knows it from its shortcut.
    let name = start["name"].as_str().map_or_else(|| entrypoint.split('/').next().unwrap_or_default().to_owned(), str::to_owned);
    let state = Arc::new(Mutex::new(State {
        chosen,
        trusted: page.trusted,
        entrypoint: entrypoint.to_owned(),
        subject: subject.map(str::to_owned),
        view,
        next: 0,
    }));
    let (class, to_window) = (class.to_owned(), to.clone());
    let entrypoint_app = entrypoint.split('/').next().unwrap_or_default().to_owned();
    let application = strict::gpui_platform::application();
    application.run(move |cx| {
        if let Err(error) = strict::add_fonts(cx.text_system()) {
            eprintln!("{error:#}");
        }
        strict::bind_keys(cx);
        cx.bind_keys([gpui::KeyBinding::new("ctrl-w", Close, None), gpui::KeyBinding::new("ctrl-q", Close, None)]);
        cx.on_action(|_: &Close, cx| cx.quit());
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = gpui::Bounds::centered(None, gpui::size(gpui::px(1024.0), gpui::px(720.0)), cx);
        let options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions { title: Some(name.into()), ..Default::default() }),
            app_id: Some(class),
            ..Default::default()
        };
        let (respond_state, respond_to, respond_asked, chosen) = (state.clone(), to_window.clone(), asked.clone(), state.clone());
        let opened = cx.open_window(options, move |window, cx| {
            cx.new(|cx| {
                let asked_by_app: strict::Asked =
                    Box::new(move |message, window, _| respond(message, &respond_state, &respond_to, &respond_asked, window));
                let mut screen = strict::Screen::new(sandbox, Some(asked_by_app), window, cx);
                screen.theme = theme(chosen.lock().expect("nothing panics holding it").chosen.as_deref());
                screen
            })
        });
        let Ok(handle) = opened else {
            eprintln!("its window didn't open");
            return cx.quit();
        };
        // How long it took to open, from this process starting, as a page tells the app's window it has drawn (`opened`).
        let (opened_to, app) = (to_window.clone(), entrypoint_app.clone());
        let _ = handle.update(cx, |_, window, _| {
            window.on_next_frame(move |_, _| {
                let ms = started.elapsed().as_millis() as u64;
                let _ = opened_to.send(json!({"id": "opened", "command": "opened", "args": {"view": app, "ms": ms}}));
                let _ = opened_to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": true}}));
            })
        });
        hear(heard, handle, state, cx);
    });
    let _ = std::fs::remove_file(&socket);
    Ok(ExitCode::SUCCESS)
}

gpui::actions!(strict_window, [Close]);

/// What the window keeps of where it is.
struct State {
    /// The Colour Mode chosen on this device, or none for as the device is.
    chosen: Option<String>,
    trusted: bool,
    entrypoint: String,
    subject: Option<String>,
    view: String,
    next: u64,
}

fn theme(chosen: Option<&str>) -> Option<WindowAppearance> {
    match chosen {
        Some("dark") => Some(WindowAppearance::Dark),
        Some("light") => Some(WindowAppearance::Light),
        _ => None,
    }
}

/// Answers what the app's JS posted its client: what the window page answers itself, here, at once, and the rest by
/// the app's window, as from the app, which comes back as it's heard (`hear`).
fn respond(
    message: Value,
    state: &Arc<Mutex<State>>,
    to: &UnboundedSender<Value>,
    asked: &Arc<Mutex<HashMap<String, Asked>>>,
    window: &gpui::Window,
) -> Option<Value> {
    let method = message["method"].as_str()?;
    // A notification, which needs no answer: none is heard here yet.
    let id = message.get("id").cloned()?;
    let params = &message["params"];
    let mut state = state.lock().expect("nothing panics holding it");
    let dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
    let shown = state.chosen.clone().unwrap_or_else(|| if dark { "dark" } else { "light" }.into());
    let answered = match method {
        "ui/initialize" => Some(Ok(json!({
            "protocolVersion": "2026-01-26",
            "hostInfo": {"name": "one", "version": "0.1.0"},
            "hostCapabilities": {"serverTools": {}, "serverResources": {}},
            "hostContext": {
                "theme": shown,
                "displayMode": "fullscreen",
                "availableDisplayModes": ["fullscreen"],
                "deviceCapabilities": {"touch": false, "hover": true},
                "safeAreaInsets": {"top": 0, "right": 0, "bottom": 0, "left": 0},
                "breadcrumb": false,
                "connection": "online",
                "updateReady": false,
            },
        }))),
        "ping" => Some(Ok(json!({}))),
        "one/client" => {
            let changes: Vec<&String> = params.as_object().into_iter().flat_map(|params| params.keys()).filter(|change| *change != "statusBar").collect();
            if !changes.is_empty() && !state.trusted {
                Some(Err(json!({"code": -32000, "message": format!("{} is installed untrusted, so it doesn't change this device.", state.entrypoint)})))
            } else if let Some(theme) = params.get("theme") {
                // Colour Mode is the device's: the app's window tells every window, this one too (`hear`).
                state.chosen = theme.as_str().map(str::to_owned);
                let _ = to.send(json!({"id": "theme", "command": "theme", "args": {"theme": theme}}));
                Some(Ok(json!({"theme": theme})))
            } else if changes.is_empty() {
                Some(Ok(json!({"theme": state.chosen})))
            } else {
                Some(Err(json!({"code": -32000, "message": format!("A computer's native window can't change {} yet.", changes.iter().map(|change| change.as_str()).collect::<Vec<_>>().join(", "))})))
            }
        }
        method if method.starts_with("ui/") => Some(Err(json!({"code": -32601, "message": format!("This client has no {method}")}))),
        _ => None,
    };
    match answered {
        Some(Ok(result)) => Some(json!({"jsonrpc": "2.0", "id": id, "result": result})),
        Some(Err(error)) => Some(json!({"jsonrpc": "2.0", "id": id, "error": error})),
        None => {
            // What the client answers, as from the app, for whoever this window is for.
            state.next += 1;
            let at = format!("m{}", state.next);
            let app = state.entrypoint.split('/').next().unwrap_or_default().to_owned();
            let mut params = params.clone();
            let mut meta = if state.trusted { params["_meta"].as_object().cloned().unwrap_or_default() } else { Default::default() };
            meta.insert("app".into(), json!(app));
            params["_meta"] = Value::Object(meta);
            asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
            let _ = to.send(json!({"id": at, "command": "request", "args": {"view": state.view, "method": method, "params": params}}));
            None
        }
    }
}

/// Hears the app's window: answers to what the app asked, Colour Mode chosen in another window, what changed, and
/// being opened again or told to end.
fn hear(mut heard: UnboundedReceiver<Heard>, handle: gpui::WindowHandle<strict::Screen>, state: Arc<Mutex<State>>, cx: &mut gpui::App) {
    cx.spawn(async move |cx| {
        while let Some(heard) = heard.recv().await {
            let quit = matches!(heard, Heard::Quit);
            let _ = handle.update(cx, |screen, window, cx| match heard {
                Heard::Answer(id, result) => {
                    let answer = match result {
                        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": error.as_str().map_or_else(|| error.to_string(), str::to_owned)}}),
                    };
                    screen.message(&answer, window, cx);
                }
                Heard::Event(event, payload) => match event.as_str() {
                    "theme" => {
                        let chosen = payload.as_str().map(str::to_owned);
                        screen.theme = theme(chosen.as_deref());
                        let dark = matches!(screen.theme.unwrap_or(window.appearance()), WindowAppearance::Dark | WindowAppearance::VibrantDark);
                        state.lock().expect("nothing panics holding it").chosen = chosen;
                        let theme = if dark { "dark" } else { "light" };
                        screen.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"theme": theme}}), window, cx);
                    }
                    "changed" if payload.as_str() == Some(state.lock().expect("nothing panics holding it").view.as_str()) => {
                        screen.message(&json!({"jsonrpc": "2.0", "method": "notifications/one/changed"}), window, cx);
                    }
                    "open" => window.activate_window(),
                    _ => {}
                },
                Heard::Inspect(request, answer) => inspect(request, answer, &state, window),
                Heard::Quit => {}
            });
            if quit {
                let _ = cx.update(|cx| cx.quit());
                return;
            }
        }
    })
    .detach();
}

/// Connects to the app's window, says hello as the web view's window does (`host::channel`), and talks with it on a
/// thread of its own. → where to send it a message.
fn channel(
    address: &str,
    entrypoint: &str,
    subject: Option<&str>,
    link: &str,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    heard: UnboundedSender<Heard>,
) -> anyhow::Result<UnboundedSender<Value>> {
    let (to, mut out) = unbounded_channel::<Value>();
    let _ = to.send(json!({"hello": {"entrypoint": entrypoint, "subject": subject, "link": link}}));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    let stream = runtime.block_on(crate::host::connect(address))?;
    let ended = heard.clone();
    std::thread::spawn(move || {
        runtime.block_on(async move {
            let (read, mut write) = tokio::io::split(stream);
            tokio::spawn(async move {
                while let Some(message) = out.recv().await {
                    if write.write_all(format!("{message}\n").as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                if message["quit"] == true {
                    break;
                }
                if let Some(event) = message["event"].as_str() {
                    let _ = heard.send(Heard::Event(event.to_owned(), message["payload"].clone()));
                    continue;
                }
                let Some(id) = message["id"].as_str() else { continue };
                let Some(waiting) = asked.lock().expect("nothing panics holding it").remove(id) else { continue };
                let result = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
                match waiting {
                    Asked::File(answer) => {
                        let file = result
                            .map(|file| BASE64.decode(file["body"].as_str().unwrap_or_default().as_bytes()).unwrap_or_default())
                            .map_err(|error| error.as_str().map_or_else(|| error.to_string(), str::to_owned));
                        let _ = answer.send(file);
                    }
                    Asked::Start(answer) => drop(answer.send(result.unwrap_or_default())),
                    Asked::Message(id) => drop(heard.send(Heard::Answer(id, result))),
                }
            }
        });
        let _ = ended.send(Heard::Quit);
    });
    Ok(to)
}

/// Where window `pid` of the client in `home` serves the inspector: beside the app's window's channel (`hosts`), named
/// after it.
fn socket(home: &std::path::Path, pid: u32) -> std::path::PathBuf {
    let address = crate::hosts::address(home);
    std::path::PathBuf::from(format!("{}-inspect-{pid}.sock", address.trim_end_matches(".sock")))
}

/// The inspector sockets of the client in `home`'s strict windows, by each window's process.
fn sockets(home: &std::path::Path) -> Vec<(u32, std::path::PathBuf)> {
    let sample = socket(home, 0);
    let (Some(folder), Some(name)) = (sample.parent(), sample.file_name().and_then(|name| name.to_str())) else { return Vec::new() };
    let prefix = name.trim_end_matches("0.sock").to_owned();
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let pid = path.file_name()?.to_str()?.strip_prefix(&prefix)?.strip_suffix(".sock")?.parse().ok()?;
            Some((pid, path))
        })
        .collect()
}

/// Serves the inspector's protocol on `socket`, only to this user, each request answered by the window (`inspect`)
/// on a thread of its own, one connection's in the order asked.
fn inspector(socket: &std::path::Path, heard: UnboundedSender<Heard>) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(socket);
    let listener = std::os::unix::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().build()?;
    std::thread::spawn(move || {
        runtime.block_on(async move {
            let Ok(listener) = tokio::net::UnixListener::from_std(listener) else { return };
            while let Ok((stream, _)) = listener.accept().await {
                let heard = heard.clone();
                tokio::spawn(async move {
                    let (read, mut write) = stream.into_split();
                    let mut lines = BufReader::new(read).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let answer = match strict::inspect::Request::parse(&line) {
                            Err(refused) => refused,
                            Ok(request) => {
                                let id = request.id.clone();
                                let (answer_to, answer) = tokio::sync::oneshot::channel();
                                if heard.send(Heard::Inspect(request, answer_to)).is_err() {
                                    return;
                                }
                                answer.await.unwrap_or_else(|_| strict::inspect::answer(&id, Err(anyhow::anyhow!("the window closed"))))
                            }
                        };
                        if write.write_all(format!("{answer}\n").as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        })
    });
    Ok(())
}

/// Answers an inspector's request once the window has drawn what changed last, as the protocol's rects are where the
/// last frame put each node: after the frame that's next, and the one after it, since a frame's callbacks run before
/// it's drawn.
fn inspect(request: strict::inspect::Request, answer: tokio::sync::oneshot::Sender<Value>, state: &Arc<Mutex<State>>, window: &mut gpui::Window) {
    let (entrypoint, subject) = {
        let state = state.lock().expect("nothing panics holding it");
        (state.entrypoint.clone(), state.subject.clone())
    };
    window.refresh();
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, cx| {
            let Some(Some(screen)) = window.root::<strict::Screen>() else { return };
            if request.op == "snap" {
                let _ = answer.send(strict::inspect::answer(&request.id, strict::snap(window)));
                return;
            }
            let op = request.op.clone();
            let reply: strict::Reply = Box::new(move |mut answered: Value| {
                if op == "window" && answered.get("result").is_some() {
                    answered["result"]["entrypoint"] = json!(entrypoint);
                    answered["result"]["subject"] = json!(subject);
                    answered["result"]["window"] = json!(std::process::id());
                }
                let _ = answer.send(answered);
            });
            screen.update(cx, |screen, cx| screen.inspect(request, reply, window, cx));
        });
    });
}

/// The Inspector app's tools of this device: its strict windows, each by its process (`window`), through their
/// sockets (`socket`), as the network asks them of the app's client (`client::DeviceTools`).
pub struct Inspector(pub std::path::PathBuf);

impl ::client::DeviceTools for Inspector {
    fn app(&self) -> &'static str {
        "inspector"
    }

    fn call<'a>(&'a self, tool: &'a str, arguments: Value) -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let result = match tool {
                "windows" => {
                    let mut windows = Vec::new();
                    for (pid, path) in sockets(&self.0) {
                        match ask(&path, json!({"v": strict::inspect::VERSION, "id": 1, "op": "window"})).await {
                            Ok(window) => windows.push(window),
                            // Its process ended without taking its socket away.
                            Err(_) if !std::path::Path::new(&format!("/proc/{pid}")).exists() => drop(std::fs::remove_file(&path)),
                            Err(_) => {}
                        }
                    }
                    let windows = json!({"device": ::client::name(), "windows": windows});
                    return Ok(json!({"content": [{"type": "text", "text": windows.to_string()}], "structuredContent": windows}));
                }
                _ => {
                    let mut request = match arguments {
                        Value::Object(inputs) => inputs,
                        _ => Default::default(),
                    };
                    let open = sockets(&self.0);
                    let path = match request.remove("window").and_then(|window| window.as_u64()) {
                        Some(pid) => open.into_iter().find(|(open, _)| u64::from(*open) == pid).map(|(_, path)| path),
                        None if open.len() == 1 => open.into_iter().next().map(|(_, path)| path),
                        None => None,
                    };
                    let Some(path) = path else {
                        return Ok(refused("Say which strict window, by the `window` that `windows` lists: none or more than one is open here, or that one isn't."));
                    };
                    request.remove("device");
                    request.insert("v".into(), json!(strict::inspect::VERSION));
                    request.insert("id".into(), json!(1));
                    request.insert("op".into(), json!(tool));
                    ask(&path, Value::Object(request)).await
                }
            };
            Ok(match result {
                Err(error) => refused(&format!("{error:#}")),
                Ok(result) if tool == "snap" => json!({
                    "content": [{"type": "image", "mimeType": "image/png", "data": result["png"]}],
                    "structuredContent": {"width": result["width"], "height": result["height"]},
                }),
                Ok(result) => json!({"content": [{"type": "text", "text": result.to_string()}], "structuredContent": result}),
            })
        })
    }
}

/// A tool's result that says why it didn't.
fn refused(why: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": why}]})
}

/// Asks a strict window's inspector one request, on a connection of its own. → its result.
async fn ask(socket: &std::path::Path, request: Value) -> anyhow::Result<Value> {
    let stream = tokio::net::UnixStream::connect(socket).await?;
    let (read, mut write) = stream.into_split();
    write.write_all(format!("{request}\n").as_bytes()).await?;
    let line = BufReader::new(read).lines().next_line().await?.ok_or_else(|| anyhow::anyhow!("the window closed"))?;
    let mut answer: Value = serde_json::from_str(&line)?;
    match answer.get("error") {
        Some(error) => anyhow::bail!("{}", error.as_str().unwrap_or_default()),
        None => Ok(answer["result"].take()),
    }
}
