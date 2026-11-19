//! The app on a computer (a Mac, Windows or Linux): a launcher, as Spotlight and Raycast are. Its window is the Home
//! Screen in a panel the system summons over whatever's in front, near the top of the screen the pointer is on, which
//! goes again once an app opens from it, on Escape, or once something else is clicked. Every app opens in a window of
//! its own (`hosts`, `host`), so starting them is all the launcher does. It's summoned by its shortcut (`shortcut`), the
//! app's own icon in the Dock or taskbar (a second start, `hosts::again`), a Mac's menu bar, where the ring is, and an
//! app window's Home. The program runs on with it put away, holding the device's one connection to its network, its
//! notifications and its app windows, until it's quit (its menu, Cmd-Q, or Ctrl-Q on Linux).
//!
//! Where the Home Screen's page is of a version this program speaks, the launcher is drawn natively, in a
//! panel of this process's own (`renderer::launcher`), as every app's native window is, and it shows within a frame of
//! being summoned. Otherwise, and while the device signs in or needs updating, which its page doesn't show, it's a
//! window and a web view (tao and wry): its page is the window's own (`page/`), which shows the Home Screen, and asks
//! what it asks of the app (`hosts::command`) through its bridge (`host-initialization-script.js`). Put away a while,
//! it discards its page (`discard`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Theme, Window, WindowBuilder};
use wry::http::Request;
use wry::{RequestAsyncResponder, WebView, WebViewBuilder, WebViewId};

use crate::discard::Discarding;
use crate::{hosts, notifications};

/// The launcher's width and tallest height, in points: Raycast's panel's width, its search bar, two of a phone's pages
/// side by side with their six rows, near a phone's size (`measure` in the Home Screen's layout.ts), and the room its
/// footer's pills float in. It's as tall as what it shows, which its page says (`resize`).
pub(crate) const SIZE: LogicalSize<f64> = LogicalSize::new(750.0, 730.0);
/// The most a natively drawn launcher's page may ask its panel to be, in points: on a Mac a see-through panel that
/// holds what floats over the desktop, its search and its card, with room for their shadows.
const MOST: LogicalSize<f64> = LogicalSize::new(1200.0, 900.0);
/// How round the launcher's corners are, in points, as Raycast's are, and the edge its page draws inside them.
pub(crate) const CORNER: f64 = 18.0;

/// What the program is told to do on its main thread, where its window and web view are.
pub(crate) enum Told {
    /// Runs script in the launcher's page.
    Script(String),
    /// Shows the launcher, or puts it away if it's showing: its shortcut, or the ring in a Mac's menu bar, when it
    /// was pressed, and the key the shortcut ends with, as the DOM names it.
    Toggle(std::time::Instant, Option<String>),
    Summon,
    /// The launcher is drawn natively in this window (`renderer::launcher`), or with none, it's the web launcher.
    Native(Option<renderer::platform::WindowId>),
    /// Puts it away, and with `true` gives the keyboard back to the app that had it (Escape, or its page done).
    Hide(bool),
    Theme(Option<crate::Theme>),
    /// Makes it as tall as its page's content, or its tallest, and as wide where it says (`resize`).
    Resize(Option<f64>, Option<f64>),
    /// Loads its page again, as the device's session starts again.
    Reload,
    /// Its servers talk with it again after they said it was behind, which made it the web launcher (`native`): which
    /// launcher it is is decided again, the native one where it can be.
    Talks,
    /// Opens a `one:` link.
    Open(String),
    Restart,
    Quit,
}

static PROXY: OnceLock<Mutex<EventLoopProxy<Told>>> = OnceLock::new();

/// Tells the main thread `told`.
fn tell(told: Told) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.lock().expect("nothing panics holding it").send_event(told);
    }
}

/// Shows the launcher, as an app window's Home does.
pub(crate) fn summon() {
    tell(Told::Summon)
}

/// Quits the app, every window with it, as its menu's Quit does.
pub(crate) fn quit() {
    tell(Told::Quit)
}

/// Puts the launcher away, as an app opens from it, or with `give_back`, as it's dismissed.
pub(crate) fn hide(give_back: bool) {
    tell(Told::Hide(give_back))
}

/// Makes the launcher as tall as `height`, in points, or its tallest, as its page asks.
pub(crate) fn resize(width: Option<f64>, height: Option<f64>) {
    tell(Told::Resize(width, height))
}

/// The launcher is drawn natively in window `id`, or with none, it's the web launcher, as its page turns out to need.
pub(crate) fn native(id: Option<renderer::platform::WindowId>) {
    tell(Told::Native(id))
}

/// Why this device's native windows couldn't start, where they couldn't: the web launcher says so in place of the
/// Home Screen (`unnative` in its page's `start`).
static UNNATIVE: Mutex<Option<String>> = Mutex::new(None);

/// This device's native windows couldn't start, so there's no Home Screen: the web launcher says why.
pub(crate) fn unnative(why: String) {
    *UNNATIVE.lock().expect("nothing panics holding it") = Some(why);
    tell(Told::Native(None))
}

/// Why this device's native windows couldn't start, if they couldn't (`unnative`).
pub(crate) fn unnatived() -> Option<String> {
    UNNATIVE.lock().expect("nothing panics holding it").clone()
}

/// Its servers talk with it again: a launcher that's the web one for being behind them is native again (`Told::Talks`).
pub(crate) fn talks() {
    tell(Told::Talks)
}

/// The native launcher's page was updated: it opens again, on its new version.
pub(crate) fn reload_native() {
    tell(Told::Reload)
}

/// The Home Screen's page, as this user's apps list it (`<app>/<page>`), once the native launcher has looked.
static LAUNCHER: Mutex<Option<String>> = Mutex::new(None);

/// Whether `page` is the launcher's.
pub(crate) fn is_launcher(page: &str) -> bool {
    LAUNCHER.lock().expect("nothing panics holding it").as_deref() == Some(page)
}

/// Opens the launcher natively, where it can be, and tells the main thread which it is (`Told::Native`): where the
/// device is in its network,. Signing in, it's the web launcher's. The `one:` link the app was started with, or that
/// came before, opens once the native launcher has, or is the web launcher's to open.
async fn prepare() {
    let mut link = None;
    let opened = async {
        let started = crate::start().await.ok()?;
        link = started["open"].as_str().map(str::to_owned);
        if started["state"]["session"]["state"] == "signIn" {
            return None;
        }
        let listed = hosts::command("main", "apps", json!({})).await.ok()?;
        let (page, _) = crate::native::pages(&listed).find(|(_, meta)| meta["category"] == "launcher")?;
        *LAUNCHER.lock().expect("nothing panics holding it") = Some(page.clone());
        crate::native::launcher(&page).await
    };
    let opened = opened.await;
    match (link, opened.is_some()) {
        (Some(link), true) => crate::spawn(crate::native::go(link, None)),
        // The web launcher's page asks for it as it starts (`start`).
        (Some(link), false) => *OPENED.lock().expect("nothing panics holding it") = Some(link),
        (None, _) => {}
    }
    tell(Told::Native(opened))
}

/// The app was started again: its icon in the Dock or taskbar, which shows the launcher, or a `one:` link it opens.
pub(crate) fn again(link: Option<String>) {
    tell(link.and_then(|link| crate::link(&link)).map_or(Told::Summon, Told::Open))
}

/// Runs `job` on the native windows' thread, where the native launcher is.
fn on_native(job: impl FnOnce(&mut renderer::platform::Windows) + Send + 'static) {
    match renderer::platform::Native::get() {
        Ok(native) => native.run(job),
        Err(error) => eprintln!("no native window here: {error:#}"),
    }
}

/// The runtime the app's client and its channel run on.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime starts"))
}

/// A `one:` link for the page to open once it asks where it is: what the app was started with, or one that came
/// while its page was loading.
static OPENED: Mutex<Option<String>> = Mutex::new(None);

/// A computer's window, as what's here tells and asks it (`Shell`).
struct Computer;

impl crate::Shell for Computer {
    fn home(&self) -> anyhow::Result<std::path::PathBuf> {
        ::client::home()
    }

    fn emit(&self, event: &str, payload: Value) {
        if event == "open" {
            return tell(Told::Open(payload.as_str().unwrap_or_default().to_owned()));
        }
        tell(Told::Script(format!("window.__ONE_HOST__.event({}, {payload})", Value::from(event))));
        if hosts::EVENTS.contains(&event) {
            hosts::tell_all(json!({"event": event, "payload": payload}));
        }
    }

    fn reload(&self) {
        hosts::close_all();
        tell(Told::Reload);
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        open::that_detached(url).map_err(|error| error.to_string())
    }

    fn restart(&self) {
        tell(Told::Restart)
    }

    fn opened_at(&self) -> Option<String> {
        OPENED.lock().expect("nothing panics holding it").take()
    }

    /// A tap on one opens its app, as a `one:` link to it does.
    fn listen(&self, client: Arc<::client::Client>) {
        crate::spawn(notifications::listen(client, ::client::NAME, |app| crate::app().shell.emit("open", json!(app))));
    }

    fn theme(&self, theme: Option<crate::Theme>) {
        tell(Told::Theme(theme))
    }
}

/// Starts the app, unless it's running already, which it then tells it was started again (`hosts::again`), and runs
/// until it's quit.
pub fn run() {
    let started = std::time::Instant::now();
    // Linux and Windows open a `one:` link with this program, by giving it as the only argument; a Mac tells the
    // program running (`Event::Opened`).
    let link = std::env::args().nth(1).filter(|arg| arg.starts_with("one:"));
    let home = match ::client::home() {
        Ok(home) => home,
        Err(error) => return eprintln!("{error:#}"),
    };
    keep_log(&home);
    // Started again by itself as it updates (`Told::Restart`), it doesn't hand over to the one that's ending.
    if std::env::var_os("ONE_RESTARTED").is_none() && hosts::again(&home, link.as_deref()) {
        return;
    }
    if let Err(error) = launcher(home, link.and_then(|link| crate::link(&link)), started) {
        eprintln!("{error:#}");
    }
}

/// What it says (`eprintln!`) goes to its own log, `app.log` in its home, the one before as `app.log.1`, where nothing
/// it was started from keeps it: an app opened from the Dock, the Start menu or a desktop's launcher says it to nowhere,
/// so how one that stopped ended was never known (20 Nov). Started from a terminal or into a file, as a person or an
/// agent reading it starts it, it's left there. Started again as it updates, it carries on in the same log.
fn keep_log(home: &std::path::Path) {
    if !says_to_nowhere() {
        return;
    }
    let log = home.join("app.log");
    // Each start from nowhere begins a new one past 8 MB, as `server.log` is kept.
    if std::fs::metadata(&log).is_ok_and(|log| log.len() > 8 << 20) {
        let _ = std::fs::rename(&log, log.with_extension("log.1"));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) else { return };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // Standard error is the log's from now on, for this program and what it starts.
        unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        let handle = windows::Win32::Foundation::HANDLE(file.into_raw_handle());
        let _ = unsafe { windows::Win32::System::Console::SetStdHandle(windows::Win32::System::Console::STD_ERROR_HANDLE, handle) };
    }
}

/// Whether its standard error goes nowhere: to `/dev/null`, or on Windows, a windowed program's none.
fn says_to_nowhere() -> bool {
    #[cfg(unix)]
    {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let null = std::fs::metadata("/dev/null").map(|null| std::os::unix::fs::MetadataExt::rdev(&null)).ok();
        let unread = unsafe { libc::fstat(libc::STDERR_FILENO, &mut stat) } != 0;
        unread || Some(stat.st_rdev as u64) == null && (stat.st_mode & libc::S_IFMT) == libc::S_IFCHR
    }
    #[cfg(windows)]
    {
        unsafe { windows::Win32::System::Console::GetStdHandle(windows::Win32::System::Console::STD_ERROR_HANDLE) }.is_ok_and(|handle| handle.is_invalid() || handle.0.is_null())
    }
}

fn launcher(home: std::path::PathBuf, link: Option<String>, started: std::time::Instant) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    gtk::glib::set_prgname(Some(::client::NAME));
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut event_loop = EventLoopBuilder::<Told>::with_user_event().build();
    #[cfg(target_os = "linux")]
    gtk::gdk::set_program_class(::client::NAME);
    // On a Mac the launcher alone has no Dock icon and no Cmd-Tab entry, as Raycast's hasn't: the One App is an
    // accessory until an app's window opens in it (the renderer's `Display::open`), and again once the last one closes.
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    *OPENED.lock().expect("nothing panics holding it") = link;
    crate::begin(Computer, started);
    crate::page::load(&home);

    menu_events();
    #[cfg(target_os = "macos")]
    let _menu = menu();
    let _ring = ring();

    let window = WindowBuilder::new().with_title(crate::page::NAME).with_inner_size(SIZE).with_decorations(false).with_resizable(false).with_visible(false).with_always_on_top(true);
    #[cfg(not(target_os = "linux"))]
    let window = window.with_transparent(true);
    #[cfg(target_os = "macos")]
    let window = window.with_visible_on_all_workspaces(true);
    #[cfg(target_os = "linux")]
    let window = {
        use tao::platform::unix::WindowBuilderExtUnix;
        // GTK keeps a window that isn't resizable the size it was made, so here it's resizable, held to the size it's
        // given (`Told::Resize`).
        window.with_skip_taskbar(true).with_resizable(true).with_min_inner_size(SIZE).with_max_inner_size(SIZE)
    };
    #[cfg(windows)]
    let window = {
        use tao::platform::windows::WindowBuilderExtWindows;
        window.with_skip_taskbar(true).with_undecorated_shadow(true)
    };
    let window = window.build(&event_loop)?;
    glass(&window);

    // The web launcher's page, made once it's known the launcher isn't native (`Told::Native`): its own pages and
    // apps' it asks the app for, and has nothing else load.
    let page_started = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "linux")]
    keys(&window);
    // Its data is where Tauri kept it, which Windows' WebView2 shares with each app's window.
    #[cfg(any(target_os = "linux", windows))]
    let data_directory = crate::shortcuts::local_data_dir().ok();
    #[cfg(target_os = "macos")]
    let data_directory: Option<std::path::PathBuf> = None;
    let make_page = {
        let page_started = page_started.clone();
        move |window: &Window| match Discarding::new(window, Page { started: page_started.clone() }, data_directory.clone()) {
            Ok(page) => Some(page),
            Err(error) => {
                eprintln!("the launcher's page didn't load: {error:#}");
                None
            }
        }
    };
    let mut page: Option<Discarding<Page>> = None;
    // The native launcher's window, once it's open, and whether it's been decided which it is.
    let mut native: Option<renderer::platform::WindowId> = None;
    let mut decided = false;
    crate::spawn(prepare());

    crate::spawn(crate::opens());
    hosts::serve(&home)?;
    #[cfg(all(any(windows, target_os = "linux"), not(debug_assertions)))]
    if let Err(error) = links() {
        eprintln!("this device's one: links don't open this app: {error:#}");
    }
    #[cfg(all(windows, not(debug_assertions)))]
    if let Err(error) = toasts() {
        eprintln!("this device shows this app's notifications as another app's: {error:#}");
    }
    let _shortcut = shortcut(&home);
    // Started again as it updated, the app windows that were open open again.
    // The windows open before it updated, once it's signed in: before, an app's page can't be read. The launcher, open
    // as the native one is, isn't an app's window.
    if let Some(reopen) = std::env::var("ONE_REOPEN").ok().and_then(|reopen| serde_json::from_str::<Vec<(String, Option<String>)>>(&reopen).ok()) {
        crate::spawn(async move {
            crate::signed_in().await;
            let listed = hosts::command("main", "apps", json!({})).await.unwrap_or_default();
            let launcher = crate::native::pages(&listed).find(|(_, meta)| meta["category"] == "launcher").map(|(page, _)| page);
            for (entrypoint, subject) in reopen.into_iter().filter(|(entrypoint, _)| Some(entrypoint) != launcher.as_ref()) {
                if let Err(error) = hosts::open_window(entrypoint.clone(), subject, entrypoint) {
                    eprintln!("{error}");
                }
            }
        });
    }
    eprintln!("the window was made {} ms after the app started", started.elapsed().as_millis());

    // Started for an app's own window alone, or as it updated with it put away, the launcher stays put away. Until
    // it's decided which launcher it is, it's shown once it is, if it's wanted then.
    let mut shown = false;
    let mut wanted = std::env::var_os("ONE_HIDDEN").is_none();
    let mut focused = false;
    event_loop.run(move |event, _, flow| {
        if let (Event::WindowEvent { event, .. }, Some(page)) = (&event, page.as_mut()) {
            page.heard(&window, event);
        }
        match event {
            Event::WindowEvent { event: WindowEvent::Focused(true), .. } => focused = true,
            // Something else was clicked, or came in front: it goes, as Spotlight does.
            Event::WindowEvent { event: WindowEvent::Focused(false), .. } if shown && focused => {
                focused = false;
                shown = false;
                put_away(&window, page.as_mut(), false);
            }
            // Alt-F4, which closes nothing but puts it away.
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                shown = false;
                put_away(&window, page.as_mut(), true);
            }
            // The Dock's icon clicked.
            #[cfg(target_os = "macos")]
            Event::Reopen { .. } => tell(Told::Summon),
            // A `one:` link opened, as a Mac tells the app that takes them.
            #[cfg(target_os = "macos")]
            Event::Opened { urls } => {
                for link in urls.iter().filter_map(|url| crate::link(url.as_str())) {
                    tell(Told::Open(link));
                }
            }
            Event::UserEvent(Told::Script(script)) => {
                if let Some(webview) = page.as_ref().and_then(|page| page.webview()) {
                    drop(webview.evaluate_script(&script))
                }
            }
            // The native launcher's ready, put away, and shown now if it's wanted: the web launcher goes.
            Event::UserEvent(Told::Native(Some(id))) => {
                decided = true;
                native = Some(id);
                if shown {
                    put_away(&window, page.as_mut(), false);
                    shown = false;
                }
                page = None;
                if std::mem::take(&mut wanted) {
                    let asked = std::time::Instant::now();
                    on_native(move |windows| windows.summon(id, asked, None));
                }
            }
            // It's the web launcher: its page loads, and it shows if it's wanted.
            Event::UserEvent(Told::Native(None)) => {
                decided = true;
                if let Some(id) = native.take() {
                    wanted = true;
                    on_native(move |windows| windows.close(id));
                }
                if page.is_none() {
                    page = make_page(&window);
                    if let Some(page) = page.as_mut().filter(|_| !wanted) {
                        page.shown(&window, false);
                    }
                }
                if std::mem::take(&mut wanted) {
                    show(&window, page.as_mut());
                    shown = true;
                }
            }
            Event::UserEvent(Told::Toggle(asked, key)) if native.is_some() => {
                let id = native.expect("it's native");
                on_native(move |windows| if windows.showing(id) { windows.put_away(id, true) } else { windows.summon(id, asked, key.as_deref()) });
            }
            Event::UserEvent(Told::Summon) if native.is_some() => {
                let (id, asked) = (native.expect("it's native"), std::time::Instant::now());
                on_native(move |windows| windows.summon(id, asked, None));
            }
            Event::UserEvent(Told::Toggle(..) | Told::Summon) if !decided => wanted = true,
            Event::UserEvent(Told::Toggle(..)) if shown => {
                shown = false;
                put_away(&window, page.as_mut(), true);
            }
            Event::UserEvent(Told::Toggle(..) | Told::Summon) => {
                show(&window, page.as_mut());
                shown = true;
                focused = false;
            }
            Event::UserEvent(Told::Hide(give_back)) if native.is_some() => {
                let id = native.expect("it's native");
                on_native(move |windows| windows.put_away(id, give_back));
            }
            Event::UserEvent(Told::Hide(give_back)) => {
                shown = false;
                focused = false;
                put_away(&window, page.as_mut(), give_back);
            }
            Event::UserEvent(Told::Theme(theme)) => window.set_theme(theme.map(|theme| match theme {
                crate::Theme::Light => Theme::Light,
                crate::Theme::Dark => Theme::Dark,
            })),
            // Its top stays where it is, as Raycast's and Spotlight's grow and shrink downwards.
            Event::UserEvent(Told::Resize(width, height)) => {
                if let Some(id) = native {
                    let width = width.map(|width| width.clamp(1.0, MOST.width) as f32);
                    let height = height.unwrap_or(SIZE.height).clamp(1.0, MOST.height) as f32;
                    on_native(move |windows| windows.resize(id, width, height));
                    return;
                }
                let height = height.unwrap_or(SIZE.height).clamp(1.0, SIZE.height);
                let size = LogicalSize::new(SIZE.width, height);
                let at = window.outer_position();
                #[cfg(target_os = "linux")]
                {
                    window.set_min_inner_size(Some(size));
                    window.set_max_inner_size(Some(size));
                }
                window.set_inner_size(size);
                if let Ok(at) = at {
                    window.set_outer_position(at);
                }
            }
            // The device's session started again, or the native launcher's page was updated: which launcher it is
            // is decided again, the native one open again where it can be.
            Event::UserEvent(Told::Reload) if native.is_some() || !decided => {
                if let Some(id) = native.take() {
                    on_native(move |windows| {
                        if windows.showing(id) {
                            tell(Told::Summon);
                        }
                        windows.close(id)
                    });
                }
                if let Some(launcher) = LAUNCHER.lock().expect("nothing panics holding it").as_deref() {
                    hosts::forget(launcher, None);
                }
                decided = false;
                crate::spawn(prepare());
            }
            Event::UserEvent(Told::Talks) if native.is_some() || !decided => {}
            // Its page is made again, and may be native now.
            Event::UserEvent(Told::Reload | Told::Talks) => {
                page_started.store(false, Ordering::Relaxed);
                page = None;
                decided = false;
                wanted = shown;
                if shown {
                    put_away(&window, None, false);
                    shown = false;
                }
                crate::spawn(prepare());
            }
            // The native launcher opens a link to an app in that app's window.
            Event::UserEvent(Told::Open(link)) if native.is_some() => crate::spawn(crate::native::go(link, None)),
            // The page opens it once it's there to (`start`), and if it isn't yet, once it asks where it is.
            Event::UserEvent(Told::Open(link)) => {
                if let Some(webview) = page.as_ref().and_then(|page| page.webview()).filter(|_| page_started.load(Ordering::Relaxed)) {
                    let _ = webview.evaluate_script(&format!("window.__ONE_HOST__.event(\"open\", {})", Value::from(link)));
                } else {
                    *OPENED.lock().expect("nothing panics holding it") = Some(link);
                    // Discarded, its page loads again to open it, out of sight if it's put away.
                    if let Some(page) = page.as_mut() {
                        page.needed(&window);
                    }
                }
            }
            Event::UserEvent(Told::Restart) => restart(&home, shown),
            Event::UserEvent(Told::Quit) => *flow = ControlFlow::Exit,
            _ => {}
        }
        if *flow != ControlFlow::Exit {
            *flow = page.as_mut().map_or(ControlFlow::Wait, |page| page.discard_due(&window));
        }
    })
}

/// Shows the launcher, centred near the top of the screen the pointer is on, with the keyboard.
fn show(window: &Window, mut page: Option<&mut Discarding<Page>>) {
    // Discarded while it was put away, its page loads again.
    if let Some(page) = page.as_mut() {
        page.shown(window, true);
    }
    let pointer = window.cursor_position().ok();
    let screen = pointer
        .and_then(|at| {
            window.available_monitors().find(|monitor| {
                let (corner, size) = (monitor.position(), monitor.size());
                at.x >= corner.x as f64 && at.y >= corner.y as f64 && at.x < (corner.x + size.width as i32) as f64 && at.y < (corner.y + size.height as i32) as f64
            })
        })
        .or_else(|| window.current_monitor())
        .or_else(|| window.primary_monitor());
    if let Some(screen) = screen {
        let scale = screen.scale_factor();
        let (corner, size) = (screen.position(), screen.size());
        let panel = SIZE.to_physical::<f64>(scale);
        let x = corner.x as f64 + (size.width as f64 - panel.width) / 2.0;
        // A fifth of the way down, as Spotlight sits, and never off the bottom of a short screen.
        let y = corner.y as f64 + (size.height as f64 * 0.2).min((size.height as f64 - panel.height).max(0.0));
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
    window.set_visible(true);
    // A Mac's app comes in front with it, so an app opened from it comes in front in turn (`hosts::open_window`).
    #[cfg(target_os = "macos")]
    activate();
    window.set_focus();
    if let Some(webview) = page.and_then(|page| page.webview()) {
        let _ = webview.focus();
    }
}

/// Puts the launcher away. Given back, the keyboard goes to the app that had it before, as a Mac hides an app.
fn put_away(
    window: &Window,
    page: Option<&mut Discarding<Page>>,
    #[cfg_attr(not(target_os = "macos"), expect(unused_variables))] give_back: bool,
) {
    window.set_visible(false);
    if let Some(page) = page {
        page.shown(window, false);
    }
    #[cfg(target_os = "macos")]
    if give_back {
        unsafe {
            use objc2::runtime::AnyObject;
            let application: *mut AnyObject = objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
            let nothing: *mut AnyObject = std::ptr::null_mut();
            let () = objc2::msg_send![application, hide: nothing];
        }
    }
}

/// The page's web view: its own files and its apps', what it asks (`hosts::command`), and the device's own browser for
/// anywhere else, as an app's own window has them (`host`).
struct Page {
    /// Whether the page has asked where it is (`start`), once it's there to open a link.
    started: Arc<AtomicBool>,
}

impl crate::discard::Page for Page {
    fn make<'a>(&mut self, builder: WebViewBuilder<'a>) -> WebViewBuilder<'a> {
        let key = format!("{:016x}{:016x}", crate::host::random(), crate::host::random());
        let own = |_: WebViewId, request: Request<Vec<u8>>| crate::page::respond(request.uri().path());
        let apps_file = |_: WebViewId, request: Request<Vec<u8>>, responder: RequestAsyncResponder| {
            let address = request.uri().to_string();
            crate::spawn(async move {
                let file = match crate::views() {
                    Ok(views) => views.file(&address).await.map_err(|error| format!("{error:#}")),
                    Err(error) => Err(error),
                };
                responder.respond(crate::host::app_file(file));
            });
        };
        let (page_key, started) = (key.clone(), self.started.clone());
        let ipc = move |request: Request<String>| {
            let Ok(message) = serde_json::from_str::<Value>(request.body()) else { return };
            if message["key"] != page_key.as_str() {
                return;
            }
            let (id, command, args) =
                (message["id"].clone(), message["command"].as_str().unwrap_or_default().to_owned(), message["args"].clone());
            if command == "start" {
                started.store(true, Ordering::Relaxed);
            }
            crate::spawn(async move {
                let answer = hosts::command("main", &command, args).await;
                tell(Told::Script(crate::host::answer(&id, answer.map_err(Value::from))));
            });
        };
        let builder = builder
            .with_custom_protocol("tauri".into(), own)
            .with_asynchronous_custom_protocol("oneapp".into(), apps_file)
            .with_ipc_handler(ipc)
            .with_initialization_script_for_main_only(crate::host::BRIDGE.replace("__KEY__", &key), true)
            // The page draws the launcher's own glass where the window has none (`data-launcher` in client.css).
            .with_initialization_script_for_main_only(
                format!(
                    "document.documentElement.dataset.launcher = {:?}",
                    if cfg!(target_os = "linux") { "solid" } else { "glass" }
                ),
                true,
            )
            .with_navigation_handler(|to| crate::stays(&to))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_transparent(!cfg!(target_os = "linux"))
            .with_devtools(cfg!(debug_assertions))
            .with_url(crate::HOME);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let builder = builder.with_initialization_script_for_main_only(crate::INITIALIZATION_SCRIPT, false);
        builder
    }

    /// A developer's build writes what its pages say to their console to its stdout, as WebKitGTK offers: its web
    /// view has no other console to read without a window of its own.
    fn loaded(&mut self, _webview: &WebView) {
        #[cfg(all(target_os = "linux", debug_assertions))]
        {
            use webkit2gtk::{SettingsExt, WebViewExt};
            use wry::WebViewExtUnix;
            if let Some(settings) = _webview.webview().settings() {
                settings.set_enable_write_console_messages_to_stdout(true);
            }
        }
    }

    /// Gone, it opens a link once it's loaded again and asks where it is.
    fn discarded(&mut self) {
        self.started.store(false, Ordering::Relaxed);
    }
}

/// The launcher's glass, the system's own material under its page, rounded: a Mac's Liquid Glass where it has it
/// (macOS 26), and its window vibrancy before that, and Windows' Acrylic. Linux has none, so the page draws its own
/// (`data-launcher`).
fn glass(#[cfg_attr(target_os = "linux", expect(unused_variables))] window: &Window) {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::runtime::{AnyClass, AnyObject};
        use objc2_foundation::NSRect;
        use tao::platform::macos::WindowExtMacOS;
        let panel = window.ns_window().cast::<AnyObject>();
        // On every Space, over a full-screen app's too, and out of Cmd-` and Mission Control, as a menu bar app's
        // panel is (`NSWindowCollectionBehavior`: canJoinAllSpaces, stationary, ignoresCycle, fullScreenAuxiliary).
        let behaviour: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
        let () = objc2::msg_send![panel, setCollectionBehavior: behaviour];
        // NSStatusWindowLevel: over other apps' windows, as Spotlight is.
        let () = objc2::msg_send![panel, setLevel: 25isize];
        let () = objc2::msg_send![panel, setHasShadow: true];
        let content: *mut AnyObject = objc2::msg_send![panel, contentView];
        let bounds: NSRect = objc2::msg_send![content, bounds];
        match AnyClass::get(c"NSGlassEffectView") {
            Some(glass) => {
                let view: *mut AnyObject = objc2::msg_send![glass, alloc];
                let view: *mut AnyObject = objc2::msg_send![view, initWithFrame: bounds];
                let () = objc2::msg_send![view, setCornerRadius: CORNER];
                // Width and height follow the window's.
                let () = objc2::msg_send![view, setAutoresizingMask: 18usize];
                let nothing: *mut AnyObject = std::ptr::null_mut();
                // NSWindowBelow: under the web view.
                let () = objc2::msg_send![content, addSubview: view, positioned: -1isize, relativeTo: nothing];
            }
            None => {
                use window_vibrancy::{NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy};
                if let Err(error) = apply_vibrancy(window, NSVisualEffectMaterial::HudWindow, Some(NSVisualEffectState::Active), Some(CORNER)) {
                    eprintln!("the launcher has no glass: {error}");
                }
            }
        }
    }
    #[cfg(windows)]
    {
        if let Err(error) = window_vibrancy::apply_acrylic(window, None) {
            eprintln!("the launcher has no glass: {error}");
        }
        // Windows 11's own round corners, which a window with no frame doesn't have unless it asks.
        use tao::platform::windows::WindowExtWindows;
        use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
        let round = DWMWCP_ROUND;
        let window = windows::Win32::Foundation::HWND(window.hwnd() as _);
        if let Err(error) = unsafe { DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, std::ptr::from_ref(&round).cast(), size_of_val(&round) as u32) } {
            eprintln!("the launcher has square corners: {error}");
        }
    }
}

/// A Mac's app comes in front, as a person summoned it.
#[cfg(target_os = "macos")]
fn activate() {
    unsafe {
        use objc2::runtime::AnyObject;
        let application: *mut AnyObject = objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
        let () = objc2::msg_send![application, activateIgnoringOtherApps: true];
    }
}

/// The launcher's shortcut, which shows it from anywhere, or puts it away. It's ⌥Space on a Mac, as Raycast suggests,
/// which macOS leaves free (Spotlight is ⌘Space, the input source ⌃Space); on Windows and Linux Alt+Space is the
/// window menu's, and Super+Space the input source's, so it's Super+Alt+Space there. A file `shortcut` in the client's
/// folder says another, as `alt+Space` (global-hotkey's words), or nothing for none; Linux's is X11's, and a Wayland
/// session has none. → what keeps it, for as long as the app runs.
fn shortcut(home: &std::path::Path) -> Option<global_hotkey::GlobalHotKeyManager> {
    use global_hotkey::hotkey::HotKey;
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    let default = if cfg!(target_os = "macos") { "alt+Space" } else { "super+alt+Space" };
    let chosen = std::fs::read_to_string(home.join("shortcut")).map(|chosen| chosen.trim().to_owned()).unwrap_or_else(|_| default.to_owned());
    if chosen.is_empty() {
        return None;
    }
    let key: HotKey = match chosen.parse() {
        Ok(key) => key,
        Err(error) => {
            eprintln!("the launcher's shortcut isn't one ({chosen}): {error}");
            return None;
        }
    };
    let keeper = GlobalHotKeyManager::new().and_then(|keeper| keeper.register(key).map(|()| keeper));
    let keeper = match keeper {
        Ok(keeper) => keeper,
        Err(error) => {
            eprintln!("the launcher has no shortcut ({chosen}): {error}");
            return None;
        }
    };
    // The key it ends with, as the DOM names it, which a page it summons doesn't take as its own.
    let code = format!("{:?}", key.key);
    let last = match code.as_str() {
        "Space" => " ".to_owned(),
        letter if letter.starts_with("Key") => letter[3..].to_lowercase(),
        digit if digit.starts_with("Digit") => digit[5..].to_owned(),
        other => other.to_owned(),
    };
    GlobalHotKeyEvent::set_event_handler(Some(move |pressed: GlobalHotKeyEvent| {
        if pressed.id == key.id() && pressed.state == HotKeyState::Pressed {
            tell(Told::Toggle(std::time::Instant::now(), Some(last.clone())));
        }
    }));
    eprintln!("the launcher's shortcut is {chosen}");
    Some(keeper)
}

/// Starts the app again on the build it was just updated to: the app windows that are open open again in it, and the
/// launcher shows again if it was showing. Each app's window ends with this program's channel.
fn restart(#[cfg_attr(not(unix), expect(unused_variables))] home: &std::path::Path, shown: bool) {
    // Asked again before it has gone, it starts one program, not two (two did, 19 Nov).
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let reopen = serde_json::to_string(&hosts::open_now()).unwrap_or_default();
    // The new program serves the channel at the same address.
    #[cfg(unix)]
    let _ = std::fs::remove_file(hosts::address(home));
    let mut command = match cli::serving::program() {
        Ok(program) => std::process::Command::new(program),
        Err(error) => {
            STARTED.store(false, std::sync::atomic::Ordering::Relaxed);
            return eprintln!("couldn't start again: {error}");
        }
    };
    command.env("ONE_RESTARTED", "1").env("ONE_REOPEN", reopen).stdin(std::process::Stdio::null());
    if shown {
        command.env_remove("ONE_HIDDEN");
    } else {
        command.env("ONE_HIDDEN", "1");
    }
    match command.spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => {
            STARTED.store(false, std::sync::atomic::Ordering::Relaxed);
            eprintln!("couldn't start again: {error}")
        }
    }
}

/// Has the device open `one:` links with this program, as Tauri's deep links did: Linux by a desktop entry that takes
/// them (`one-handler.desktop`), and Windows by its classes in the registry. A Mac's are its bundle's, in its
/// `Info.plist`.
#[cfg(all(any(windows, target_os = "linux"), not(debug_assertions)))]
fn links() -> anyhow::Result<()> {
    let program = cli::serving::program()?;
    #[cfg(target_os = "linux")]
    {
        let entries = dirs::data_dir().ok_or_else(|| anyhow::anyhow!("this user has no data folder"))?.join("applications");
        std::fs::create_dir_all(&entries)?;
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\" %u\nMimeType=x-scheme-handler/one;\nNoDisplay=true\nTerminal=false\n",
            crate::page::NAME,
            program.display()
        );
        let file = entries.join("one-handler.desktop");
        if std::fs::read_to_string(&file).ok().as_deref() != Some(entry.as_str()) {
            std::fs::write(&file, entry)?;
            std::process::Command::new("xdg-mime").args(["default", "one-handler.desktop", "x-scheme-handler/one"]).status()?;
        }
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
        use windows::core::HSTRING;
        let set = |key: &str, name: &str, value: &str| {
            let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
            RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_SZ.0, Some(value.as_ptr().cast()), (value.len() * 2) as u32).ok()
        };
        let program = program.display().to_string();
        set(r"Software\Classes\one", "", &format!("URL:{} protocol", ::client::NAME))?;
        set(r"Software\Classes\one", "URL Protocol", "")?;
        set(r"Software\Classes\one\DefaultIcon", "", &format!("{program},0"))?;
        set(r"Software\Classes\one\shell\open\command", "", &format!("\"{program}\" \"%1\""))?;
    }
    Ok(())
}

/// Has Windows show this app's notifications as this app's, with its name and icon: a toast is shown as the app whose
/// AppUserModelID it names (`notifications::listen`), and an app that has no shortcut with that ID in the Start menu
/// is known by the ID's registration in the registry, as an unpackaged app's is.
#[cfg(all(windows, not(debug_assertions)))]
fn toasts() -> anyhow::Result<()> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    use windows::core::HSTRING;
    let icon = crate::shortcuts::local_data_dir()?.join("AppIcon.ico");
    std::fs::create_dir_all(icon.parent().expect("a file has a folder"))?;
    let drawn = include_bytes!("../../../icons/AppIcon.ico");
    if std::fs::read(&icon).ok().as_deref() != Some(drawn.as_slice()) {
        std::fs::write(&icon, drawn)?;
    }
    let key = format!(r"Software\Classes\AppUserModelId\{}", ::client::NAME);
    for (name, value) in [("DisplayName", crate::page::NAME.to_owned()), ("IconUri", icon.display().to_string())] {
        let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(&key), &HSTRING::from(name), REG_SZ.0, Some(value.as_ptr().cast()), (value.len() * 2) as u32)
        }
        .ok()?;
    }
    Ok(())
}

/// Ctrl-Q quits the app, as it does a Linux app.
#[cfg(target_os = "linux")]
fn keys(window: &Window) {
    use gtk::gdk::{ModifierType, keys::constants as key};
    use gtk::glib::Propagation;
    use gtk::prelude::*;
    use tao::platform::unix::WindowExtUnix;
    window.gtk_window().connect_key_press_event(move |_, pressed| {
        let held = pressed.state() & (ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK | ModifierType::MOD1_MASK | ModifierType::SUPER_MASK);
        if held != ModifierType::CONTROL_MASK || pressed.keyval().to_lower() != key::q {
            return Propagation::Proceed;
        }
        tell(Told::Quit);
        Propagation::Stop
    });
}

/// What a Mac app's About says of it: the build that's running, as `one --version` says it, `0.26.90 (3810.a2b17a3b510d)`,
/// rather than its bundle's, which an update that replaces the program alone leaves as it was.
#[cfg(target_os = "macos")]
pub(crate) fn about() -> muda::AboutMetadata {
    let (version, build) = ::client::BUILD.split_once('+').unwrap_or((::client::BUILD, ""));
    muda::AboutMetadata { short_version: Some(version.to_owned()), version: Some(build.to_owned()), ..Default::default() }
}

/// A Mac app's menu bar, as every Mac app's: its app menu (About, Settings… ⌘, Hide ⌘H, Hide Others ⌥⌘H, Show All,
/// Quit), Edit, which a field's copy and paste keys need, and Window (Minimize ⌘M, Zoom, Enter Full Screen ⌃⌘F, Home
/// Screen ⇧⌘H as Finder's and Safari's Home, Bring All to Front, and every window open, which macOS lists itself).
/// Its items are heard where the ring's are (`ring`).
#[cfg(target_os = "macos")]
fn menu() -> muda::Menu {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let menu = Menu::new();
    let settings = MenuItem::with_id("settings", "Settings…", true, Some(Accelerator::new(Modifiers::META, Code::Comma)));
    let home = MenuItem::with_id("open", "Home Screen", true, Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyH)));
    let app = Submenu::with_items(
        crate::page::NAME,
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about())),
            &PredefinedMenuItem::separator(),
            &settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    );
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    );
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(Some("Zoom")),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &home,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ],
    );
    if let (Ok(app), Ok(edit), Ok(window)) = (app, edit, window) {
        let _ = menu.append_items(&[&app, &edit, &window]);
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
    }
    menu
}

/// What's chosen in a menu, every one muda makes: the ring's and the menu bar's (`ring`, `menu`), and a window's own,
/// a field's menu for text, which its window does (`renderer::platform::menu_chosen`).
fn menu_events() {
    use tray_icon::menu::MenuEvent;
    MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
        "open" => tell(Told::Summon),
        "settings" => tell(Told::Open("settings".to_owned())),
        "quit" => tell(Told::Quit),
        id => drop(renderer::platform::menu_chosen(id)),
    }));
}

/// The ring in a Mac's menu bar, as Raycast's is there, in Windows' notification area, as a running app that has no
/// window of its own is there, and in a Linux desktop's panel. Its menu has Open, Settings… and Quit: on a Mac a click
/// opens it, as Raycast's and every menu bar icon's does; on Windows a click shows the launcher or puts it away, and a
/// right-click opens the menu, as a notification-area icon's does. A Mac's is a template, drawn as the menu bar's
/// other icons are.
fn ring() -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};
    #[cfg(windows)]
    use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
    #[cfg(target_os = "macos")]
    let picture = include_bytes!("../../../icons/MenuBarIcon@2x.png");
    #[cfg(not(target_os = "macos"))]
    let picture = include_bytes!("../../../icons/AppIcon@2x.png");
    let picture = resvg::tiny_skia::Pixmap::decode_png(picture).ok()?;
    let icon = Icon::from_rgba(picture.data().to_vec(), picture.width(), picture.height()).ok()?;
    let open = MenuItem::with_id("open", format!("Open {}", crate::page::NAME), true, None);
    // Settings…, the Settings app's window, as Raycast's menu has its own settings.
    let settings = MenuItem::with_id("settings", "Settings…", true, None);
    let quit = MenuItem::with_id("quit", format!("Quit {}", crate::page::NAME), true, None);
    let menu = Menu::with_items(&[&open, &settings, &PredefinedMenuItem::separator(), &quit]).ok()?;
    #[cfg(windows)]
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            tell(Told::Toggle(std::time::Instant::now(), None));
        }
    }));
    let ring = TrayIconBuilder::new().with_icon(icon).with_icon_as_template(cfg!(target_os = "macos")).with_menu(Box::new(menu)).with_menu_on_left_click(cfg!(target_os = "macos")).with_tooltip(crate::page::NAME).build();
    ring.map_err(|error| eprintln!("the menu bar has no ring: {error}")).ok()
}
