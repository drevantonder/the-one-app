//! An app's window drawn natively, where its entrypoint's page is in strict mode (`host` in
//! core/ui/src/strict/protocol.ts) and of a version this program speaks (`strict::VERSION`): this process, the One
//! App's, draws it on its strict thread (`strict::platform`) beside every other strict app's window, so another open
//! app costs a few MB, and its JS runs in a QuickJS sandbox of its own. Any other app's window is a web view in a
//! process of its own, as before (`host`), and so is a strict one whose app sends a version this program doesn't
//! speak.
//!
//! It asks what a page would, over the same channel a web view's window does (`hosts`), here an in-memory one: its
//! files, and what the page asks of its client, which an MCP App asks its host (JSON-RPC). What a web view's window
//! page answers itself (`respond` in page/client.js) is answered here: `ui/initialize`, `ping` and `one/client`'s
//! Colour Mode; the rest is the client's, as from the app (`request`).
//!
//! The app's client offers the network the inspector's protocol (`strict::inspect`) for this device's strict windows,
//! as the Inspector app's tools (`Inspector`), so an agent anywhere in the network reads and drives a strict window as
//! it would one of its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Instant;

use data_encoding::BASE64;
use serde_json::{Value, json};
use strict::geometry::{Pixels, Size};
use strict::js::{Clock, Files, Sandbox};
use strict::platform::{Event, Strict, WindowId, WindowOptions};
use strict::{Appearance, Screen};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use url::Url;

/// Something asked of the app's window that it hasn't answered: a file the sandbox waits for, what the app's JS
/// asked, by its JSON-RPC id, or `start`.
enum Asked {
    File(mpsc::Sender<Result<Vec<u8>, String>>),
    Message(Value),
    Start(tokio::sync::oneshot::Sender<Value>),
}

/// Opens `entrypoint`'s window here if its page is strict and of a version this program speaks, at `link`, for
/// `subject`. → whether it did; if not, it opens in a web view.
pub(crate) async fn open(entrypoint: &str, subject: Option<&str>, link: &str) -> bool {
    let started = Instant::now();
    match window(started, entrypoint, subject, link).await {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("{entrypoint} opens in a web view: {error:#}");
            false
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

/// Whether `entrypoint`'s page is strict and of a version this program speaks, and if it is, the module it loads
/// and whether its app is trusted.
async fn probe(entrypoint: &str, subject: Option<&str>) -> anyhow::Result<Option<(Url, bool)>> {
    let listed = crate::hosts::command("strict", "apps", json!({"subject": subject})).await.map_err(|error| anyhow::anyhow!(error))?;
    let (app, path) = entrypoint.split_once('/').unwrap_or((entrypoint, ""));
    let Some(resource) = listed["resources"].as_array().into_iter().flatten().find(|resource| {
        let uri = resource["uri"].as_str().unwrap_or_default();
        uri.strip_prefix(&format!("ui://{app}/")).and_then(|rest| rest.split_once('/')).is_some_and(|(_, page)| page == path)
    }) else {
        return Ok(None);
    };
    if !resource["_meta"]["host"].as_u64().is_some_and(|host| host <= u64::from(strict::VERSION)) {
        return Ok(None);
    }
    let trusted = resource["_meta"]["trusted"] == true;
    let page = Url::parse(&origin(entrypoint, subject))?.join(entrypoint)?;
    let html = crate::hosts::file(page.as_str()).await.map_err(|error| anyhow::anyhow!(error))?;
    let html = BASE64.decode(html["body"].as_str().unwrap_or_default().as_bytes())?;
    let module = module_script(&String::from_utf8_lossy(&html)).ok_or_else(|| anyhow::anyhow!("its page loads no module"))?;
    Ok(Some((page.join(&module)?, trusted)))
}

async fn window(started: Instant, entrypoint: &str, subject: Option<&str>, link: &str) -> anyhow::Result<bool> {
    let Some((module, trusted)) = probe(entrypoint, subject).await? else { return Ok(false) };
    let strict = Strict::get()?;
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let window: Arc<OnceLock<WindowId>> = Arc::default();
    let to = channel(entrypoint, subject, link, asked.clone(), strict.clone(), window.clone())?;
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());

    // Where the device is with its network, as a page asks as it opens: the Colour Mode chosen on it, if one was.
    let (start_to, start) = tokio::sync::oneshot::channel();
    asked.lock().expect("nothing panics holding it").insert("start".into(), Asked::Start(start_to));
    let _ = to.send(json!({"id": "start", "command": "start", "args": {"entrypoint": entrypoint}}));
    let start = start.await?;
    let chosen = start["theme"].as_str().map(str::to_owned);
    let name = start["name"].as_str().map_or_else(|| entrypoint.split('/').next().unwrap_or_default().to_owned(), str::to_owned);

    let (files_to, files_asked) = (to.clone(), asked.clone());
    let state = Arc::new(Mutex::new(State { chosen: chosen.clone(), trusted, entrypoint: entrypoint.to_owned(), view, next: 0 }));
    let (entrypoint, subject, link) = (entrypoint.to_owned(), subject.map(str::to_owned), link.to_owned());
    let cache = ::client::home()?.join("strict").join(::client::BUILD);
    strict.run(move |windows| {
        // The page's files, which the app's window has, each asked for as the sandbox imports it, on the strict thread.
        let next = std::sync::atomic::AtomicU64::new(0);
        let mut files: Files = Box::new(move |url: &Url| {
            let id = format!("f{}", next.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            let (answer_to, answer) = mpsc::channel();
            files_asked.lock().expect("nothing panics holding it").insert(id.clone(), Asked::File(answer_to));
            let _ = files_to.send(json!({"id": id, "file": url.as_str()}));
            let file = answer.recv()?.map_err(|error| anyhow::anyhow!("{error}"))?;
            Ok(String::from_utf8(file)?)
        });
        let options = WindowOptions {
            title: name.clone(),
            app_id: ::client::NAME.to_owned(),
            size: Size { width: Pixels(1024.0), height: Pixels(720.0) },
            tag: json!({"entrypoint": entrypoint, "subject": subject}),
        };
        // The design tokens, which come with the UI library, as its pages' do.
        let tokens = module.join("/ui/client/ui.css").map_err(anyhow::Error::from).and_then(|css| files(&css));
        let sandbox = tokens.and_then(|css| {
            strict::tokens::load(&css);
            Sandbox::start(&module, files, Some(cache), Clock::real())
        });
        let app = entrypoint.split('/').next().unwrap_or_default().to_owned();
        let on_event = {
            let (to, app, entrypoint, subject, link) = (to.clone(), app.clone(), entrypoint.clone(), subject.clone(), link.clone());
            move |event: Event| match event {
                // How long it took to open, from being asked to, as a page tells the app's window it has drawn.
                Event::Drawn => {
                    let ms = started.elapsed().as_millis() as u64;
                    let _ = to.send(json!({"id": "opened", "command": "opened", "args": {"view": app, "ms": ms}}));
                    let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": true}}));
                }
                Event::Closed => {}
                Event::Mismatched => {
                    let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
                    if let Err(error) = crate::hosts::open_web_view(entrypoint.clone(), subject, link) {
                        eprintln!("{entrypoint}: {error}");
                    }
                }
            }
        };
        let opened = match sandbox {
            Ok(sandbox) => {
                let respond_state = state.clone();
                let respond_to = to.clone();
                let respond_asked = asked.clone();
                let asked_by_app: strict::Asked =
                    Box::new(move |message, appearance| respond(message, appearance, &respond_state, &respond_to, &respond_asked));
                let mut screen = Screen::new(sandbox, Some(asked_by_app));
                screen.theme = theme(chosen.as_deref());
                windows.open(options, screen, on_event)
            }
            // Its page's own module didn't start: the window says so, as a web view's page does.
            Err(error) => {
                eprintln!("{entrypoint} didn't start: {error:#}");
                windows.open_failed(options, &format!("{name} didn't open"), on_event)
            }
        };
        match opened {
            Ok(id) => drop(window.set(id)),
            Err(error) => eprintln!("{entrypoint}'s window didn't open: {error:#}"),
        }
    });
    Ok(true)
}

/// What the window keeps of where it is.
struct State {
    /// The Colour Mode chosen on this device, or none for as the device is.
    chosen: Option<String>,
    trusted: bool,
    entrypoint: String,
    view: String,
    next: u64,
}

fn theme(chosen: Option<&str>) -> Option<Appearance> {
    match chosen {
        Some("dark") => Some(Appearance::Dark),
        Some("light") => Some(Appearance::Light),
        _ => None,
    }
}

/// Answers what the app's JS posted its client: what the window page answers itself, here, at once, and the rest by
/// the app's window, as from the app, which comes back as it's heard (`channel`).
fn respond(
    message: Value,
    appearance: Appearance,
    state: &Arc<Mutex<State>>,
    to: &UnboundedSender<Value>,
    asked: &Arc<Mutex<HashMap<String, Asked>>>,
) -> Option<Value> {
    let method = message["method"].as_str()?;
    // A notification, which needs no answer: none is heard here yet.
    let id = message.get("id").cloned()?;
    let params = &message["params"];
    let mut state = state.lock().expect("nothing panics holding it");
    let shown = state.chosen.clone().unwrap_or_else(|| if appearance == Appearance::Dark { "dark" } else { "light" }.into());
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
                // Colour Mode is the device's: the app's window tells every window, this one too (`channel`).
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

/// Talks with this process's own channel for the app's windows (`hosts::talk`), as a web view's window does over its
/// socket, saying hello as it does: what it hears is done to the window on the strict thread. → where to send it a
/// message.
fn channel(
    entrypoint: &str,
    subject: Option<&str>,
    link: &str,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    strict: Strict,
    window: Arc<OnceLock<WindowId>>,
) -> anyhow::Result<UnboundedSender<Value>> {
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    crate::spawn(crate::hosts::talk(theirs));
    let (to, mut out) = unbounded_channel::<Value>();
    let _ = to.send(json!({"hello": {"entrypoint": entrypoint, "subject": subject, "link": link}}));
    let (read, mut write) = tokio::io::split(ours);
    crate::spawn(async move {
        while let Some(message) = out.recv().await {
            if write.write_all(format!("{message}\n").as_bytes()).await.is_err() {
                return;
            }
        }
        // The window's gone, and with it all that sends: the channel ends, so its app's window forgets it.
        let _ = write.shutdown().await;
    });
    let to_window = to.clone();
    crate::spawn(async move {
        let mut lines = BufReader::new(read).lines();
        // The window this is, once the strict thread has opened it; a job runs after the one that opened it.
        let on = |job: Box<dyn FnOnce(&mut Screen) + Send>| {
            let window = window.clone();
            strict.run(move |windows| {
                if let Some(id) = window.get() {
                    windows.with(*id, job);
                }
            })
        };
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            if message["quit"] == true {
                break;
            }
            if let Some(event) = message["event"].as_str() {
                let payload = message["payload"].clone();
                match event {
                    "theme" => on(Box::new(move |screen| {
                        screen.theme = theme(payload.as_str());
                        let theme = if screen.dark() { "dark" } else { "light" };
                        screen.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"theme": theme}}));
                    })),
                    "changed" if payload.as_str() == Some(view.as_str()) => {
                        on(Box::new(move |screen| screen.message(&json!({"jsonrpc": "2.0", "method": "notifications/one/changed"}))))
                    }
                    "open" => {
                        let window = window.clone();
                        strict.run(move |windows| {
                            if let Some(id) = window.get() {
                                windows.activate(*id);
                            }
                        });
                    }
                    _ => {}
                }
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
                Asked::Message(id) => {
                    let answer = match result {
                        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": error.as_str().map_or_else(|| error.to_string(), str::to_owned)}}),
                    };
                    on(Box::new(move |screen| screen.message(&answer)));
                }
            }
        }
        // Told to end, or the One App is: the window closes.
        let window = window.clone();
        strict.run(move |windows| {
            if let Some(id) = window.get() {
                windows.close(*id);
            }
        });
    });
    Ok(to_window)
}

/// The Inspector app's tools of this device: its strict windows, each by its id (`window`), as the network asks them
/// of the app's client (`client::DeviceTools`), answered on the strict thread.
pub struct Inspector;

impl ::client::DeviceTools for Inspector {
    fn app(&self) -> &'static str {
        "inspector"
    }

    fn call<'a>(&'a self, tool: &'a str, arguments: Value) -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let strict = match Strict::get() {
                Ok(strict) => strict,
                Err(error) => return Ok(refused(&format!("no strict window can open here: {error:#}"))),
            };
            if tool == "windows" {
                let (listed_to, listed) = tokio::sync::oneshot::channel();
                strict.run(move |windows| drop(listed_to.send(windows.list())));
                let windows = json!({"device": ::client::name(), "windows": listed.await.unwrap_or_default()});
                return Ok(json!({"content": [{"type": "text", "text": windows.to_string()}], "structuredContent": windows}));
            }
            let mut request = match arguments {
                Value::Object(inputs) => inputs,
                _ => Default::default(),
            };
            let window = request.remove("window").and_then(|window| window.as_u64());
            request.remove("device");
            request.insert("v".into(), json!(strict::inspect::VERSION));
            request.insert("id".into(), json!(1));
            request.insert("op".into(), json!(tool));
            let request = match strict::inspect::Request::parse(&Value::Object(request).to_string()) {
                Ok(request) => request,
                Err(refused_with) => return Ok(refused(refused_with["error"].as_str().unwrap_or("that request isn't one"))),
            };
            let (answer_to, answer) = tokio::sync::oneshot::channel::<Value>();
            strict.run(move |windows| {
                let open: Vec<WindowId> = windows.list().iter().filter_map(|listed| listed["window"].as_u64()).collect();
                let reply: strict::Reply = Box::new(move |answered| drop(answer_to.send(answered)));
                match window.or_else(|| (open.len() == 1).then(|| open[0])) {
                    Some(id) => windows.inspect(id, request, reply),
                    None => reply(strict::inspect::answer(&request.id, Err(anyhow::anyhow!("Say which strict window, by the `window` that `windows` lists: none or more than one is open here.")))),
                }
            });
            let mut answer = answer.await.unwrap_or_else(|_| json!({"error": "the window closed"}));
            Ok(match answer.get("error") {
                Some(error) => refused(error.as_str().unwrap_or_default()),
                None if tool == "snap" => json!({
                    "content": [{"type": "image", "mimeType": "image/png", "data": answer["result"]["png"]}],
                    "structuredContent": {"width": answer["result"]["width"], "height": answer["result"]["height"]},
                }),
                None => {
                    let result = answer["result"].take();
                    json!({"content": [{"type": "text", "text": result.to_string()}], "structuredContent": result})
                }
            })
        })
    }
}

/// A tool's result that says why it didn't.
fn refused(why: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": why}]})
}
