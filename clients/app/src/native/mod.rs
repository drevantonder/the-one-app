//! An app's window drawn natively, where its entrypoint's page needs a runtime version this
//! program has (`runtimeVersion`, core/ui/src/drawing-primitives/renderer.ts's `RUNTIME_VERSION`, `renderer::RUNTIME_VERSION`):
//! this process, the One App's, draws it on its native thread (`renderer::platform`) beside every other app's window, so another open
//! app costs a few MB, and its JS runs in a QuickJS sandbox of its own, on a thread of its own. Any other app's window is a web view in a
//! process of its own, as before (`host`), and so is one that needs a later runtime version.
//!
//! It asks what a page would, over the same channel a web view's window does (`hosts`), here an in-memory one: its
//! files, and what the page asks of its client, which an MCP App asks its host (JSON-RPC). What a web view's window
//! page answers itself (`respond` in page/bridge.js) is answered here: `ui/initialize`, `ping` and `one/client`'s
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
use crate::{NAME, computer};
#[cfg(mobile)]
use computer::NAME;

mod channel;
mod inspector;
mod launcher;
mod opening;
mod own_pages;
mod snapshots;
mod start;
mod unopened;

use channel::*;
pub(crate) use inspector::*;
pub(crate) use launcher::*;
pub(crate) use opening::*;
pub(crate) use own_pages::*;
pub(crate) use snapshots::*;
use start::*;
pub(crate) use unopened::*;

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

/// Closes the window `shown`, once it's open: on the windows' thread, after the job that opens it.
fn close(shown: &Arc<OnceLock<WindowId>>) {
    let shown = shown.clone();
    if let Ok(native) = Native::get() {
        native.run(move |windows| {
            if let Some(id) = shown.get().copied() {
                windows.close(id);
            }
        });
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
    tables: Vec<Table>,
    /// Its name and icon, as a `data:` address, as its app lists them, for the window's title.
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

/// Whether `entrypoint`'s page needs a runtime version this program has, its tables too, and if it does, what
/// it's drawn with.
async fn probe(entrypoint: &str, subject: Option<&str>) -> anyhow::Result<Option<Page>> {
    match listing(entrypoint, subject).await? {
        Some(resource) => read(entrypoint, subject, &resource).await,
        None => Ok(None),
    }
}

/// `entrypoint`'s page as its app lists it for `subject`, where it needs a runtime version this program has.
async fn listing(entrypoint: &str, subject: Option<&str>) -> anyhow::Result<Option<Value>> {
    let listed = crate::hosts::command("native", "apps", json!({"subject": subject})).await.map_err(|error| anyhow::anyhow!(error))?;
    Opening::step(&format!("{}|{entrypoint}", subject.unwrap_or_default()), "listed");
    let Some(resource) = listed_page(&listed, entrypoint) else { return Ok(None) };
    // A page that names no runtime version (a component's or a sign-in page's, which no entrypoint names) is tried, as
    // every page is drawn natively: one this program can't draw says so as it starts.
    let needed = resource["_meta"]["runtimeVersion"].as_u64();
    Ok(needed.is_none_or(|needed| needed <= u64::from(renderer::RUNTIME_VERSION)).then(|| resource.clone()))
}

/// What `entrypoint`'s page, listed as `resource`, is drawn with, where its tables need a runtime version this program
/// has too.
async fn read(entrypoint: &str, subject: Option<&str>, resource: &Value) -> anyhow::Result<Option<Page>> {
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    let trusted = resource["_meta"]["trusted"] == true;
    // Its icon, as its app lists it, as a `data:` address, for the window's title.
    let icon = match resource["icons"][0]["src"].as_str() {
        Some(src) if src.starts_with("data:") => Some(src.to_owned()),
        Some(src) => crate::hosts::file(src).await.ok().and_then(|file| Some(format!("data:{};base64,{}", file["kind"].as_str()?, file["body"].as_str()?))),
        None => None,
    };
    let page = Url::parse(&origin(entrypoint, subject))?.join(entrypoint)?;
    let html = crate::hosts::file(page.as_str()).await.map_err(|error| anyhow::anyhow!(error))?;
    let html = BASE64.decode(html["body"].as_str().unwrap_or_default().as_bytes())?;
    Opening::step(&view, "page read");
    let module = module_script(&String::from_utf8_lossy(&html)).ok_or_else(|| anyhow::anyhow!("its page loads no module"))?;
    // Its tables: the UI library's, and its app's styles, which its build writes beside every page (`core/ui/app.js`).
    let Some(tables) = tables(&[page.join("/ui/client/tokens.json")?, page.join("/ui/client/styles.json")?, page.join("styles.json")?]).await? else { return Ok(None) };
    Opening::step(&view, "tables read");
    Ok(Some(Page { module: page.join(&module)?, page, trusted, tables, name: listed_name(resource), icon }))
}

/// A page's name, as its app lists it (`resource`).
fn listed_name(resource: &Value) -> Option<String> {
    resource["name"].as_str().filter(|name| !name.is_empty()).map(str::to_owned)
}

/// A table a page is drawn with (`Page::tables`): the file it is, by the version it's of, which never changes
/// (`hosts::resource`), and what's in it, where it isn't loaded already (`LOADED`).
struct Table {
    file: String,
    text: Option<String>,
}

/// The tables loaded, each by its file (`Table`), so a page opened on what's loaded already reads and parses none
/// again, where every open did both: a Surface's opens took about 60 ms (p50) to read them alone. The token table
/// loaded now, which another replaces (`renderer::tokens::load_table`), and each style table loaded, which stay
/// (`renderer::load_styles`), with its page's own tokens' number where it's a page's (`renderer::tokens::load_page`).
/// A new build of the UI library or an app is new files, which are loaded as its pages open.
static LOADED: Mutex<(Option<String>, Vec<(String, u32)>)> = Mutex::new((None, Vec::new()));

/// Whether table `at` of a page's, `file`, is loaded: its token table, if it's the one loaded now; another, if it's
/// been loaded.
fn loaded(at: usize, file: &str) -> bool {
    let loaded = LOADED.lock().expect("nothing panics holding it");
    if at == 0 { loaded.0.as_deref() == Some(file) } else { loaded.1.iter().any(|(known, _)| known == file) }
}

/// The tables at `addresses`, those not loaded already read, or none where one needs a runtime version this program
/// hasn't, so its page is a web view's.
async fn tables(addresses: &[Url]) -> anyhow::Result<Option<Vec<Table>>> {
    let mut tables = Vec::new();
    for (at, table) in addresses.iter().enumerate() {
        let file = crate::hosts::resource(table.as_str()).await.map_err(|error| anyhow::anyhow!("{table} didn't load: {error}"))?;
        if loaded(at, &file) {
            tables.push(Table { file, text: None });
            continue;
        }
        let read = crate::hosts::file(table.as_str()).await.map_err(|error| anyhow::anyhow!("{table} didn't load: {error}"))?;
        let read = BASE64.decode(read["body"].as_str().unwrap_or_default().as_bytes())?;
        let text = String::from_utf8(read)?;
        let v = serde_json::from_str::<Value>(&text).map(|table| table["v"].as_u64().unwrap_or(0)).unwrap_or(0);
        if v > u64::from(renderer::RUNTIME_VERSION) {
            return Ok(None);
        }
        tables.push(Table { file, text: Some(text) });
    }
    Ok(Some(tables))
}

/// What a native window is: an app's own, or the launcher, a panel the system summons, which tells whoever opens it
/// its window once it's open, or that it didn't.
enum Kind {
    /// An app's, opened from where its icon is on the screen, if it was.
    App(Option<Bounds<f32>>),
    /// The launcher, which tells whoever opens it its window, and with `Instead`, a page of the client's own in place
    /// of the Home Screen's.
    Launcher(Option<tokio::sync::oneshot::Sender<Option<WindowId>>>, Option<Instead>),
    /// The client's own, over an app's, where a recovery key is shown or typed (`recovery`).
    Sheet(Sheet),
    /// The client's own, in place of an app's that didn't open (`unopened`).
    Unopened(Unopened),
}

/// Opens a native window of `kind` for `entrypoint`'s page at `link`, for `subject`, asked to at `started`: its window
/// made as soon as it's known what it shows, while its page is read and started, which it's then given. → whether it
/// opened; not where its page needs a web view.
async fn window(started: Instant, entrypoint: &str, subject: Option<&str>, link: &str, mut kind: Kind) -> anyhow::Result<bool> {
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    Opening::begin(&view, started);
    let Some((own, listed)) = shows(&kind, entrypoint, subject).await? else {
        OPENING.lock().expect("nothing panics holding it").remove(&view);
        return Ok(false);
    };
    let name = own.as_ref().and_then(|page| page.name.clone()).or_else(|| listed.as_ref().and_then(listed_name));
    let name = name.unwrap_or_else(|| entrypoint.split('/').next().unwrap_or_default().to_owned());
    let launcher = matches!(kind, Kind::Launcher(..));

    // Its window, made at once, and its channel, which its page starts on once it's read (`started`).
    let window: Arc<OnceLock<WindowId>> = Arc::default();
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let to = channel(entrypoint, subject, link, launcher, asked.clone(), Native::get()?, Place::Window(window.clone()))?;
    let options = options(&kind, entrypoint, subject, &name);
    let title_row = options.title_row;
    let on_event = events(&kind, &to, entrypoint, subject, link, &window, started);
    let told = match &mut kind {
        Kind::Launcher(told, _) => told.take(),
        Kind::App(_) | Kind::Sheet(_) | Kind::Unopened(_) => None,
    };
    let opened = open_window(&view, options, on_event, &window, told)?;

    // Its page, read and started meanwhile.
    let page = match own {
        Some(page) => Ok(Some(page)),
        None => read(entrypoint, subject, listed.as_ref().expect("an app's page is listed")).await,
    };
    let Some(Page { page, module, trusted, tables, name: _, icon }) = page.inspect_err(|_| close(&window))? else {
        close(&window);
        OPENING.lock().expect("nothing panics holding it").remove(&view);
        return Ok(false);
    };
    let (begun, tokens_page, chosen) = start_page(&view, entrypoint, subject, &module, page, &tables, &to, &asked).await.inspect_err(|_| close(&window))?;

    // Its window shows its page.
    let fragment = link.split_once('#').map(|(_, fragment)| fragment.to_owned());
    let sheet = match &kind {
        Kind::Sheet(sheet) => Some(sheet.clone()),
        _ => None,
    };
    let unopened = match &kind {
        Kind::Unopened(unopened) | Kind::Launcher(_, Some(Instead::Unopened(unopened))) => Some(unopened.clone()),
        _ => None,
    };
    let signing_in = matches!(kind, Kind::Launcher(_, Some(Instead::SignIn)));
    let behind = matches!(kind, Kind::Launcher(_, Some(Instead::Behind)));
    // A page of the client's own in the launcher's place floats nothing of its own: the whole panel is its card.
    let instead = matches!(kind, Kind::Launcher(_, Some(_)));
    let state = State { chosen: chosen.clone(), trusted, entrypoint: entrypoint.to_owned(), subject: subject.map(str::to_owned), view: view.clone(), page: 0, next: 0, launcher, framed: false, fragment, sheet, unopened, signing_in, behind, window: window.clone() };
    // Its title bar, which its page keeps clear, as its host context's `safeAreaInsets` tells it. The launcher has none.
    let top = if launcher { 0.0 } else { BAR.0 as f32 };
    let insets = renderer::geometry::Edges { top, right: 0.0, bottom: 0.0, left: 0.0 };
    let framing = window.clone();
    let screen = move || {
        begun.screen(state, insets).map(|mut screen| {
            Opening::step(&view, "screen made");
            screen.theme = theme(chosen.as_deref());
            // A phone's is under a finger from the start, so it's drawn with its tokens' touch values from its
            // first frame (pointer: coarse), as the page is told (`deviceCapabilities`).
            screen.touch = cfg!(mobile);
            screen.page = tokens_page;
            screen.make_frame = trusted.then(|| frames(framing));
            if launcher && cfg!(desktop) {
                // Its glass is the system's, under its page, where there's one; Linux has none, so it's a
                // popover's colour (DESIGN.md, Launcher).
                screen.panel = Some(renderer::Panel { corner: computer::CORNER as f32, fill: cfg!(target_os = "linux"), whole: instead });
            } else if title_row {
                // Its app's icon and name in its title row (DESIGN.md, App windows).
                screen.title = Some(renderer::Title { icon, name, start: BAR.1 as f32 });
            }
            screen
        })
    };
    let attached = attach(&window, screen)?;

    match opened.await {
        Ok(Ok(())) => {}
        Ok(Err(why)) => anyhow::bail!("its window didn't open: {why}"),
        Err(_) => anyhow::bail!("its device's native windows didn't start: {}", renderer::platform::unstarted().unwrap_or_else(|| "it didn't say why".into())),
    }
    match attached.await {
        Ok(Err(why)) => anyhow::bail!("its page didn't start: {why}"),
        // Closed before its page was there, as its person may.
        Ok(Ok(())) | Err(_) => Ok(true),
    }
}

/// What a window of `kind` shows: a page of the client's own, or its app's, as its app lists it for `subject`, where
/// it's drawn here. None where it isn't.
async fn shows(kind: &Kind, entrypoint: &str, subject: Option<&str>) -> anyhow::Result<Option<(Option<Page>, Option<Value>)>> {
    Ok(match kind {
        Kind::Sheet(_) => Some((Some(client_page(RECOVERY, "Recovery Key").await?), None)),
        Kind::Unopened(unopened) | Kind::Launcher(_, Some(Instead::Unopened(unopened))) => Some((Some(client_page(UNOPENED, &unopened.name).await?), None)),
        Kind::Launcher(_, Some(Instead::SignIn)) => Some((Some(client_page(SIGN_IN, NAME).await?), None)),
        Kind::Launcher(_, Some(Instead::Behind)) => Some((Some(client_page(UPDATE, NAME).await?), None)),
        Kind::App(_) | Kind::Launcher(_, None) => listing(entrypoint, subject).await?.map(|resource| (None, Some(resource))),
    })
}

/// A window of `kind`'s, for `entrypoint`'s page for `subject`, called `name`.
fn options(kind: &Kind, entrypoint: &str, subject: Option<&str>, name: &str) -> WindowOptions {
    let launcher = matches!(kind, Kind::Launcher(..));
    let sheet = match kind {
        Kind::Sheet(sheet) => Some(sheet),
        _ => None,
    };
    let (size, panel) = match (launcher && cfg!(desktop), sheet) {
        (true, _) => ((computer::SIZE.width as f32, computer::SIZE.height as f32), Some(computer::CORNER)),
        (false, Some(_)) => ((440.0, 360.0), None),
        (false, None) => ((1024.0, 720.0), None),
    };
    WindowOptions {
        title: if launcher { NAME.to_owned() } else { name.to_owned() },
        app_id: ::client::NAME.to_owned(),
        size: Size { width: Pixels(size.0), height: Pixels(size.1) },
        tag: json!({"entrypoint": entrypoint, "subject": subject}),
        panel: panel.map(|corner| corner as f32),
        // The launcher's kept where it's dragged to, on each screen.
        places: (launcher && cfg!(desktop)).then(|| crate::app().shell.home().ok()).flatten().map(|home| home.join("launcher-places.json")),
        // An app's window opens where it was left, as each computer's apps do.
        frame: frame_file(launcher, sheet.is_some(), entrypoint, subject),
        // A phone's launcher is its Home Screen, under every app's window.
        root: launcher && cfg!(mobile),
        icon: match kind {
            Kind::App(from) => *from,
            Kind::Launcher(..) | Kind::Sheet(_) | Kind::Unopened(_) => None,
        },
        // The recovery key window is a sheet of the app's that asked, as the system attaches one.
        owner: sheet.and_then(|sheet| sheet.owner),
        // An app's window, and the Home Screen, shows its last frame as it opens again.
        snapshot: matches!(kind, Kind::App(_) | Kind::Launcher(..)).then(|| snapshot_file(entrypoint, subject)).flatten(),
        // Its title row is its page's: its app's icon and name (DESIGN.md, App windows).
        title_row: cfg!(desktop) && !launcher && sheet.is_none(),
    }
}

/// What a window of `kind` does as things happen to it (`Event`), over its page's channel `to`.
fn events(kind: &Kind, to: &UnboundedSender<Value>, entrypoint: &str, subject: Option<&str>, link: &str, window: &Arc<OnceLock<WindowId>>, started: Instant) -> impl FnMut(Event) + 'static {
    let launcher = matches!(kind, Kind::Launcher(..));
    // A launcher whose page fails as it starts has its problem box in its place, in a panel of its own.
    #[cfg(desktop)]
    let launcher_page = matches!(kind, Kind::Launcher(_, None));
    // An app's own window, which says why in a window of the client's own if its page doesn't start (`unopened`).
    let app_window = matches!(kind, Kind::App(_));
    let sheet = match kind {
        Kind::Sheet(sheet) => Some(sheet.clone()),
        _ => None,
    };
    let app = entrypoint.split('/').next().unwrap_or_default().to_owned();
    let (to, entrypoint, subject, link, shown) = (to.clone(), entrypoint.to_owned(), subject.map(str::to_owned), link.to_owned(), window.clone());
    let opening_view = format!("{}|{entrypoint}", subject.as_deref().unwrap_or_default());
    let mut opened = false;
    move |event: Event| match event {
            // How long it took to open, from being asked to, as a page tells the app's window it has drawn: once, as
            // a page loaded again in it (`reloaded`) says its own (`Opening`).
            Event::Drawn => {
                let ms = started.elapsed().as_millis() as u64;
                Opening::drawn(&opening_view, &app);
                crate::deployed(&app);
                if !std::mem::replace(&mut opened, true) {
                    let _ = to.send(json!({"id": "opened", "command": "opened", "args": {"view": app, "ms": ms}}));
                }
                // A computer's launcher is drawn put away, and is on screen once it's summoned (`Shown`).
                if !(launcher && cfg!(desktop)) {
                    let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": true}}));
                }
                // The device hears its person's notifications once their home is on screen, as the web
                // launcher's page has it (`listen`).
                if launcher {
                    let _ = to.send(json!({"id": "listen", "command": "listen", "args": {}}));
                }
                crate::memory::changed();
            }
            // How long the launcher took to show as it was summoned, to its first frame.
            Event::Shown(took) => {
                let ms = took.as_secs_f64() * 1000.0;
                let load = Load::now();
                eprintln!("the launcher showed {ms:.1} ms after it was summoned, {load}");
                if let Ok(views) = crate::views() {
                    views.client.report(json!({"name": "one.client.native.summon.duration", "value": ms, "attributes": {"one.view": app, "one.system.busy": load.busy()}}));
                }
                let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": true}}));
                crate::memory::changed();
            }
            Event::Live => Opening::live(&opening_view),
            Event::Frames(frames) => frames_reported(&app, &frames),
            Event::PutAway => {
                let _ = to.send(json!({"id": "visible", "command": "visible", "args": {"onScreen": false}}));
                crate::memory::changed();
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
                crate::memory::changed();
                if let Some(told) = sheet.as_ref().and_then(|sheet| sheet.done.lock().expect("nothing panics holding it").take()) {
                    let _ = told.send(false);
                }
            }
            // A page that crashed after it started is gone, and Telemetry hears why, as an error: an app's window
            // closes and its app is forgotten, so it opens afresh, and the client's own page says it stopped, with
            // Reload, as a browser's crashed tab does. So does the launcher's, in its place: a computer's panel, and a
            // phone's page. A page of the client's own in the launcher's place opens again.
            // One still blank a while after it opened tells Telemetry, as nothing else would.
            Event::Blank(why) => opening::blank(&opening_view, &app, &why),
            Event::Crashed(why) => {
                eprintln!("{entrypoint} crashed: {why}");
                if let Ok(views) = crate::views() {
                    views.client.report(json!({"name": "one.client.native.crash", "error": why, "attributes": {"one.view": app}}));
                }
                #[cfg(desktop)]
                if launcher_page {
                    return replace_launcher(entrypoint.clone(), why, true, shown.get().copied());
                }
                #[cfg(desktop)]
                if launcher {
                    return computer::reload_native();
                }
                #[cfg(mobile)]
                if launcher {
                    return computer::unopened(why, true);
                }
                close(&shown);
                crate::hosts::forget(&entrypoint, subject.as_deref());
                self::unopened(entrypoint.clone(), subject.clone(), link.clone(), why, true);
            }
            // A window whose page didn't start says so, and Telemetry hears why: an app's in the client's own
            // window with its problem box in its place (`unopened`), and a phone's launcher is then its page's.
            Event::Failed(why) => {
                reported(&entrypoint, started, &why);
                #[cfg(mobile)]
                if launcher {
                    computer::unopened(why.clone(), false);
                }
                #[cfg(desktop)]
                if launcher_page {
                    replace_launcher(entrypoint.clone(), why, false, shown.get().copied());
                    return;
                }
                if app_window {
                    close(&shown);
                    crate::hosts::forget(&entrypoint, subject.as_deref());
                    self::unopened(entrypoint.clone(), subject.clone(), link.clone(), why, false);
                }
            }
            // A computer's window says why it didn't open, as one that didn't start does, and Telemetry hears; a
            // phone opens it in its page.
            Event::Mismatched(why) => {
                reported(&entrypoint, started, &why);
                #[cfg(desktop)]
                if launcher_page {
                    replace_launcher(entrypoint.clone(), why, false, shown.get().copied());
                    return;
                }
                #[cfg(desktop)]
                if app_window {
                    close(&shown);
                    crate::hosts::forget(&entrypoint, subject.as_deref());
                    self::unopened(entrypoint.clone(), subject.clone(), link.clone(), why, false);
                }
                #[cfg(mobile)]
                if launcher {
                    computer::unopened(why, false);
                } else {
                    close(&shown);
                    let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
                    if let Err(error) = crate::hosts::open_web_view(entrypoint.clone(), subject, link) {
                        eprintln!("{entrypoint}: {error}");
                    }
                }
            }
    }
}

/// Makes window `window` with `options` on the windows' thread, telling `on_event` what happens to it, and `told` its
/// id, a launcher's. → whether it opened, as the windows' thread says: a job that's dropped, as a phone's is where its
/// windows couldn't start, says why there.
fn open_window(view: &str, options: WindowOptions, on_event: impl FnMut(Event) + 'static + Send, window: &Arc<OnceLock<WindowId>>, told: Option<tokio::sync::oneshot::Sender<Option<WindowId>>>) -> anyhow::Result<tokio::sync::oneshot::Receiver<Result<(), String>>> {
    let (opened_to, opened) = tokio::sync::oneshot::channel();
    let (view, window) = (view.to_owned(), window.clone());
    Native::get()?.run(move |windows| {
        Opening::step(&view, "on the windows' thread");
        let made = windows.opening(options, on_event);
        if made.is_ok() {
            Opening::step(&view, "window made");
        }
        let _ = opened_to.send(made.map(|id| { let _ = window.set(id); }).map_err(|error| format!("{error:#}")));
        if let Some(told) = told {
            let _ = told.send(window.get().copied());
        }
    });
    Ok(opened)
}

/// Starts `page`, its `module` and `tables`, over its window's channel `to`: its tables loaded, its JS on a thread of
/// its own, and where its device is with its network, as a page asks as it opens. → it, its page's own tokens
/// (`load`), and the Colour Mode chosen on its device, if one was.
#[allow(clippy::too_many_arguments)]
async fn start_page(view: &str, entrypoint: &str, subject: Option<&str>, module: &Url, page: Url, tables: &[Table], to: &UnboundedSender<Value>, asked: &Arc<Mutex<HashMap<String, Asked>>>) -> anyhow::Result<(Started, u32, Option<String>)> {
    let tokens_page = load(tables)?;
    Opening::step(view, "tables loaded");
    let begun = self::started(entrypoint, subject, to.clone(), asked.clone(), module, page);
    Opening::step(view, "JS thread started");
    let (start_to, start) = tokio::sync::oneshot::channel();
    asked.lock().expect("nothing panics holding it").insert("start".into(), Asked::Start(start_to));
    let _ = to.send(json!({"id": "start", "command": "start", "args": {"entrypoint": entrypoint}}));
    let start = start.await?;
    Opening::step(view, "started");
    Ok((begun, tokens_page, start["theme"].as_str().map(str::to_owned)))
}

/// Window `window` shows its page, `screen`'s, once the windows' thread has opened it: or closes, where its screen
/// couldn't be made. → why, if it couldn't.
fn attach(window: &Arc<OnceLock<WindowId>>, screen: impl FnOnce() -> anyhow::Result<Screen> + Send + 'static) -> anyhow::Result<tokio::sync::oneshot::Receiver<Result<(), String>>> {
    let (attached_to, attached) = tokio::sync::oneshot::channel();
    let window = window.clone();
    Native::get()?.run(move |windows| {
        let Some(id) = window.get().copied() else { return };
        let failed = match screen() {
            Ok(screen) => {
                windows.attach(id, screen);
                None
            }
            Err(error) => {
                windows.close(id);
                Some(format!("{error:#}"))
            }
        };
        let _ = attached_to.send(failed.map_or(Ok(()), Err));
    });
    Ok(attached)
}

/// Loads the design tokens and styles, which come with the UI library, as its pages' do, and a page's app's styles,
/// those not loaded already (`LOADED`). → the number of its page's own tokens, where its styles have them, which are
/// its screen's alone (`tokens::load_page`).
fn load(tables: &[Table]) -> anyhow::Result<u32> {
    let mut loaded = LOADED.lock().expect("nothing panics holding it");
    let mut page = 0;
    for (at, Table { file, text }) in tables.iter().enumerate() {
        match (at, text) {
            (0, Some(tokens)) => {
                renderer::tokens::load_table(tokens)?;
                loaded.0 = Some(file.clone());
            }
            (_, Some(styles)) => {
                renderer::load_styles(styles)?;
                let own = if at == 2 { renderer::tokens::load_page(styles)? } else { 0 };
                loaded.1.push((file.clone(), own));
            }
            (_, None) => {}
        }
        if at == 2 {
            page = loaded.1.iter().find(|(known, _)| known == file).map_or(0, |(_, own)| *own);
        }
    }
    Ok(page)
}

/// What makes the pages a window's frames show, each another app's page in a sandbox of its own (`frame`): only a
/// trusted app's page shows one, as on the DOM (`one/frame` in page/bridge.js).
fn frames(window: Arc<OnceLock<WindowId>>) -> renderer::MakeFrame {
    Box::new(move |node, src, subject| {
        let (frames, src, subject) = (window.clone(), src.to_owned(), subject.map(str::to_owned));
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
    })
}

/// An app's window loads its page again in place, on its app's new version, as a browser's tab reloads (`stale`): the
/// new page starts over the window's channel, `to`, beside the one it shows, and takes its place in the same window
/// (`Windows::reload`), whose frame stays on screen until the new page has started. What the page before asked that's
/// still unanswered is forgotten, so nothing meant for it reaches the new one. → whether its window was open.
async fn reloaded(window: Arc<OnceLock<WindowId>>, entrypoint: String, subject: Option<String>, link: String, to: UnboundedSender<Value>, asked: Arc<Mutex<HashMap<String, Asked>>>) -> anyhow::Result<bool> {
    let view = format!("{}|{entrypoint}", subject.as_deref().unwrap_or_default());
    Opening::begin(&view, Instant::now());
    let Some(Page { page, module, trusted, tables, .. }) = probe(&entrypoint, subject.as_deref()).await? else {
        OPENING.lock().expect("nothing panics holding it").remove(&view);
        anyhow::bail!("its new version is no page this host draws")
    };
    let tokens_page = load(&tables)?;
    let begun = started(&entrypoint, subject.as_deref(), to.clone(), asked.clone(), &module, page);
    let number = begun.number;
    let (start_to, start) = tokio::sync::oneshot::channel();
    asked.lock().expect("nothing panics holding it").insert("start".into(), Asked::Start(start_to));
    let _ = to.send(json!({"id": "start", "command": "start", "args": {"entrypoint": entrypoint}}));
    let start = start.await?;
    Opening::step(&view, "started");
    let chosen = start["theme"].as_str().map(str::to_owned);
    let fragment = link.split_once('#').map(|(_, fragment)| fragment.to_owned());
    let state = State { chosen: chosen.clone(), trusted, entrypoint: entrypoint.clone(), subject: subject.clone(), view, page: 0, next: 0, launcher: false, framed: false, fragment, sheet: None, unopened: None, signing_in: false, behind: false, window: window.clone() };
    let (reloaded_to, reloaded) = tokio::sync::oneshot::channel::<anyhow::Result<bool>>();
    Native::get()?.run(move |windows| {
        let Some(id) = window.get().copied() else { return drop(reloaded_to.send(Ok(false))) };
        let screen = begun.screen(state, Default::default()).map(|mut screen| {
            screen.theme = theme(chosen.as_deref());
            screen.page = tokens_page;
            screen.make_frame = trusted.then(|| frames(window.clone()));
            screen
        });
        let outcome = screen.map(|screen| {
            // What the page before asked is no one's now: its answers go nowhere.
            let ours = |key: &str| key == "start" || key.get(1..).and_then(|rest| rest.split('.').next()) == Some(number.to_string().as_str());
            asked.lock().expect("nothing panics holding it").retain(|key, _| ours(key));
            windows.reload(id, screen)
        });
        let _ = reloaded_to.send(outcome);
    });
    reloaded.await?
}

/// Starts the page that frame `node` of `window`'s page shows, `src` (`<app>/<page>[#<fragment>]`), for `subject`, and
/// hands it to that page's screen: its own app's, in a sandbox of its own, calling with that app's permissions, as its
/// own window's page would, and drawn in the frame's box.
async fn frame(window: Arc<OnceLock<WindowId>>, node: renderer::tree::Id, src: &str, subject: Option<&str>) -> anyhow::Result<()> {
    let entrypoint = src.split_once('#').map_or(src, |(page, _)| page);
    let Some(Page { page, module, trusted, tables, .. }) = probe(entrypoint, subject).await? else {
        anyhow::bail!("it's no page of an app's this host draws")
    };
    let tokens_page = load(&tables)?;
    let started = start(entrypoint, subject, src, false, Place::Frame(window.clone(), node), &module, page)?;
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    let fragment = src.split_once('#').map(|(_, fragment)| fragment.to_owned());
    let state = State { chosen: None, trusted, entrypoint: entrypoint.to_owned(), subject: subject.map(str::to_owned), view, page: 0, next: 0, launcher: false, framed: true, fragment, sheet: None, unopened: None, signing_in: false, behind: false, window: window.clone() };
    let src = src.to_owned();
    Native::get()?.run(move |windows| {
        let Some(id) = window.get() else { return };
        windows.with(*id, |parent| match started.screen(state, Default::default()) {
            Ok(mut screen) => {
                screen.page = tokens_page;
                parent.framed(node, &src, screen)
            }
            Err(error) => parent.mismatched = Some(format!("its frame's page, {src}, didn't start: {error:#}")),
        });
    });
    Ok(())
}

/// The row at a window's top that its title is in, with its page's top bar after it (a unified toolbar), 52
/// points, as macOS's System Settings and Raycast have it, and where the title starts in it: on a Mac 8 points
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

    /// Behind its servers, its window closes, and the page says Update Required.
    pub fn behind() {
        close();
        crate::app().shell.emit("launcher", serde_json::json!({"native": false, "behind": true}));
    }

    /// The launcher's page didn't open after all, as `why` says, or `crashed` once it had: its window closes, and the
    /// page says so in its place, with Try Again, or Reload where it crashed.
    pub fn unopened(why: String, crashed: bool) {
        close();
        crate::app().shell.emit("launcher", serde_json::json!({"native": false, "why": why, "crashed": crashed}));
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

/// The page `app` opens first, for `subject`, as its apps are listed (`launcherIn` in apps.js): the last one listed,
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
