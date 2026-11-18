//! An app's window drawn natively, where its entrypoint's page is in strict mode and needs a runtime version this
//! program has (`runtimeVersion`, core/ui/src/strict/renderer.ts's `RUNTIME_VERSION`, `strict::RUNTIME_VERSION`):
//! this process, the One App's, draws it on its strict thread (`strict::platform`) beside every other strict app's window, so another open
//! app costs a few MB, and its JS runs in a QuickJS sandbox of its own, on a thread of its own. Any other app's window is a web view in a
//! process of its own, as before (`host`), and so is a strict one that needs a later runtime version.
//!
//! It asks what a page would, over the same channel a web view's window does (`hosts`), here an in-memory one: its
//! files, and what the page asks of its client, which an MCP App asks its host (JSON-RPC). What a web view's window
//! page answers itself (`respond` in page/client.js) is answered here: `ui/initialize`, `ping` and `one/client`'s
//! Colour Mode; the rest is the client's, as from the app (`request`).
//!
//! The app's client offers the network the inspector's protocol (`strict::inspect`) for this device's strict windows,
//! as the Inspector app's tools (`Inspector`), so an agent anywhere in the network reads and drives a strict window as
//! it would one of its own.
//!
//! On Android the same window is a view over the page, on the UI thread, and any other app is a frame in the page
//! (`hosts::open_window`); what a computer's launcher does for it, a phone does as its own (`computer`, below).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Instant;

use data_encoding::BASE64;
use serde_json::{Value, json};
use strict::geometry::{Pixels, Size};
use strict::js::{Clock, Files, Thread};
use strict::platform::{Event, Strict, WindowId, WindowOptions};
use strict::{Appearance, Screen};
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

/// Opens `entrypoint`'s window here if its page is strict and of a version this program speaks, at `link`, for
/// `subject`. → whether it did; if not, it opens in a web view.
pub(crate) async fn open(entrypoint: &str, subject: Option<&str>, link: &str) -> bool {
    let started = Instant::now();
    match window(started, entrypoint, subject, link, Kind::App).await {
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

/// What a strict page is drawn with: the module it loads, whether its app is trusted, and the tables its styles
/// come from: the UI library's tokens and styles and its app's styles (`Tokens` and `Styles` in core/ui/src/strict/renderer.ts).
struct Page {
    /// Its page's address, which what it names by a relative address is from, and the module it loads.
    page: Url,
    module: Url,
    trusted: bool,
    /// The UI library's design tokens (`ui.css`), and its tables.
    css: String,
    tables: Vec<String>,
    /// Its name and icon, as a `data:` address, as its app lists them, for the window's title and breadcrumb.
    name: Option<String>,
    icon: Option<String>,
}

/// Whether `entrypoint`'s page is strict and needs a runtime version this program has, its tables too, and if it is, what
/// it's drawn with.
async fn probe(entrypoint: &str, subject: Option<&str>, framed: bool) -> anyhow::Result<Option<Page>> {
    let listed = crate::hosts::command("strict", "apps", json!({"subject": subject})).await.map_err(|error| anyhow::anyhow!(error))?;
    let (app, path) = entrypoint.split_once('/').unwrap_or((entrypoint, ""));
    let Some(resource) = listed["resources"].as_array().into_iter().flatten().find(|resource| {
        let uri = resource["uri"].as_str().unwrap_or_default();
        uri.strip_prefix(&format!("ui://{app}/")).and_then(|rest| rest.split_once('/')).is_some_and(|(_, page)| page == path)
    }) else {
        return Ok(None);
    };
    // A page in a frame that names no runtime version (a component's, which no entrypoint names) is tried: one that
    // isn't strict doesn't start, and the page the frame's in opens in a web view instead.
    let needed = resource["_meta"]["runtimeVersion"].as_u64();
    if !needed.map_or(framed, |needed| needed <= u64::from(strict::RUNTIME_VERSION)) {
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
    let css = crate::hosts::file(page.join("/ui/client/ui.css")?.as_str()).await.map_err(|error| anyhow::anyhow!("ui.css didn't load: {error}"))?;
    let css = String::from_utf8(BASE64.decode(css["body"].as_str().unwrap_or_default().as_bytes())?)?;
    // Its tables, each needing a runtime version this program has, or it's a web view's. An app with no styles of its own has
    // no table of them.
    let mut tables = Vec::new();
    for (table, needed) in [(page.join("/ui/client/tokens.json")?, true), (page.join("/ui/client/styles.json")?, true), (page.join("styles.json")?, false)] {
        let file = match crate::hosts::file(table.as_str()).await {
            Ok(file) => BASE64.decode(file["body"].as_str().unwrap_or_default().as_bytes())?,
            Err(_) if !needed => continue,
            Err(error) => anyhow::bail!("{table} didn't load: {error}"),
        };
        let text = String::from_utf8(file)?;
        let v = serde_json::from_str::<Value>(&text).map(|table| table["v"].as_u64().unwrap_or(0)).unwrap_or(0);
        if v > u64::from(strict::RUNTIME_VERSION) {
            return Ok(None);
        }
        tables.push(text);
    }
    let name = resource["name"].as_str().filter(|name| !name.is_empty()).map(str::to_owned);
    Ok(Some(Page { module: page.join(&module)?, page, trusted, css, tables, name, icon }))
}

/// Opens the launcher's page natively, if it's strict and of a version this program speaks, in a panel put away until
/// it's summoned (`computer`). → its window, once it's open, or none for its page to be the web launcher's.
#[cfg(desktop)]
pub(crate) async fn launcher(entrypoint: &str) -> Option<WindowId> {
    let (opened_to, opened) = tokio::sync::oneshot::channel();
    match window(Instant::now(), entrypoint, None, entrypoint, Kind::Launcher(Some(opened_to))).await {
        Ok(true) => opened.await.ok().flatten(),
        Ok(false) => None,
        Err(error) => {
            eprintln!("the launcher is a web view's: {error:#}");
            None
        }
    }
}

/// What a strict window is: an app's own, or the launcher, a panel the system summons, which tells whoever opens it
/// its window once it's open, or that it didn't.
enum Kind {
    App,
    #[cfg_attr(mobile, allow(dead_code))]
    Launcher(Option<tokio::sync::oneshot::Sender<Option<WindowId>>>),
}

async fn window(started: Instant, entrypoint: &str, subject: Option<&str>, link: &str, mut kind: Kind) -> anyhow::Result<bool> {
    let Some(Page { page, module, trusted, css, tables, name: listed, icon }) = probe(entrypoint, subject, false).await? else { return Ok(false) };
    let launcher = matches!(kind, Kind::Launcher(_));
    let tokens_page = load(&css, &tables)?;
    let strict = Strict::get()?;
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
    let state = State { chosen: chosen.clone(), trusted, entrypoint: entrypoint.to_owned(), view, next: 0, launcher, framed: false, fragment };
    let (entrypoint, subject, link) = (entrypoint.to_owned(), subject.map(str::to_owned), link.to_owned());
    strict.run(move |windows| {
        let (size, panel) = match launcher {
            true => ((computer::SIZE.width as f32, computer::SIZE.height as f32), Some(computer::CORNER)),
            false => ((1024.0, 720.0), None),
        };
        let options = WindowOptions {
            title: if launcher { NAME.to_owned() } else { name.clone() },
            app_id: ::client::NAME.to_owned(),
            size: Size { width: Pixels(size.0), height: Pixels(size.1) },
            tag: json!({"entrypoint": entrypoint, "subject": subject}),
            panel: panel.map(|corner| corner as f32),
        };
        let app = entrypoint.split('/').next().unwrap_or_default().to_owned();
        let on_event = {
            let (to, app, entrypoint, subject, link) = (to.clone(), app.clone(), entrypoint.clone(), subject.clone(), link.clone());
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
                Event::Closed => {}
                // The launcher's is only its page's: one that didn't start is the web launcher's.
                Event::Failed(why) if launcher => {
                    eprintln!("the launcher is a web view's: its page didn't start: {why}");
                    computer::native(None);
                }
                Event::Failed(why) => eprintln!("{entrypoint} didn't start: {why}"),
                // The launcher whose page turns out to need what this host can't draw is the web launcher's.
                Event::Mismatched(why) if launcher => {
                    eprintln!("the launcher is a web view's: {why}");
                    computer::native(None);
                }
                Event::Mismatched(why) => {
                    eprintln!("{entrypoint} opens in a web view: {why}");
                    let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
                    if let Err(error) = crate::hosts::open_web_view(entrypoint.clone(), subject, link) {
                        eprintln!("{entrypoint}: {error}");
                    }
                }
            }
        };
        // Its title bar, which its page keeps clear, as its host context's `safeAreaInsets` tells it. The launcher has
        // none.
        let top = if launcher { 0.0 } else { BAR.0 as f32 };
        let insets = strict::geometry::Edges { top, right: 0.0, bottom: 0.0, left: 0.0 };
        let opened = match begun.screen(state, Size { width: options.size.width.0, height: options.size.height.0 }, insets) {
            Ok(mut screen) => {
                screen.theme = theme(chosen.as_deref());
                screen.page = tokens_page;
                // What its frames show, each another app's page in a sandbox of its own (`frame`): only a trusted
                // app's page shows one, as on the DOM (`one/frame` in page/client.js).
                let frames = window.clone();
                screen.make_frame = trusted.then(|| -> strict::MakeFrame { Box::new(move |node, src, subject| {
                    let (frames, src, subject) = (frames.clone(), src.to_owned(), subject.map(str::to_owned));
                    crate::spawn(async move {
                        if let Err(error) = frame(frames.clone(), node, &src, subject.as_deref()).await {
                            eprintln!("{src} isn't drawn in its frame: {error:#}");
                            // The page it's in can't be drawn here without it: it opens in a web view instead.
                            if let Ok(strict) = Strict::get() {
                                strict.run(move |windows| {
                                    if let Some(id) = frames.get() {
                                        windows.with(*id, |screen| screen.mismatched = Some(format!("its frame's page, {src}, isn't drawn natively: {error:#}")));
                                    }
                                })
                            }
                        }
                    });
                }) });
                if launcher {
                    // Its glass is the system's, under its page, where there's one; Linux has none, so it's a
                    // popover's colour (DESIGN.md, Launcher).
                    screen.panel = Some(strict::Panel { corner: computer::CORNER as f32, fill: cfg!(target_os = "linux") });
                } else if cfg!(desktop) {
                    // Its way Home, as an app's window on a computer has (DESIGN.md, App windows): Home brings the
                    // launcher forward and keeps this window open, as Window → Home Screen does.
                    let ring = format!("data:image/png;base64,{}", BASE64.encode(include_bytes!("../../../icons/Logo.png")));
                    screen.breadcrumb = Some(strict::Breadcrumb { ring, icon, name: name.clone(), start: BAR.1 as f32, home: Box::new(computer::summon) });
                }
                windows.open(options, screen, on_event)
            }
            Err(error) => Err(error),
        };
        match opened {
            Ok(id) => drop(window.set(id)),
            Err(error) => eprintln!("{entrypoint}'s window didn't open: {error:#}"),
        }
        if let Kind::Launcher(told) = &mut kind
            && let Some(told) = told.take()
        {
            let _ = told.send(window.get().copied());
        }
    });
    Ok(true)
}

/// Loads the design tokens and styles, which come with the UI library, as its pages' do, and a page's app's styles.
/// → the number of its page's own tokens, where its styles have them, which are its screen's alone
/// (`tokens::load_page`).
fn load(css: &str, tables: &[String]) -> anyhow::Result<u32> {
    strict::tokens::load(css);
    if let [tokens, styles @ ..] = tables {
        strict::tokens::load_table(tokens)?;
        for styles in styles {
            strict::load_styles(styles)?;
        }
    }
    Ok(match tables {
        [_, _, own] => strict::tokens::load_page(own)?,
        _ => 0,
    })
}

/// Where a page is drawn: in its own window, or in a frame, by its node, of a window's page.
#[derive(Clone)]
enum Place {
    Window(Arc<OnceLock<WindowId>>),
    Frame(Arc<OnceLock<WindowId>>, strict::tree::Id),
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
    let strict = Strict::get()?;
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let to = channel(entrypoint, subject, link, launcher, asked.clone(), strict, place)?;
    let cache = crate::app().shell.home()?.join("strict").join(::client::BUILD);
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
    let js = Thread::start(module, files, Some(cache), Clock::real());
    Ok(Started { js, to, asked, page })
}

impl Started {
    /// Its screen, on the strict thread: what its page asks answered as `state` says, and a picture it names by an
    /// address fetched as one of its app's files, as a web view's page loads it.
    fn screen(self, state: State, room: Size<f32>, insets: strict::geometry::Edges<f32>) -> anyhow::Result<Screen> {
        let Started { js, to, asked, page } = self;
        let state = Arc::new(Mutex::new(state));
        let (respond_to, respond_asked) = (to.clone(), asked.clone());
        let asked_by_app: strict::Asked = Box::new(move |message, appearance| respond(message, appearance, &state, &respond_to, &respond_asked));
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
async fn frame(window: Arc<OnceLock<WindowId>>, node: strict::tree::Id, src: &str, subject: Option<&str>) -> anyhow::Result<()> {
    let entrypoint = src.split_once('#').map_or(src, |(page, _)| page);
    let Some(Page { page, module, trusted, css, tables, .. }) = probe(entrypoint, subject, true).await? else {
        anyhow::bail!("it's no page of an app's this host draws")
    };
    let tokens_page = load(&css, &tables)?;
    let started = start(entrypoint, subject, src, false, Place::Frame(window.clone(), node), &module, page)?;
    let view = format!("{}|{entrypoint}", subject.unwrap_or_default());
    let fragment = src.split_once('#').map(|(_, fragment)| fragment.to_owned());
    let state = State { chosen: None, trusted, entrypoint: entrypoint.to_owned(), view, next: 0, launcher: false, framed: true, fragment };
    let src = src.to_owned();
    Strict::get()?.run(move |windows| {
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

/// The row at a window's top that its breadcrumb is in, with its page's top bar after it (a unified toolbar), and where
/// the breadcrumb starts in it: a Mac's title bar, after its traffic lights over the page (`title_bar`); elsewhere the
/// same height, under the window's own frame.
#[cfg(target_os = "macos")]
const BAR: (f64, f64) = (crate::title_bar::MAC.height, crate::title_bar::MAC.start);
#[cfg(all(desktop, not(target_os = "macos")))]
const BAR: (f64, f64) = (crate::title_bar::HEIGHT, 12.0);
/// A phone's window has none: the activity keeps the page clear of the system's bars (`MainActivity`).
#[cfg(mobile)]
const BAR: (f64, f64) = (0.0, 0.0);

/// What a strict window asks of a computer's launcher (`crate::computer`), as a phone has it: its Home Screen is under
/// every app's window, so Home closes them; a `one:` link opens in its page; and it has no launcher panel to size, put
/// away or reload, as its launcher is its page's.
#[cfg(mobile)]
mod computer {
    pub struct Size {
        pub width: f64,
        pub height: f64,
    }
    pub const SIZE: Size = Size { width: 0.0, height: 0.0 };
    pub const CORNER: f64 = 0.0;
    pub const NAME: &str = "";

    pub fn summon() {
        crate::hosts::close_all();
    }

    pub fn again(link: Option<String>) {
        if let Some(opens) = link.and_then(|link| crate::link(&link)) {
            crate::app().shell.emit("open", serde_json::json!(opens));
        }
    }

    pub fn hide(_: bool) {}
    pub fn resize(_: Option<f64>, _: Option<f64>) {}
    pub fn native(_: Option<strict::platform::WindowId>) {}
    pub fn reload_native() {}
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
                // A page in a frame is named by nothing above it: its first screen says its heading.
                "breadcrumb": cfg!(desktop) && !state.framed,
                // The place its link names, as its address's `#` says on the DOM (`link()`).
                "link": state.fragment,
                "connection": "online",
                "updateReady": false,
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
                // The launcher's page lays itself out as a computer's launcher (`client().launcher` in home.tsx).
                // `transparent`: the launcher's panel is see-through where its page draws nothing, so what its page draws
                // floats over the desktop, on the system's glass where it asks for it (a Mac's).
                Some(Ok(json!({"theme": state.chosen, "launcher": state.launcher, "mac": cfg!(target_os = "macos"), "transparent": state.launcher && cfg!(target_os = "macos")})))
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
    let listed = crate::hosts::command("strict", "apps", json!({"subject": subject})).await.ok()?;
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
/// socket, saying hello as it does: what it hears is done to the window on the strict thread. → where to send it a
/// message.
fn channel(
    entrypoint: &str,
    subject: Option<&str>,
    link: &str,
    launcher: bool,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    strict: Strict,
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
            let (strict, window) = (strict.clone(), window.clone());
            let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
            move || {
                // The launcher's main thread closes it and opens it again, shown again if it was (`Told::Reload`).
                if launcher {
                    return computer::reload_native();
                }
                // A frame's page starts again in it.
                if let Some(node) = framed {
                    let window = window.clone();
                    return strict.run(move |windows| {
                        if let Some(id) = window.get() {
                            windows.with(*id, |screen| screen.reload_frame(node));
                        }
                    });
                }
                let (entrypoint, subject, link) = (entrypoint.clone(), subject.clone(), link.clone());
                let window = window.clone();
                strict.run(move |windows| {
                    if let Some(id) = window.get() {
                        windows.close(*id);
                    }
                    if let Err(error) = crate::hosts::reopen(entrypoint, subject, link) {
                        eprintln!("{error}");
                    }
                })
            }
        };
        // The window this is, once the strict thread has opened it, a job running after the one that opened it; or the
        // page in its frame, once it's there.
        let on = |job: Box<dyn FnOnce(&mut Screen) + Send>| {
            let window = window.clone();
            strict.run(move |windows| {
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
                    // Over its page, while its network doesn't answer, as over an app on a phone (DESIGN.md).
                    "connection" if framed.is_none() => on(Box::new(move |screen| {
                        screen.notice = match payload.as_str() {
                            Some("reconnecting") => Some(strict::Notice { text: "Reconnecting…".into(), action: None }),
                            Some("offline") => Some(strict::Notice { text: "Can't reach your network".into(), action: None }),
                            _ => screen.notice.take().filter(|notice| notice.action.is_some()),
                        }
                    })),
                    // Its page, or the UI library it runs, was loaded against a version that's gone: "Updated", with
                    // Reload, as over an app on a phone.
                    "stale" if payload.as_array().is_some_and(|stale| stale.iter().filter_map(Value::as_str).any(|host| host.split('.').next() == Some("ui") || host == app_host)) => {
                        let reload = reload.clone();
                        on(Box::new(move |screen| screen.notice = Some(strict::Notice { text: "Updated".into(), action: Some(("Reload".into(), Box::new(reload))) })))
                    }
                    // Behind its servers, the launcher is the web launcher's, which says Update Required.
                    "behind" if launcher => computer::native(None),
                    // The launcher, linked to, comes forward.
                    "open" if launcher => computer::summon(),
                    // Its window comes forward, and its page is told the place the link names, as a page's `hashchange`.
                    "open" if framed.is_none() => {
                        let window = window.clone();
                        let fragment = payload.as_str().and_then(|link| link.split_once('#')).map(|(_, fragment)| fragment.to_owned());
                        strict.run(move |windows| {
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

/// Whether a strict window holds something its person typed that its app hasn't taken (`Windows::typed`).
pub async fn typed() -> bool {
    let Some(strict) = Strict::started() else { return false };
    let (told, answer) = tokio::sync::oneshot::channel();
    strict.run(move |windows| {
        let _ = told.send(windows.typed());
    });
    answer.await.unwrap_or(false)
}
