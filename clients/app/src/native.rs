//! An app's window drawn natively, where its entrypoint's page needs a runtime version this
//! program has (`runtimeVersion`, core/ui/src/drawing-primitives/renderer.ts's `RUNTIME_VERSION`, `renderer::RUNTIME_VERSION`):
//! this process, the One App's, draws it on its native thread (`renderer::platform`) beside every other app's window, so another open
//! app costs a few MB, and its JS runs in a QuickJS sandbox of its own, on a thread of its own. Any other app's window is a web view in a
//! process of its own, as before (`host`), and so is one that needs a later runtime version.
//!
//! It asks what a page would, over the same channel a web view's window does (`hosts`), here an in-memory one: its
//! files, and what the page asks of its client, which an MCP App asks its host (JSON-RPC). What a web view's window
//! page answers itself (`respond` in page/client.js) is answered here: `ui/initialize`, `ping` and `one/client`'s
//! Colour Mode; the rest is the client's, as from the app (`request`).
//!
//! The app's client offers the network the inspector's protocol (`renderer::inspect`) for this device's native windows,
//! as the Inspector app's tools (`Inspector`), so an agent anywhere in the network reads and drives a native window as
//! it would one of its own.
//!
//! On a phone the same window is a view over the page, on its UI thread, and any other app is a frame in the page
//! (`hosts::open_window`); what a computer's launcher does for it, a phone does as its own (`computer`, below).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Instant;

use data_encoding::BASE64;
use serde_json::{Value, json};
use renderer::geometry::{Bounds, Pixels, Size};
use renderer::js::{Clock, Files, Thread};
use renderer::platform::{Event, Native, WindowId, WindowOptions};
use renderer::{Appearance, Screen};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use url::Url;

#[cfg(desktop)]
use crate::{computer, page::NAME};
#[cfg(mobile)]
use computer::NAME;

/// Something asked of the app's window that it hasn't answered: a file the sandbox waits for, what the app's JS
/// asked, by its JSON-RPC id, or `start`.
enum Asked {
    File(mpsc::Sender<Result<Vec<u8>, String>>),
    /// A picture its page draws by its address, as it names it.
    Picture(String),
    Message(Value),
    Start(tokio::sync::oneshot::Sender<Value>),
}

/// Opens `entrypoint`'s window here if its page is of a version this program speaks, at `link`, for
/// `subject`. → whether it did; if not, it opens in a web view, as it needs a newer host. One that should have and
/// didn't is an error, which Telemetry hears too, for whoever opened it to show.
pub(crate) async fn open(entrypoint: &str, subject: Option<&str>, link: &str, icon: Option<Bounds<f32>>) -> Result<bool, String> {
    let started = Instant::now();
    window(started, entrypoint, subject, link, Kind::App(icon)).await.map_err(|error| {
        let why = format!("{error:#}");
        reported(entrypoint, started, &why);
        why
    })
}

/// Tells Telemetry an app's window didn't open, and why, as it's said in the window, where it could open one.
pub(crate) fn reported(entrypoint: &str, started: Instant, why: &str) {
    eprintln!("{entrypoint} didn't open: {why}");
    if let Ok(views) = crate::views() {
        let app = entrypoint.split('/').next().unwrap_or_default();
        let took = started.elapsed().as_millis() as u64;
        views.client.report(json!({"name": "one.client.native.open", "took": took, "error": why, "attributes": {"one.view": app}}));
    }
}

/// Opens a window for `entrypoint`, at `link`, for `subject`, that says only why its page isn't there, as Telemetry
/// hears (`open`): one this program can't draw, or that didn't open. It's the problem box DESIGN.md has, with Try
/// Again, on the client's own page (`UNOPENED`); where that can't be drawn either, as with no UI library to draw it
/// with, it's said in words alone.
pub(crate) fn unopened(entrypoint: String, subject: Option<String>, link: String, why: String) {
    crate::spawn(async move {
        let listed = crate::hosts::command("native", "apps", json!({"subject": subject})).await.unwrap_or_default();
        let name = listed_page(&listed, &entrypoint).and_then(|page| page["name"].as_str()).filter(|name| !name.contains('/')).map(str::to_owned);
        let name = name.unwrap_or_else(|| entrypoint.split('/').next().unwrap_or_default().to_owned());
        let said = format!("{name} didn't open: {why}");
        // It's the app's window, as its client knows it, so the app opened again brings it forward.
        let unopened = Unopened { entrypoint: entrypoint.clone(), subject: subject.clone(), link: link.clone(), name: name.clone(), why };
        match window(Instant::now(), &entrypoint, subject.as_deref(), &link, Kind::Unopened(unopened)).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => eprintln!("{entrypoint}'s problem didn't open: {error:#}"),
        }
        let Ok(native) = Native::get() else { return };
        let tag = json!({"entrypoint": entrypoint});
        native.run(move |windows| {
            let options = WindowOptions { title: name.clone(), app_id: ::client::NAME.to_owned(), size: Size { width: Pixels(1024.0), height: Pixels(720.0) }, tag, panel: None, places: None, root: false, icon: None, owner: None };
            if let Err(error) = windows.open(options, Err(said), |_| {}) {
                eprintln!("{name}'s window didn't open: {error:#}");
            }
        });
    });
}

/// Closes the window `shown`, once it's open.
fn close(shown: &OnceLock<WindowId>) {
    let Some(id) = shown.get().copied() else { return };
    if let Ok(native) = Native::get() {
        native.run(move |windows| windows.close(id));
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

/// What a native page is drawn with: the module it loads, whether its app is trusted, and the tables its styles
/// come from: the UI library's tokens and styles and its app's styles (`Tokens` and `Styles` in core/ui/src/drawing-primitives/renderer.ts).
struct Page {
    /// Its page's address, which what it names by a relative address is from, and the module it loads.
    page: Url,
    module: Url,
    trusted: bool,
    /// The UI library's tables, and its app's styles where it has them.
    tables: Vec<String>,
    /// Its name and icon, as a `data:` address, as its app lists them, for the window's title and breadcrumb.
    name: Option<String>,
    icon: Option<String>,
}

/// `entrypoint`'s page as the apps `listed` list it, if they do.
fn listed_page<'a>(listed: &'a Value, entrypoint: &str) -> Option<&'a Value> {
    let (app, path) = entrypoint.split_once('/').unwrap_or((entrypoint, ""));
    listed["resources"].as_array().into_iter().flatten().find(|resource| {
        let uri = resource["uri"].as_str().unwrap_or_default();
        uri.strip_prefix(&format!("ui://{app}/")).and_then(|rest| rest.split_once('/')).is_some_and(|(_, page)| page == path)
    })
}

/// Whether `entrypoint`'s page is needs a runtime version this program has, its tables too, and if it is, what
/// it's drawn with.
async fn probe(entrypoint: &str, subject: Option<&str>, framed: bool) -> anyhow::Result<Option<Page>> {
    let listed = crate::hosts::command("native", "apps", json!({"subject": subject})).await.map_err(|error| anyhow::anyhow!(error))?;
    let Some(resource) = listed_page(&listed, entrypoint) else { return Ok(None) };
    // A page in a frame that names no runtime version (a component's, which no entrypoint names) is tried: one that
    // isn't drawn natively doesn't start, and the page the frame's in opens in a web view instead.
    let needed = resource["_meta"]["runtimeVersion"].as_u64();
    if !needed.map_or(framed, |needed| needed <= u64::from(renderer::RUNTIME_VERSION)) {
        return Ok(None);
    }
    let trusted = resource["_meta"]["trusted"] == true;
    // Its icon, as its app lists it, as a `data:` address, for the breadcrumb.
    let icon = match resource["icons"][0]["src"].as_str() {
        Some(src) if src.starts_with("data:") => Some(src.to_owned()),
        Some(src) => crate::hosts::file(src).await.ok().and_then(|file| Some(format!("data:{};base64,{}", file["kind"].as_str()?, file["body"].as_str()?))),
        None => None,
    };
    let page = Url::parse(&origin(entrypoint, subject))?.join(entrypoint)?;
    let html = crate::hosts::file(page.as_str()).await.map_err(|error| anyhow::anyhow!(error))?;
    let html = BASE64.decode(html["body"].as_str().unwrap_or_default().as_bytes())?;
    let module = module_script(&String::from_utf8_lossy(&html)).ok_or_else(|| anyhow::anyhow!("its page loads no module"))?;
    // Its tables: the UI library's, and its app's styles, which its build writes beside every page (`core/ui/app.js`).
    let Some(tables) = tables(&[page.join("/ui/client/tokens.json")?, page.join("/ui/client/styles.json")?, page.join("styles.json")?]).await? else { return Ok(None) };
    let name = resource["name"].as_str().filter(|name| !name.is_empty()).map(str::to_owned);
    Ok(Some(Page { module: page.join(&module)?, page, trusted, tables, name, icon }))
}

/// The tables at `addresses`, or none where one needs a runtime version this program hasn't, so its page is a web
/// view's.
async fn tables(addresses: &[Url]) -> anyhow::Result<Option<Vec<String>>> {
    let mut tables = Vec::new();
    for table in addresses {
        let file = crate::hosts::file(table.as_str()).await.map_err(|error| anyhow::anyhow!("{table} didn't load: {error}"))?;
        let file = BASE64.decode(file["body"].as_str().unwrap_or_default().as_bytes())?;
        let text = String::from_utf8(file)?;
        let v = serde_json::from_str::<Value>(&text).map(|table| table["v"].as_u64().unwrap_or(0)).unwrap_or(0);
        if v > u64::from(renderer::RUNTIME_VERSION) {
            return Ok(None);
        }
        tables.push(text);
    }
    Ok(Some(tables))
}

/// A page of the client's own (`RECOVERY`, `UNOPENED`), called `name`, drawn with the UI library as an app's page is,
/// which it loads as one does, and no styles of its own.
async fn client_page(address: &str, name: &str) -> anyhow::Result<Page> {
    let module = Url::parse(address)?;
    let Some(tables) = tables(&[module.join("/ui/client/tokens.json")?, module.join("/ui/client/styles.json")?]).await? else {
        anyhow::bail!("the UI library needs a newer runtime than this program's")
    };
    Ok(Page { page: module.clone(), module, trusted: false, tables, name: Some(name.to_owned()), icon: None })
}

/// Opens the launcher's page natively: on a computer in a panel put away until it's summoned (`computer`), on a phone
/// as its root window, under every app's. → its window, once it's open. A computer's whose page this program can't
/// draw, or that didn't open, is a panel that says why, and Telemetry hears it; where its native windows can't start,
/// there's none, and the web launcher says so (`computer::unnative`). A phone's is its page's.
pub(crate) async fn launcher(entrypoint: &str) -> Option<WindowId> {
    let started = Instant::now();
    let (opened_to, opened) = tokio::sync::oneshot::channel();
    let why = match window(started, entrypoint, None, entrypoint, Kind::Launcher(Some(opened_to))).await {
        Ok(true) => return opened.await.ok().flatten(),
        Ok(false) => "this One App doesn't draw its page: it needs a newer One App".to_owned(),
        Err(error) => format!("{error:#}"),
    };
    reported(entrypoint, started, &why);
    unopened_launcher(why).await
}

/// A phone's launcher that isn't drawn natively is its page's.
#[cfg(mobile)]
async fn unopened_launcher(_: String) -> Option<WindowId> {
    None
}

/// A panel that says why the Home Screen didn't open, or where this computer's native windows can't start, none.
#[cfg(desktop)]
async fn unopened_launcher(why: String) -> Option<WindowId> {
    let said = format!("The Home Screen didn't open: {why}");
    let (shown_to, shown) = tokio::sync::oneshot::channel();
    if let Ok(native) = Native::get() {
        native.run(move |windows| {
            let options = WindowOptions {
                title: NAME.to_owned(),
                app_id: ::client::NAME.to_owned(),
                size: Size { width: Pixels(computer::SIZE.width as f32), height: Pixels(computer::SIZE.height as f32) },
                tag: json!({}),
                panel: Some(computer::CORNER as f32),
                places: crate::app().shell.home().ok().map(|home| home.join("launcher-places.json")),
                root: false,
                icon: None,
                owner: None,
            };
            let _ = shown_to.send(windows.open(options, Err(said), |_| {}).ok());
        });
    }
    let id = shown.await.ok().flatten();
    if id.is_none() {
        computer::unnative(why);
    }
    id
}

/// What a native window is: an app's own, or the launcher, a panel the system summons, which tells whoever opens it
/// its window once it's open, or that it didn't.
enum Kind {
    /// An app's, opened from where its icon is on the screen, if it was.
    App(Option<Bounds<f32>>),
    Launcher(Option<tokio::sync::oneshot::Sender<Option<WindowId>>>),
    /// The client's own, over an app's, where a recovery key is shown or typed (`recovery`).
    Sheet(Sheet),
    /// The client's own, in place of an app's that didn't open (`unopened`).
    Unopened(Unopened),
}

/// An app's window that didn't open: its entrypoint, who and where it was opened for, its app's name, and why.
#[derive(Clone)]
pub(crate) struct Unopened {
    entrypoint: String,
    subject: Option<String>,
    link: String,
    name: String,
    why: String,
}

/// What the client's recovery key window is for: showing the key of this device's person, or of `subject`, a group or
/// the network, or with `typed`, letting this device in with the one they type; and who hears whether it was done.
#[derive(Clone)]
struct Sheet {
    typed: bool,
    subject: Option<String>,
    /// The window of the app that asked, which it's a sheet of (`WindowOptions::owner`).
    owner: Option<WindowId>,
    done: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
}

/// The client's own page, which only the recovery key window shows: its JS, the client's and no app's, at an address
/// of its own (`hosts::file`).
pub(crate) const RECOVERY: &str = "oneapp://client.localhost/client/recovery.js";
/// The client's own page in an app's window that didn't open, its JS too (`hosts::file`).
pub(crate) const UNOPENED: &str = "oneapp://client.localhost/client/unopened.js";

/// Shows its person the recovery key, theirs or `subject`'s, or with `typed` asks them for it, in a window of the
/// client's own over the app that asked, which is told only whether they saw it or the device is in now: no app's
/// page is handed a recovery key, as an Apple device shows one in its own system's UI and never in an app.
/// It's boxed, as a window it opens asks the client what a window's page asks it (`hosts::command`), which opens this.
pub(crate) fn recovery(typed: bool, subject: Option<String>, owner: Option<WindowId>) -> std::pin::Pin<Box<dyn Future<Output = Result<bool, String>> + Send>> {
    Box::pin(async move {
        let (done_to, done) = tokio::sync::oneshot::channel();
        let sheet = Sheet { typed, subject, owner, done: Arc::new(Mutex::new(Some(done_to))) };
        let started = Instant::now();
        window(started, "client/recovery.js", None, "", Kind::Sheet(sheet)).await.map_err(|error| format!("{error:#}"))?;
        Ok(done.await.unwrap_or(false))
    })
}

async fn window(started: Instant, entrypoint: &str, subject: Option<&str>, link: &str, mut kind: Kind) -> anyhow::Result<bool> {
    let page = match &kind {
        Kind::Sheet(_) => Some(client_page(RECOVERY, "Recovery Key").await?),
        Kind::Unopened(unopened) => Some(client_page(UNOPENED, &unopened.name).await?),
        _ => probe(entrypoint, subject, false).await?,
    };
    let Some(Page { page, module, trusted, tables, name: listed, icon }) = page else { return Ok(false) };
    let launcher = matches!(kind, Kind::Launcher(_));
    let sheet = match &kind {
        Kind::Sheet(sheet) => Some(sheet.clone()),
        _ => None,
    };
    let unopened = match &kind {
        Kind::Unopened(unopened) => Some(unopened.clone()),
        _ => None,
    };
    // An app's own window, which says why in a window of the client's own if its page doesn't start (`unopened`).
    let app_window = matches!(kind, Kind::App(_));
    let from = match kind {
        Kind::App(from) => from,
        Kind::Launcher(_) | Kind::Sheet(_) | Kind::Unopened(_) => None,
    };
    let tokens_page = load(&tables)?;
    let native = Native::get()?;
    let window: Arc<OnceLock<WindowId>> = Arc::default();
    let begun = start(entrypoint, subject, link, launcher, Place::Window(window.clone()), &module, page)?;
    let (to, asked) = (begun.to.clone(), begun.asked.clone());
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());

    // Where the device is with its network, as a page asks as it opens: the Colour Mode chosen on it, if one was.
    let (start_to, start) = tokio::sync::oneshot::channel();
    asked.lock().expect("nothing panics holding it").insert("start".into(), Asked::Start(start_to));
    let _ = to.send(json!({"id": "start", "command": "start", "args": {"entrypoint": entrypoint}}));
    let start = start.await?;
    let chosen = start["theme"].as_str().map(str::to_owned);
    let name = listed.or_else(|| start["name"].as_str().map(str::to_owned)).unwrap_or_else(|| entrypoint.split('/').next().unwrap_or_default().to_owned());

    let fragment = link.split_once('#').map(|(_, fragment)| fragment.to_owned());
    let state = State { chosen: chosen.clone(), trusted, entrypoint: entrypoint.to_owned(), subject: subject.map(str::to_owned), view, next: 0, launcher, framed: false, fragment, sheet: sheet.clone(), unopened, window: window.clone() };
    let (entrypoint, subject) = (entrypoint.to_owned(), subject.map(str::to_owned));
    let link = link.to_owned();
    // Whether it opened, as the windows' thread says: a job that's dropped, as a phone's is where its windows
    // couldn't start, says why there.
    let (opened_to, opened) = tokio::sync::oneshot::channel::<Result<(), String>>();
    native.run(move |windows| {
        let (size, panel) = match (launcher && cfg!(desktop), &sheet) {
            (true, _) => ((computer::SIZE.width as f32, computer::SIZE.height as f32), Some(computer::CORNER)),
            (false, Some(_)) => ((440.0, 360.0), None),
            (false, None) => ((1024.0, 720.0), None),
        };
        let options = WindowOptions {
            title: if launcher { NAME.to_owned() } else { name.clone() },
            app_id: ::client::NAME.to_owned(),
            size: Size { width: Pixels(size.0), height: Pixels(size.1) },
            tag: json!({"entrypoint": entrypoint, "subject": subject}),
            panel: panel.map(|corner| corner as f32),
            // The launcher's kept where it's dragged to, on each screen.
            places: (launcher && cfg!(desktop)).then(|| crate::app().shell.home().ok()).flatten().map(|home| home.join("launcher-places.json")),
            // A phone's launcher is its Home Screen, under every app's window.
            root: launcher && cfg!(mobile),
            icon: from,
            // The recovery key window is a sheet of the app's that asked, as the system attaches one.
            owner: sheet.as_ref().and_then(|sheet| sheet.owner),
        };
        let app = entrypoint.split('/').next().unwrap_or_default().to_owned();
        let on_event = {
            let (to, app, entrypoint, sheet) = (to.clone(), app.clone(), entrypoint.clone(), sheet.clone());
            let (subject, link, shown) = (subject.clone(), link.clone(), window.clone());
            move |event: Event| match event {
                // How long it took to open, from being asked to, as a page tells the app's window it has drawn.
                Event::Drawn => {
                    let ms = started.elapsed().as_millis() as u64;
                    let _ = to.send(json!({"id": "opened", "command": "opened", "args": {"view": app, "ms": ms}}));
                    let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": true}}));
                    // The device hears its person's notifications once their home is on screen, as the web
                    // launcher's page has it (`listen`).
                    if launcher {
                        let _ = to.send(json!({"id": "listen", "command": "listen", "args": {}}));
                    }
                }
                // How long the launcher took to show as it was summoned, to its first frame.
                Event::Shown(took) => {
                    eprintln!("the launcher showed {:.1} ms after it was summoned", took.as_secs_f64() * 1000.0);
                    let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": true}}));
                }
                Event::PutAway => {
                    let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": false}}));
                }
                // Its person is using something else, so a new build waiting for what they typed to be sent looks
                // again (`updater::ready`), as a web view's window does as its frames say they're done typing.
                Event::Unfocused => crate::spawn(crate::updater::ready()),
                Event::Quit => {
                    #[cfg(desktop)]
                    computer::quit();
                }
                // Closed before it was done, the recovery key wasn't seen or used.
                Event::Closed => {
                    if let Some(told) = sheet.as_ref().and_then(|sheet| sheet.done.lock().expect("nothing panics holding it").take()) {
                        let _ = told.send(false);
                    }
                }
                // A window whose page didn't start says so, and Telemetry hears why: an app's in the client's own
                // window with its problem box in its place (`unopened`), and a phone's launcher is then its page's.
                Event::Failed(why) => {
                    reported(&entrypoint, started, &why);
                    #[cfg(mobile)]
                    if launcher {
                        computer::native(None);
                    }
                    if app_window {
                        close(&shown);
                        crate::hosts::forget(&entrypoint, subject.as_deref());
                        self::unopened(entrypoint.clone(), subject.clone(), link.clone(), why);
                    }
                }
                // A computer's window says why it didn't open, as one that didn't start does, and Telemetry hears; a
                // phone opens it in its page.
                Event::Mismatched(why) => {
                    reported(&entrypoint, started, &why);
                    #[cfg(desktop)]
                    if app_window {
                        close(&shown);
                        crate::hosts::forget(&entrypoint, subject.as_deref());
                        self::unopened(entrypoint.clone(), subject.clone(), link.clone(), why);
                    }
                    #[cfg(mobile)]
                    if launcher {
                        computer::native(None);
                    } else {
                        close(&shown);
                        let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
                        if let Err(error) = crate::hosts::open_web_view(entrypoint.clone(), subject, link) {
                            eprintln!("{entrypoint}: {error}");
                        }
                    }
                }
            }
        };
        // Its title bar, which its page keeps clear, as its host context's `safeAreaInsets` tells it. The launcher has
        // none.
        let top = if launcher { 0.0 } else { BAR.0 as f32 };
        let insets = renderer::geometry::Edges { top, right: 0.0, bottom: 0.0, left: 0.0 };
        let opened = match begun.screen(state, Size { width: options.size.width.0, height: options.size.height.0 }, insets) {
            Ok(mut screen) => {
                screen.theme = theme(chosen.as_deref());
                // A phone's is under a finger from the start, so it's drawn with its tokens' touch values from its
                // first frame (pointer: coarse), as the page is told (`deviceCapabilities`).
                screen.touch = cfg!(mobile);
                screen.page = tokens_page;
                // What its frames show, each another app's page in a sandbox of its own (`frame`): only a trusted
                // app's page shows one, as on the DOM (`one/frame` in page/client.js).
                let frames = window.clone();
                screen.make_frame = trusted.then(|| -> renderer::MakeFrame { Box::new(move |node, src, subject| {
                    let (frames, src, subject) = (frames.clone(), src.to_owned(), subject.map(str::to_owned));
                    crate::spawn(async move {
                        if let Err(error) = frame(frames.clone(), node, &src, subject.as_deref()).await {
                            eprintln!("{src} isn't drawn in its frame: {error:#}");
                            // The page it's in can't be drawn here without it: it opens in a web view instead.
                            if let Ok(native) = Native::get() {
                                native.run(move |windows| {
                                    if let Some(id) = frames.get() {
                                        windows.with(*id, |screen| screen.mismatched = Some(format!("its frame's page, {src}, isn't drawn natively: {error:#}")));
                                    }
                                })
                            }
                        }
                    });
                }) });
                if launcher && cfg!(desktop) {
                    // Its glass is the system's, under its page, where there's one; Linux has none, so it's a
                    // popover's colour (DESIGN.md, Launcher).
                    screen.panel = Some(renderer::Panel { corner: computer::CORNER as f32, fill: cfg!(target_os = "linux") });
                } else if cfg!(desktop) && sheet.is_none() {
                    // Its way Home, as an app's window on a computer has (DESIGN.md, App windows): Home brings the
                    // launcher forward and keeps this window open, as Window → Home Screen does.
                    let ring = format!("data:image/png;base64,{}", BASE64.encode(include_bytes!("../../../icons/Logo.png")));
                    screen.breadcrumb = Some(renderer::Breadcrumb { ring, icon, name: name.clone(), start: BAR.1 as f32, home: Box::new(computer::summon) });
                }
                windows.open(options, Ok(screen), on_event)
            }
            Err(error) => Err(error),
        };
        let _ = opened_to.send(opened.map(|id| { let _ = window.set(id); }).map_err(|error| format!("{error:#}")));
        if let Kind::Launcher(told) = &mut kind
            && let Some(told) = told.take()
        {
            let _ = told.send(window.get().copied());
        }
    });
    match opened.await {
        Ok(Ok(())) => Ok(true),
        Ok(Err(why)) => anyhow::bail!("its window didn't open: {why}"),
        Err(_) => anyhow::bail!("its device's native windows didn't start: {}", renderer::platform::unstarted().unwrap_or_else(|| "it didn't say why".into())),
    }
}

/// Loads the design tokens and styles, which come with the UI library, as its pages' do, and a page's app's styles.
/// → the number of its page's own tokens, where its styles have them, which are its screen's alone
/// (`tokens::load_page`).
fn load(tables: &[String]) -> anyhow::Result<u32> {
    if let [tokens, styles @ ..] = tables {
        renderer::tokens::load_table(tokens)?;
        for styles in styles {
            renderer::load_styles(styles)?;
        }
    }
    Ok(match tables {
        [_, _, own] => renderer::tokens::load_page(own)?,
        _ => 0,
    })
}

/// Where a page is drawn: in its own window, or in a frame, by its node, of a window's page.
#[derive(Clone)]
enum Place {
    Window(Arc<OnceLock<WindowId>>),
    Frame(Arc<OnceLock<WindowId>>, renderer::tree::Id),
}

/// A page whose JS has started, on its thread, and its channel to its app's window, with what it's asked over it.
struct Started {
    js: anyhow::Result<Thread>,
    to: UnboundedSender<Value>,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    page: Url,
}

/// Starts `entrypoint`'s page, its `module`, at `link`, for `subject`, drawn at `place`: its channel to its app's
/// window, and its JS on a thread of its own, its files each asked for over that as the sandbox imports it.
fn start(entrypoint: &str, subject: Option<&str>, link: &str, launcher: bool, place: Place, module: &Url, page: Url) -> anyhow::Result<Started> {
    let native = Native::get()?;
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let to = channel(entrypoint, subject, link, launcher, asked.clone(), native, place)?;
    let kept = crate::app().shell.home()?.join("native");
    // Each build keeps its pages' compiled JS apart, as another build's is no use to it: what the builds before kept goes,
    // once, as this one first starts a page.
    static PRUNED: std::sync::Once = std::sync::Once::new();
    PRUNED.call_once(|| {
        for before in std::fs::read_dir(&kept).into_iter().flatten().flatten().filter(|entry| entry.file_name() != ::client::BUILD) {
            if let Err(error) = std::fs::remove_dir_all(before.path()) {
                eprintln!("{} wasn't removed: {error}", before.path().display());
            }
        }
    });
    let cache = kept.join(::client::BUILD);
    let (files_to, files_asked) = (to.clone(), asked.clone());
    let next = std::sync::atomic::AtomicU64::new(0);
    let files: Files = Box::new(move |url: &Url| {
        let id = format!("f{}", next.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let (answer_to, answer) = mpsc::channel();
        files_asked.lock().expect("nothing panics holding it").insert(id.clone(), Asked::File(answer_to));
        let _ = files_to.send(json!({"id": id, "file": url.as_str()}));
        let file = answer.recv()?.map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(String::from_utf8(file)?)
    });
    let js = Thread::start(module, files, Some(cache), Clock::real(), cfg!(mobile));
    Ok(Started { js, to, asked, page })
}

impl Started {
    /// Its screen, on the native thread: what its page asks answered as `state` says, and a picture it names by an
    /// address fetched as one of its app's files, as a web view's page loads it.
    fn screen(self, state: State, room: Size<f32>, insets: renderer::geometry::Edges<f32>) -> anyhow::Result<Screen> {
        let Started { js, to, asked, page } = self;
        let state = Arc::new(Mutex::new(state));
        let (respond_to, respond_asked) = (to.clone(), asked.clone());
        let asked_by_app: renderer::Asked = Box::new(move |message, appearance| respond(message, appearance, &state, &respond_to, &respond_asked));
        let mut screen = Screen::new(js?, Some(asked_by_app), room, insets);
        let next_picture = std::cell::Cell::new(0u64);
        screen.fetch = Some(Box::new(move |src: &str| {
            let Ok(address) = page.join(src) else { return };
            let id = format!("i{}", next_picture.replace(next_picture.get() + 1));
            asked.lock().expect("nothing panics holding it").insert(id.clone(), Asked::Picture(src.to_owned()));
            let _ = to.send(json!({"id": id, "file": address.as_str()}));
        }));
        Ok(screen)
    }
}

/// Starts the page that frame `node` of `window`'s page shows, `src` (`<app>/<page>[#<fragment>]`), for `subject`, and
/// hands it to that page's screen: its own app's, in a sandbox of its own, calling with that app's permissions, as its
/// own window's page would, and drawn in the frame's box.
async fn frame(window: Arc<OnceLock<WindowId>>, node: renderer::tree::Id, src: &str, subject: Option<&str>) -> anyhow::Result<()> {
    let entrypoint = src.split_once('#').map_or(src, |(page, _)| page);
    let Some(Page { page, module, trusted, tables, .. }) = probe(entrypoint, subject, true).await? else {
        anyhow::bail!("it's no page of an app's this host draws")
    };
    let tokens_page = load(&tables)?;
    let started = start(entrypoint, subject, src, false, Place::Frame(window.clone(), node), &module, page)?;
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    let fragment = src.split_once('#').map(|(_, fragment)| fragment.to_owned());
    let state = State { chosen: None, trusted, entrypoint: entrypoint.to_owned(), subject: subject.map(str::to_owned), view, next: 0, launcher: false, framed: true, fragment, sheet: None, unopened: None, window: window.clone() };
    let src = src.to_owned();
    Native::get()?.run(move |windows| {
        let Some(id) = window.get() else { return };
        windows.with(*id, |parent| match started.screen(state, Size { width: 0.0, height: 0.0 }, Default::default()) {
            Ok(mut screen) => {
                screen.page = tokens_page;
                parent.framed(node, &src, screen)
            }
            Err(error) => parent.mismatched = Some(format!("its frame's page, {src}, didn't start: {error:#}")),
        });
    });
    Ok(())
}

/// The row at a window's top that its breadcrumb is in, with its page's top bar after it (a unified toolbar), 52
/// points, as macOS's System Settings and Raycast have it, and where the breadcrumb starts in it: on a Mac 8 points
/// after its traffic lights over the page, three of 14 points, 20 apart, from 20 points in, as AppKit puts them in a
/// unified toolbar; elsewhere the same height, under the window's own frame.
#[cfg(target_os = "macos")]
const BAR: (f64, f64) = (52.0, 20.0 + 2.0 * 20.0 + 14.0 + 8.0);
#[cfg(all(desktop, not(target_os = "macos")))]
const BAR: (f64, f64) = (52.0, 12.0);
/// A phone's window has none: the activity keeps the page clear of the system's bars (`MainActivity`).
#[cfg(mobile)]
const BAR: (f64, f64) = (0.0, 0.0);

/// What a native window asks of a computer's launcher (`crate::computer`), as a phone has it: its Home Screen is under
/// every app's window, so Home closes them; a `one:` link opens in its page; and it has no launcher panel to size or
/// put away. Its launcher is its root window, where it's drawn natively (`launcher`), or else its page's.
#[cfg(mobile)]
pub(crate) mod computer {
    use std::sync::Mutex;

    pub struct Size {
        pub width: f64,
        pub height: f64,
    }
    pub const SIZE: Size = Size { width: 0.0, height: 0.0 };
    pub const CORNER: f64 = 0.0;
    pub const NAME: &str = "";

    /// The launcher's root window, while it's drawn natively, and its page.
    static LAUNCHER: Mutex<Option<(renderer::platform::WindowId, String)>> = Mutex::new(None);

    /// Opens the launcher's page `entrypoint` as the root window, under every app's. → whether it did; if not, its
    /// page shows the web launcher.
    pub async fn open(entrypoint: String) -> bool {
        let Some(id) = super::launcher(&entrypoint).await else { return false };
        *LAUNCHER.lock().expect("nothing panics holding it") = Some((id, entrypoint));
        true
    }

    /// Hides the launcher's root window while the page shows an app over it, which it draws itself, or shows it.
    pub fn shown(shown: bool) {
        let Some((id, _)) = *LAUNCHER.lock().expect("nothing panics holding it") else { return };
        if let Ok(native) = renderer::platform::Native::get() {
            native.run(move |windows| windows.hide(id, !shown));
        }
    }

    /// Home: every app's window closes, and the launcher's stays.
    pub fn summon() {
        let launcher = LAUNCHER.lock().expect("nothing panics holding it").as_ref().map(|(_, entrypoint)| format!("|{entrypoint}"));
        crate::hosts::close_all_but(launcher.as_deref());
    }

    pub fn again(link: Option<String>) {
        if let Some(opens) = link.and_then(|link| crate::link(&link)) {
            crate::app().shell.emit("open", serde_json::json!(opens));
        }
    }

    pub fn hide(_: bool) {}
    pub fn resize(_: Option<f64>, _: Option<f64>) {}

    /// The launcher can't be drawn natively after all (its page needs a newer host, or its servers are ahead): its
    /// window closes, and the page shows the web launcher.
    pub fn native(_: Option<renderer::platform::WindowId>) {
        close();
        crate::app().shell.emit("launcher", serde_json::json!({"native": false}));
    }

    /// Its page was updated: its window opens again on the new version.
    pub fn reload_native() {
        let Some(entrypoint) = close() else { return };
        crate::spawn(async move {
            if !open(entrypoint).await {
                crate::app().shell.emit("launcher", serde_json::json!({"native": false}));
            }
        });
    }

    /// Closes the launcher's window. → its page.
    fn close() -> Option<String> {
        let (id, entrypoint) = LAUNCHER.lock().expect("nothing panics holding it").take()?;
        if let Ok(native) = renderer::platform::Native::get() {
            native.run(move |windows| windows.close(id));
        }
        Some(entrypoint)
    }
}

/// What the window keeps of where it is.
struct State {
    /// The Colour Mode chosen on this device, or none for as the device is.
    chosen: Option<String>,
    /// It's the launcher's page, not an app's window, or a page in another's frame.
    launcher: bool,
    framed: bool,
    /// The place in its page the link it was opened at names: what follows its `#`.
    fragment: Option<String>,
    trusted: bool,
    entrypoint: String,
    /// Who it was opened for, a group or the network, or none for its person: whoever its requests are for.
    subject: Option<String>,
    /// It's the client's recovery key window (`recovery`).
    sheet: Option<Sheet>,
    /// It's the client's window in place of an app's that didn't open (`unopened`).
    unopened: Option<Unopened>,
    /// The window it's in, once it's open.
    window: Arc<OnceLock<WindowId>>,
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
    let launcher = state.lock().expect("nothing panics holding it").launcher;
    // The app asks to be left (`leave`): on a computer, where its window stays open, Home, as its breadcrumb's is; the
    // launcher is put away, the keyboard back with the app that had it.
    let framed = state.lock().expect("nothing panics holding it").framed;
    // A page in a frame is its frame's page's, which stays: it leaves nothing.
    if method == "ui/notifications/request-teardown" && !framed {
        if launcher { computer::hide(true) } else { computer::summon() }
        return None;
    }
    // The launcher is as tall as its page says it is, as Raycast's and Spotlight's panels grow and shrink, and as wide.
    if method == "ui/notifications/size-changed" && launcher {
        computer::resize(message["params"]["width"].as_f64(), message["params"]["height"].as_f64());
        return None;
    }
    // A phone's launcher with something open over its page (a menu, a category): the page has Android's Back come to
    // it, which shuts that (`notifications/one/back`), rather than leave the app.
    #[cfg(mobile)]
    if method == "notifications/one/over" && launcher {
        crate::app().shell.emit("launcher", json!({"over": message["params"]["over"] == true}));
        return None;
    }
    // Its page logs its errors as an MCP App logs to its host, and Telemetry hears them as its app's, as a web view's
    // (`exception` in clients/app/page/client.js).
    if method == "notifications/message" && message["params"]["level"] == "error" {
        let data = &message["params"]["data"];
        let app = state.lock().expect("nothing panics holding it").entrypoint.split('/').next().unwrap_or_default().to_owned();
        let error: String = data["message"].as_str().unwrap_or_default().chars().take(500).collect();
        let path = data["file"].as_str().and_then(|file| Url::parse(file).ok()).map(|file| file.path().to_owned());
        if let Ok(views) = crate::views() {
            views.client.report(json!({"name": "exception", "error": error, "attributes": {"code.file.path": path, "code.line.number": data["line"], "one.app": app}}));
        }
        return None;
    }
    // Any other notification needs no answer, and none is heard here yet.
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
                "deviceCapabilities": {"touch": cfg!(mobile), "hover": cfg!(desktop)},
                // A page in a frame is named by nothing above it, nor is the recovery key window's: its first screen
                // says its heading.
                "breadcrumb": cfg!(desktop) && !state.framed && state.sheet.is_none(),
                // The place its link names, as its address's `#` says on the DOM (`link()`).
                "link": state.fragment,
                "connection": "online",
            },
        }))),
        "ping" => Some(Ok(json!({}))),
        // A `one:` link opens as the device's own do, which the launcher's page opens in its app's window; a web
        // address opens in the device's browser, outside the app, as a web view's window does.
        "ui/open-link" => match params["url"].as_str().unwrap_or_default() {
            // The launcher opens an app in its own window, for whoever it's for, and goes, as the web launcher's
            // page does (`go` in client.js).
            #[cfg(desktop)]
            url if url.starts_with("one:") && state.launcher => {
                let (link, subject) = (url["one:".len()..].to_owned(), params["subject"].as_str().map(str::to_owned));
                if params["behind"] != true {
                    crate::spawn(go(link, subject));
                }
                Some(Ok(json!({})))
            }
            // A phone's launcher opens it in the page, which draws it over the launcher, for whoever it's for, or with
            // `behind` only starts it, as a finger lands on its icon (`go` in client.js).
            #[cfg(mobile)]
            url if url.starts_with("one:") && state.launcher => {
                let go = json!({"link": &url["one:".len()..], "subject": params["subject"], "behind": params["behind"] == true});
                crate::app().shell.emit("go", go);
                Some(Ok(json!({})))
            }
            url if url.starts_with("one:") => {
                computer::again(Some(url.to_owned()));
                Some(Ok(json!({})))
            }
            url if url.starts_with("https://") || url.starts_with("http://") => {
                let _ = to.send(json!({"id": "browser", "command": "open_url", "args": {"url": url}}));
                Some(Ok(json!({})))
            }
            url => Some(Err(json!({"code": -32602, "message": format!("{url} isn't a link this window opens")}))),
        },
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
                // What the client says of itself, as the window page answers (`respond` in client.js); a computer's
                // launcher's page lays itself out as one (`client().launcher` in home.tsx), and a phone's is its Home
                // Screen. Only Android's page pins shortcuts, and no native window does yet.
                let mut client = crate::client().unwrap_or_else(|_| json!({}));
                client["theme"] = json!(state.chosen);
                client["launcher"] = json!(state.launcher && cfg!(desktop));
                client["mac"] = json!(cfg!(target_os = "macos"));
                client["pinsShortcuts"] = json!(false);
                Some(Ok(client))
            } else {
                // The rest change the device, which the client does (`change` in hosts.rs), answered as it's heard.
                state.next += 1;
                let at = format!("m{}", state.next);
                asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
                let _ = to.send(json!({"id": at, "command": "change", "args": params}));
                return None;
            }
        }
        // A shortcut on the device's own home screen is Android's page's to pin, and a haptic its page's to play: a
        // native window has neither yet, which it says, as a device without them does (client.js).
        "one/pin-shortcut" => Some(Err(json!({"code": -32000, "message": "This device can't put that app on its home screen"}))),
        "one/haptic" => Some(Err(json!({"code": -32000, "message": "This device plays no haptics"}))),
        // A person's recovery key is shown and typed in the client's own window over the app (`recovery`), for whoever
        // the app asks for, and the app is told only whether it was done (`whom` in client.js).
        "one/recovery-key" if state.sheet.is_none() => {
            let said = params["subject"].as_str().filter(|_| state.trusted).map(str::to_owned);
            let subject = said.or_else(|| state.subject.clone());
            state.next += 1;
            let at = format!("m{}", state.next);
            asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
            let _ = to.send(json!({"id": at, "command": "recovery", "args": {"typed": params["typed"] == true, "subject": subject, "owner": state.window.get()}}));
            return None;
        }
        // What the recovery key window asks, which only it may: what it's for, the key it shows, the key it was given,
        // and that it's done, which closes it.
        method if method == "one/recovery" || method.starts_with("one/recovery/") => match &state.sheet {
            None => Some(Err(json!({"code": -32601, "message": format!("Only the client's own page asks {method}")}))),
            Some(sheet) => match method {
                "one/recovery" => Some(Ok(json!({"typed": sheet.typed}))),
                "one/recovery/key" | "one/recovery/recover" => {
                    let (command, args) = match method {
                        "one/recovery/key" if !sheet.typed => ("recovery_key", json!({"subject": sheet.subject})),
                        "one/recovery/recover" if sheet.typed => ("recover", json!({"key": params["key"], "subject": sheet.subject})),
                        _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("This window doesn't ask {method}")}})),
                    };
                    state.next += 1;
                    let at = format!("m{}", state.next);
                    asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
                    let _ = to.send(json!({"id": at, "command": command, "args": args}));
                    return None;
                }
                "one/recovery/done" => {
                    if let Some(told) = sheet.done.lock().expect("nothing panics holding it").take() {
                        let _ = told.send(params["done"] == true);
                    }
                    let window = state.window.clone();
                    if let Ok(native) = Native::get() {
                        native.run(move |windows| {
                            if let Some(id) = window.get() {
                                windows.close(*id);
                            }
                        });
                    }
                    Some(Ok(json!({})))
                }
                _ => Some(Err(json!({"code": -32601, "message": format!("This window doesn't ask {method}")}))),
            },
        },
        // What the window of an app that didn't open asks, which only it may: what it says, and to open the app again,
        // in its place.
        method if method == "one/unopened" || method == "one/unopened/retry" => match &state.unopened {
            None => Some(Err(json!({"code": -32601, "message": format!("Only the client's own page asks {method}")}))),
            Some(unopened) if method == "one/unopened" => Some(Ok(json!({"name": unopened.name, "why": unopened.why}))),
            Some(unopened) => {
                close(&state.window);
                if let Err(error) = crate::hosts::reopen(unopened.entrypoint.clone(), unopened.subject.clone(), unopened.link.clone()) {
                    eprintln!("{} didn't open again: {error}", unopened.entrypoint);
                }
                Some(Ok(json!({})))
            }
        },
        // The client's own windows ask nothing else: they're no app's, and call no tool.
        method if (state.sheet.is_some() || state.unopened.is_some()) && !method.starts_with("ui/") && method != "ping" && method != "one/client" => {
            Some(Err(json!({"code": -32601, "message": format!("This window doesn't ask {method}")})))
        }
        method if method.starts_with("ui/") => Some(Err(json!({"code": -32601, "message": format!("This client has no {method}")}))),
        _ => None,
    };
    // A file the page chose goes where the network keeps it, for a tool of this window's app (`upload` in hosts.rs).
    if answered.is_none() && method == "one/upload" {
        state.next += 1;
        let at = format!("m{}", state.next);
        let app = state.entrypoint.split('/').next().unwrap_or_default().to_owned();
        asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
        let _ = to.send(json!({"id": at, "command": "upload", "args": {"app": app, "body": params["body"]}}));
        return None;
    }
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
            // It asks for whoever its window was opened for, a group or the network, unless a trusted app's page says
            // whom (`whom` in client.js).
            if let Some(subject) = state.subject.as_ref().filter(|_| !meta.get("subject").is_some_and(Value::is_string)) {
                meta.insert("subject".into(), json!(subject));
            }
            params["_meta"] = Value::Object(meta);
            asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
            let _ = to.send(json!({"id": at, "command": "request", "args": {"view": state.view, "method": method, "params": params}}));
            None
        }
    }
}

/// Opens what a `one:` link from the launcher says, `<app>[/<page>][#<fragment>]`, for `subject`, in its own window,
/// and puts the launcher away, as the web launcher's page does (`go` in client.js): an app by its name opens its first
/// page, as its app lists it last; the launcher's own app brings it forward.
#[cfg(desktop)]
pub(crate) async fn go(link: String, subject: Option<String>) {
    let (entrypoint, fragment) = link.split_once('#').map_or((link.as_str(), None), |(page, fragment)| (page, Some(fragment)));
    let page = match entrypoint.contains('/') {
        true => Some(entrypoint.to_owned()),
        false => first(entrypoint, subject.as_deref()).await,
    };
    let Some(page) = page else { return eprintln!("there's no {entrypoint} to open") };
    if computer::is_launcher(&page) {
        return computer::summon();
    }
    let link = fragment.map_or_else(|| page.clone(), |fragment| format!("{page}#{fragment}"));
    match crate::hosts::open_window(page, subject, link) {
        Ok(()) => computer::hide(false),
        Err(error) => eprintln!("{error}"),
    }
}

/// The page `app` opens first, for `subject`, as its apps are listed (`launcherIn` in client.js): the last one listed,
/// or the launcher's, for its own app.
#[cfg(desktop)]
pub(crate) async fn first(app: &str, subject: Option<&str>) -> Option<String> {
    let listed = crate::hosts::command("native", "apps", json!({"subject": subject})).await.ok()?;
    let pages: Vec<(String, &Value)> = pages(&listed).collect();
    pages
        .iter()
        .find(|(page, meta)| meta["category"] == "launcher" && page.split('/').next() == Some(app))
        .or_else(|| pages.iter().rev().find(|(page, _)| page.split('/').next() == Some(app)))
        .map(|(page, _)| page.clone())
}

/// The pages listed, each `<app>/<page>`, with its `_meta`, as an MCP App's are at ui://<app>/<version>/<page>.
#[cfg(desktop)]
pub(crate) fn pages(listed: &Value) -> impl Iterator<Item = (String, &Value)> {
    listed["resources"].as_array().into_iter().flatten().filter(|resource| resource["mimeType"] == "text/html;profile=mcp-app").filter_map(|resource| {
        let mut parts = resource["uri"].as_str()?.strip_prefix("ui://")?.split('/');
        let app = parts.next()?;
        parts.next()?;
        Some((format!("{app}/{}", parts.collect::<Vec<_>>().join("/")), &resource["_meta"]))
    })
}

/// Talks with this process's own channel for the app's windows (`hosts::talk`), as a web view's window does over its
/// socket, saying hello as it does: what it hears is done to the window on the native thread. → where to send it a
/// message.
fn channel(
    entrypoint: &str,
    subject: Option<&str>,
    link: &str,
    launcher: bool,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    native: Native,
    place: Place,
) -> anyhow::Result<UnboundedSender<Value>> {
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    let (window, framed) = match &place {
        Place::Window(window) => (window.clone(), None),
        Place::Frame(window, node) => (window.clone(), Some(*node)),
    };
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    crate::spawn(crate::hosts::talk(theirs));
    let (to, mut out) = unbounded_channel::<Value>();
    let _ = to.send(json!({"hello": {"entrypoint": entrypoint, "subject": subject, "link": link, "frame": framed.is_some()}}));
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
    let (entrypoint, subject, link) = (entrypoint.to_owned(), subject.map(str::to_owned), link.to_owned());
    // Its app's host, as a server says which were loaded against what's gone (`stale`): `<app>[.<subject, last part first>]`.
    let app_host = origin(&entrypoint, subject.as_deref()).trim_start_matches("oneapp://").trim_end_matches(".localhost/").to_owned();
    crate::spawn(async move {
        let mut lines = BufReader::new(read).lines();
        // Reload, in its notice that its page was updated: the window opens again, on the page's new version.
        let reload = {
            let (native, window) = (native.clone(), window.clone());
            let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
            move || {
                // The launcher's main thread closes it and opens it again, shown again if it was (`Told::Reload`).
                if launcher {
                    return computer::reload_native();
                }
                // A frame's page starts again in it.
                if let Some(node) = framed {
                    let window = window.clone();
                    return native.run(move |windows| {
                        if let Some(id) = window.get() {
                            windows.with(*id, |screen| screen.reload_frame(node));
                        }
                    });
                }
                let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
                let window = window.clone();
                native.run(move |windows| {
                    if let Some(id) = window.get() {
                        windows.close(*id);
                    }
                    if let Err(error) = crate::hosts::reopen(entrypoint, subject, link) {
                        eprintln!("{error}");
                    }
                })
            }
        };
        // The window this is, once the native thread has opened it, a job running after the one that opened it; or the
        // page in its frame, once it's there.
        let on = |job: Box<dyn FnOnce(&mut Screen) + Send>| {
            let window = window.clone();
            native.run(move |windows| {
                if let Some(id) = window.get() {
                    windows.with(*id, |screen| match framed {
                        Some(node) => drop(screen.frame_mut(node).map(job)),
                        None => job(screen),
                    });
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
                    // Behind its servers, the launcher is the web launcher's, which says Update Required.
                    "state" if launcher && payload["connection"] == "behind" => computer::native(None),
                    // Over its page, while its network doesn't answer, as over an app on a phone (DESIGN.md).
                    "state" if framed.is_none() => on(Box::new(move |screen| {
                        screen.notice = match payload["connection"].as_str() {
                            Some("reconnecting") => Some(renderer::Notice { text: "Reconnecting…".into(), action: None }),
                            Some("offline") => Some(renderer::Notice { text: "Can't reach your network".into(), action: None }),
                            _ => screen.notice.take().filter(|notice| notice.action.is_some()),
                        }
                    })),
                    // Its page, or the UI library it runs, was loaded against a version that's gone: it loads again by
                    // itself once its window isn't in use, now if it isn't, as a desktop app or a phone's updates,
                    // never under its person (adf8f19b8). A frame's page waits on the window it's in.
                    "stale" if payload.as_array().is_some_and(|stale| stale.iter().filter_map(Value::as_str).any(|host| host.split('.').next() == Some("ui") || host == app_host)) => {
                        let (reload, window) = (reload.clone(), window.clone());
                        native.run(move |windows| {
                            if let Some(id) = window.get() {
                                windows.with(*id, |screen| match screen.active {
                                    true => screen.when_unused.push(Box::new(reload)),
                                    false => reload(),
                                });
                            }
                        });
                    }
                    // The launcher, linked to, comes forward.
                    "open" if launcher => computer::summon(),
                    // Its window comes forward, and its page is told the place the link names, as a page's `hashchange`.
                    "open" if framed.is_none() => {
                        let window = window.clone();
                        let fragment = payload.as_str().and_then(|link| link.split_once('#')).map(|(_, fragment)| fragment.to_owned());
                        native.run(move |windows| {
                            if let Some(id) = window.get() {
                                windows.activate(*id);
                                if let Some(fragment) = fragment {
                                    windows.with(*id, |screen| screen.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"link": fragment}})));
                                }
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
                Asked::Picture(src) => {
                    let data = result.ok().and_then(|file| Some(format!("data:{};base64,{}", file["kind"].as_str()?, file["body"].as_str()?)));
                    on(Box::new(move |screen| screen.fetched(&src, data)));
                }
                Asked::Message(id) => {
                    let answer = match result {
                        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": error.as_str().map_or_else(|| error.to_string(), str::to_owned)}}),
                    };
                    on(Box::new(move |screen| screen.message(&answer)));
                }
            }
        }
        // Told to end, or the One App is: the window closes, which a frame's page leaves to the window it's in.
        if framed.is_some() {
            return;
        }
        let window = window.clone();
        native.run(move |windows| {
            if let Some(id) = window.get() {
                windows.close(*id);
            }
        });
    });
    Ok(to_window)
}

/// The Inspector app's tools of this device: its native windows, each by its id (`window`), as the network asks them
/// of the app's client (`client::DeviceTools`), answered on the native thread.
pub struct Inspector;

impl ::client::DeviceTools for Inspector {
    fn app(&self) -> &'static str {
        "inspector"
    }

    fn call<'a>(&'a self, tool: &'a str, arguments: Value) -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let native = match Native::get() {
                Ok(native) => native,
                Err(error) => return Ok(refused(&format!("no native window can open here: {error:#}"))),
            };
            if tool == "windows" {
                let (listed_to, listed) = tokio::sync::oneshot::channel();
                native.run(move |windows| drop(listed_to.send(windows.list())));
                let windows = json!({"device": ::client::name(), "windows": listed.await.unwrap_or_default()});
                return Ok(json!({"content": [{"type": "text", "text": windows.to_string()}], "structuredContent": windows}));
            }
            let mut request = match arguments {
                Value::Object(inputs) => inputs,
                _ => Default::default(),
            };
            let window = request.remove("window").and_then(|window| window.as_u64());
            request.remove("device");
            request.insert("v".into(), json!(renderer::inspect::VERSION));
            request.insert("id".into(), json!(1));
            request.insert("op".into(), json!(tool));
            let request = match renderer::inspect::Request::parse(&Value::Object(request).to_string()) {
                Ok(request) => request,
                Err(refused_with) => return Ok(refused(refused_with["error"].as_str().unwrap_or("that request isn't one"))),
            };
            let (answer_to, answer) = tokio::sync::oneshot::channel::<Value>();
            native.run(move |windows| {
                let open: Vec<WindowId> = windows.list().iter().filter_map(|listed| listed["window"].as_u64()).collect();
                let reply: renderer::Reply = Box::new(move |answered| drop(answer_to.send(answered)));
                match window.or_else(|| (open.len() == 1).then(|| open[0])) {
                    Some(id) => windows.inspect(id, request, reply),
                    None => reply(renderer::inspect::answer(&request.id, Err(anyhow::anyhow!("Say which native window, by the `window` that `windows` lists: none or more than one is open here.")))),
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

/// Whether a native window holds something its person typed that its app hasn't taken (`Windows::typed`).
pub async fn typed() -> bool {
    let Some(native) = Native::started() else { return false };
    let (told, answer) = tokio::sync::oneshot::channel();
    native.run(move |windows| {
        let _ = told.send(windows.typed());
    });
    answer.await.unwrap_or(false)
}

/// An iPhone's native windows are tao's, in the event loop Tauri runs: every one of its events goes to them first
/// (`renderer::platform::ios::event`), and one of theirs goes no further; a job for them wakes the loop.
#[cfg(target_os = "ios")]
pub(crate) struct Ios;

#[cfg(target_os = "ios")]
mod ios {
    use tauri::EventLoopMessage;
    use tauri_runtime_wry::tao::event::Event;
    use tauri_runtime_wry::tao::event_loop::{ControlFlow, EventLoopProxy, EventLoopWindowTarget};
    use tauri_runtime_wry::{Context, EventLoopIterationContext, Message, Plugin, PluginBuilder, WebContextStore};

    impl PluginBuilder<EventLoopMessage> for super::Ios {
        type Plugin = super::Ios;

        fn build(self, context: Context<EventLoopMessage>) -> super::Ios {
            let proxy = context.proxy.clone();
            renderer::platform::ios::attach(move || drop(proxy.send_event(Message::Task(Box::new(|| {})))));
            self
        }
    }

    impl Plugin<EventLoopMessage> for super::Ios {
        fn on_event(
            &mut self,
            event: &Event<Message<EventLoopMessage>>,
            event_loop: &EventLoopWindowTarget<Message<EventLoopMessage>>,
            _: &EventLoopProxy<Message<EventLoopMessage>>,
            _: &mut ControlFlow,
            _: EventLoopIterationContext<'_, EventLoopMessage>,
            _: &WebContextStore,
        ) -> bool {
            renderer::platform::ios::event(event, event_loop)
        }
    }
}
