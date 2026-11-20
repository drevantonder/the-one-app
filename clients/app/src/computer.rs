//! The app on a computer (a Mac, Windows or Linux): a launcher, as Spotlight and Raycast are. Its window is the Home
//! Screen in a panel the system summons over whatever's in front, near the top of the screen the pointer is on, which
//! goes again once an app opens from it, on Escape, or once something else is clicked. Every app opens in a window of
//! its own (`hosts`, `host`), so starting them is all the launcher does. It's summoned by its shortcut (`shortcut`), the
//! app's own icon in the Dock or taskbar (a second start, `hosts::again`), a Mac's menu bar, where the ring is, and an
//! app window's Home. The program runs on with it put away, holding the device's one connection to its network, its
//! notifications and its app windows, until it's quit (its menu, Cmd-Q, or Ctrl-Q on Linux).
//!
//! The launcher is drawn natively, in a panel of this process's own, as every app's native window is, and it shows
//! within a frame of being summoned: the Home Screen's page, or in its place a page of the client's own
//! (`native::Instead`), where the device signs in, or says Update Required while its servers say this program is
//! behind them, or says why the Home Screen didn't open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};
use tao::dpi::LogicalSize;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};

use crate::native::Instead;
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

/// What the launcher shows: the Home Screen, or in its place (`Instead`), where the device signs in, Update Required,
/// or why the Home Screen didn't open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shows {
    Home,
    SignIn,
    Behind,
    Unopened,
}

/// What the program is told to do on its main thread, where its event loop is.
pub(crate) enum Told {
    /// Shows the launcher, or puts it away if it's showing: its shortcut, or the ring in a Mac's menu bar, when it
    /// was pressed, and the key the shortcut ends with, as the DOM names it.
    Toggle(std::time::Instant, Option<String>),
    Summon,
    /// The launcher is open in this window, showing what it shows, or with none, there's none, as the `prepare` it's
    /// from opened it: one from before the latest is gone already.
    Native(Option<renderer::platform::WindowId>, Shows, u64),
    /// This device's native windows can't start, so there's no launcher: the system's own alert says why.
    Unnative(String),
    /// Puts it away, and with `true` gives the keyboard back to the app that had it (Escape, or its page done).
    Hide(bool),
    /// Makes it as tall as its page's content, or its tallest, and as wide where it says (`resize`).
    Resize(Option<f64>, Option<f64>),
    /// Opens it again, as the device's session starts again, or its page was updated.
    Reload,
    /// The program before it, which started it as it updated, has ended: what only one may have is its now, the
    /// channel and the launcher's shortcut (`opened`).
    Handed,
    /// The program it started as it updated is up: it lets go of the launcher's shortcut and ends (`restart`).
    Ending,
    /// Its servers say this program is behind them: it says Update Required.
    Behind,
    /// Its servers talk with it again after they said it was behind: the Home Screen is back.
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

/// The launcher is open in window `id`, or with none, there's none.
pub(crate) fn native(id: Option<renderer::platform::WindowId>) {
    tell(Told::Native(id, Shows::Unopened, PREPARED.load(std::sync::atomic::Ordering::Relaxed)))
}

/// Which `prepare` is the latest: each opens the launcher as the device is then, and only the latest's is it.
static PREPARED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// This device's native windows couldn't start, so there's no launcher: the system's own alert says why.
pub(crate) fn unnative(why: String) {
    tell(Told::Unnative(why))
}

/// Its servers say this program is behind them: the launcher says Update Required (`Told::Behind`).
pub(crate) fn behind() {
    tell(Told::Behind)
}

/// Its servers talk with it again: a launcher that says Update Required shows the Home Screen again (`Told::Talks`).
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

/// Opens the launcher, and tells the main thread its window (`Told::Native`): the Home Screen, or in its place where the
/// device signs in, Update Required, or why the Home Screen didn't open (`Instead`). The `one:` link the app was
/// started with, or that came before, opens once the Home Screen has.
async fn prepare() {
    // One at a time: two opening the same page at once would each take the other's place as its client knows it.
    static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let prepared = PREPARED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let _one = ONE.lock().await;
    if prepared != PREPARED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let opened = |id, shows| tell(Told::Native(id, shows, prepared));
    let instead = |instead| crate::native::launcher_instead(instead);
    let unopened = |why| Instead::Unopened(crate::native::Unopened::launcher(why));
    crate::settled().await;
    let started = match crate::start().await {
        Ok(started) => started,
        Err(why) => return opened(instead(unopened(format!("the One App didn't start: {why}"))).await, Shows::Unopened),
    };
    // The link it was asked to open waits for the Home Screen, through signing in and updating.
    if let Some(link) = started["open"].as_str() {
        *OPENED.lock().expect("nothing panics holding it") = Some(link.to_owned());
    }
    if started["state"]["session"]["state"] == "signIn" {
        return opened(instead(Instead::SignIn).await, Shows::SignIn);
    }
    if started["state"]["connection"] == "behind" {
        return opened(instead(Instead::Behind).await, Shows::Behind);
    }
    let listed = match hosts::command("main", "apps", json!({})).await {
        Ok(listed) => listed,
        Err(why) => return opened(instead(unopened(format!("its apps weren't listed: {why}"))).await, Shows::Unopened),
    };
    let Some((page, _)) = crate::native::pages(&listed).find(|(_, meta)| meta["category"] == "launcher") else {
        return opened(instead(unopened("no app on your network is a Home Screen".to_owned())).await, Shows::Unopened);
    };
    *LAUNCHER.lock().expect("nothing panics holding it") = Some(page.clone());
    let id = crate::native::launcher(&page).await;
    if let Some(link) = OPENED.lock().expect("nothing panics holding it").take().filter(|_| id.is_some()) {
        crate::spawn(crate::native::go(link, None));
    }
    opened(id, if id.is_some() { Shows::Home } else { Shows::Unopened })
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

/// A `one:` link for the Home Screen to open once it has: what the app was started with, or one that came before.
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

    /// Each window is native, and told the Colour Mode chosen as it's chosen (`chose`): the app has none of its own.
    fn theme(&self, _: Option<crate::Theme>) {}
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
    // Started again by itself as it updates (`restart`), it doesn't hand over to the one that's ending, which hands
    // over to it once it's ready (`opened`). What it starts isn't started again.
    let restarted = std::env::var_os("ONE_RESTARTED").is_some();
    unsafe { std::env::remove_var("ONE_RESTARTED") };
    RESTARTED.store(restarted, Ordering::Relaxed);
    if !restarted && hosts::again(&home, link.as_deref()) {
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
    // Started by the program before it, it takes the channel and the shortcut once that one has ended (`Told::Handed`),
    // read before what tells it the app's up (`opened`) runs.
    let restarted = RESTARTED.load(Ordering::Relaxed);
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

    menu_events();
    #[cfg(target_os = "macos")]
    let _menu = menu();
    let _ring = ring();

    // The launcher's window, once it's open, whether it says Update Required, and whether it's been decided which it
    // is.
    let mut native: Option<renderer::platform::WindowId> = None;
    let mut shows = Shows::Home;
    let mut decided = false;
    crate::spawn(prepare());

    crate::spawn(crate::opens());
    if !restarted {
        hosts::serve(&home)?;
    }
    #[cfg(all(any(windows, target_os = "linux"), not(debug_assertions)))]
    if let Err(error) = links() {
        eprintln!("this device's one: links don't open this app: {error:#}");
    }
    #[cfg(all(windows, not(debug_assertions)))]
    if let Err(error) = toasts() {
        eprintln!("this device shows this app's notifications as another app's: {error:#}");
    }
    let mut _shortcut = if restarted { None } else { shortcut(&home) };
    // Started again as it updated, the app windows that were open open again, once it's signed in: before, an app's
    // page can't be read. The launcher, open as the native one is, isn't an app's window.
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

    // Started for an app's own window alone, or as it updated with it put away, the launcher stays put away. Until
    // it's open, it's shown once it is, if it's wanted then.
    let mut wanted = std::env::var_os("ONE_HIDDEN").is_none();
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
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
            // The launcher's open, put away, and shown now if it's wanted.
            // One from before the latest `prepare` is the device as it was: it goes.
            Event::UserEvent(Told::Native(id, _, from)) if from != PREPARED.load(std::sync::atomic::Ordering::Relaxed) => {
                if let Some(id) = id {
                    on_native(move |windows| windows.close(id));
                }
            }
            Event::UserEvent(Told::Native(id, showing, _)) => {
                decided = true;
                native = id;
                shows = showing;
                if let Some(id) = native.filter(|_| std::mem::take(&mut wanted)) {
                    let asked = std::time::Instant::now();
                    on_native(move |windows| windows.summon(id, asked, None));
                }
            }
            // There's no launcher, nor any window: the system's alert says why, and the app ends.
            Event::UserEvent(Told::Unnative(why)) => {
                alert(&format!("The One App can't open its windows on this device: {why}"));
                *flow = ControlFlow::Exit;
            }
            Event::UserEvent(Told::Toggle(asked, key)) if native.is_some() => {
                let id = native.expect("it's open");
                on_native(move |windows| if windows.showing(id) { windows.put_away(id, true) } else { windows.summon(id, asked, key.as_deref()) });
            }
            Event::UserEvent(Told::Summon) if native.is_some() => {
                let (id, asked) = (native.expect("it's open"), std::time::Instant::now());
                on_native(move |windows| windows.summon(id, asked, None));
            }
            Event::UserEvent(Told::Toggle(..) | Told::Summon) if !decided => wanted = true,
            Event::UserEvent(Told::Hide(give_back)) => {
                if let Some(id) = native {
                    on_native(move |windows| windows.put_away(id, give_back));
                }
            }
            // Its top stays where it is, as Raycast's and Spotlight's grow and shrink downwards.
            Event::UserEvent(Told::Resize(width, height)) => {
                if let Some(id) = native {
                    let width = width.map(|width| width.clamp(1.0, MOST.width) as f32);
                    let height = height.unwrap_or(SIZE.height).clamp(1.0, MOST.height) as f32;
                    on_native(move |windows| windows.resize(id, width, height));
                }
            }
            // The device's session started again, or its page was updated; its servers say it's behind them, or talk
            // with it again after they did: it opens again, as what it is now, and shows again if it was showing.
            Event::UserEvent(told @ (Told::Reload | Told::Behind | Told::Talks)) if matches!(told, Told::Reload) || matches!(told, Told::Behind) != (shows == Shows::Behind) => {
                // Signed in just now, its person sees their Home Screen, as an app opens to it once they're in: the
                // sign-in page is gone already, as every window is as the session starts again (`Shell::reload`).
                wanted |= shows == Shows::SignIn;
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
                match told {
                    Told::Behind => crate::spawn(async {
                        let prepared = PREPARED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        tell(Told::Native(crate::native::launcher_instead(Instead::Behind).await, Shows::Behind, prepared))
                    }),
                    _ => crate::spawn(prepare()),
                }
            }
            // The Home Screen opens a link to an app in that app's window; until it's open, the link waits for it.
            Event::UserEvent(Told::Open(link)) if native.is_some() && shows == Shows::Home => crate::spawn(crate::native::go(link, None)),
            Event::UserEvent(Told::Open(link)) => *OPENED.lock().expect("nothing panics holding it") = Some(link),
            Event::UserEvent(Told::Restart) => match native {
                Some(id) => on_native(move |windows| restart(windows.showing(id))),
                None => restart(false),
            },
            Event::UserEvent(Told::Quit) => *flow = ControlFlow::Exit,
            // The shortcut is let go of before it ends, which the system has done once this returns, so the new
            // program, which takes it as this one ends, finds it free.
            Event::UserEvent(Told::Ending) => {
                if let Some((keeper, key)) = _shortcut.take() {
                    let _ = keeper.unregister(key);
                }
                std::process::exit(0)
            }
            Event::UserEvent(Told::Handed) => {
                if let Err(error) = hosts::serve(&home) {
                    eprintln!("its channel didn't start: {error:#}");
                }
                _shortcut = shortcut(&home);
            }
            _ => {}
        }
    })
}

/// The system's own alert, as an app shows what it can't go on from: rfd's (NSAlert, Windows' message box), and
/// GTK's on Linux, as rfd's GTK is another GTK's than the one tao runs.
fn alert(said: &str) {
    #[cfg(any(target_os = "macos", windows))]
    rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title(crate::NAME).set_description(said).show();
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        let alert = gtk::MessageDialog::new(None::<&gtk::Window>, gtk::DialogFlags::MODAL, gtk::MessageType::Error, gtk::ButtonsType::Ok, said);
        alert.set_title(crate::NAME);
        alert.run();
        alert.close();
    }
}

/// The launcher's shortcut, which shows it from anywhere, or puts it away. It's ⌥Space on a Mac, as Raycast suggests,
/// which macOS leaves free (Spotlight is ⌘Space, the input source ⌃Space); on Windows and Linux Alt+Space is the
/// window menu's, and Super+Space the input source's, so it's Super+Alt+Space there. A file `shortcut` in the client's
/// folder says another, as `alt+Space` (global-hotkey's words), or nothing for none; Linux's is X11's, and a Wayland
/// session has none. → what keeps it, for as long as the app runs.
fn shortcut(home: &std::path::Path) -> Option<(global_hotkey::GlobalHotKeyManager, global_hotkey::hotkey::HotKey)> {
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
    Some((keeper, key))
}

/// Whether the program before it started this one as it updated (`restart`).
static RESTARTED: AtomicBool = AtomicBool::new(false);

/// Starts the app again on the build it was just updated to: the app windows that are open open again in it, and the
/// launcher shows again if it was showing. This program ends only once the new one says it's up (`opened`), on its
/// standard output, and the new one takes what only one may have once its standard input, held here, closes as this
/// one ends. One that ends before it's up leaves this one running, Telemetry hearing why, and the next update tries
/// again: so a build that can't start never leaves the device with no app (20 Nov).
fn restart(shown: bool) {
    use std::io::BufRead;
    // Asked again before the new one's up, it starts one program, not two (two did, 19 Nov).
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let reopen = serde_json::to_string(&hosts::open_now()).unwrap_or_default();
    let mut command = match cli::serving::program() {
        Ok(program) => std::process::Command::new(program),
        Err(error) => {
            STARTED.store(false, Ordering::Relaxed);
            return eprintln!("couldn't start again: {error}");
        }
    };
    command.env("ONE_RESTARTED", "1").env("ONE_REOPEN", reopen).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
    if shown {
        command.env_remove("ONE_HIDDEN");
    } else {
        command.env("ONE_HIDDEN", "1");
    }
    let mut started = match command.spawn() {
        Ok(started) => started,
        Err(error) => {
            STARTED.store(false, Ordering::Relaxed);
            return eprintln!("couldn't start again: {error}");
        }
    };
    let (handing, said) = (started.stdin.take(), started.stdout.take());
    std::thread::spawn(move || {
        let up = said.is_some_and(|said| std::io::BufReader::new(said).lines().map_while(Result::ok).any(|line| line == "ready"));
        if up {
            // Held open until this program ends, which closes it: the new one takes over then, once this one has let
            // go of its shortcut (`Told::Ending`).
            std::mem::forget(handing);
            return tell(Told::Ending);
        }
        drop(handing);
        let ended = started.wait().map_or_else(|error| error.to_string(), |status| status.to_string());
        crate::updater::report("one.client.update.failed", json!({"error": format!("its new build ended before it was up: {ended}")}));
        STARTED.store(false, Ordering::Relaxed);
    });
}

/// The app is up: its window made and its client opened. Started by the program before it as it updated (`restart`),
/// it says so to that one, which ends, and then takes what only one may have (`Told::Handed`).
pub(crate) fn opened() {
    use std::io::{Read, Write};
    if !RESTARTED.swap(false, Ordering::Relaxed) {
        return;
    }
    let said = std::io::stdout().write_all(b"ready\n").and_then(|()| std::io::stdout().flush());
    // Its standard output goes nowhere from now on, as the program it told is ending.
    if let Ok(nowhere) = std::fs::OpenOptions::new().write(true).open(if cfg!(windows) { "NUL" } else { "/dev/null" }) {
        #[cfg(unix)]
        unsafe {
            libc::dup2(std::os::fd::AsRawFd::as_raw_fd(&nowhere), libc::STDOUT_FILENO)
        };
        #[cfg(windows)]
        {
            let handle = windows::Win32::Foundation::HANDLE(std::os::windows::io::IntoRawHandle::into_raw_handle(nowhere));
            let _ = unsafe { windows::Win32::System::Console::SetStdHandle(windows::Win32::System::Console::STD_OUTPUT_HANDLE, handle) };
        }
    }
    if said.is_err() {
        return tell(Told::Handed);
    }
    std::thread::spawn(|| {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        tell(Told::Handed)
    });
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
            crate::NAME,
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
    for (name, value) in [("DisplayName", crate::NAME.to_owned()), ("IconUri", icon.display().to_string())] {
        let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(&key), &HSTRING::from(name), REG_SZ.0, Some(value.as_ptr().cast()), (value.len() * 2) as u32)
        }
        .ok()?;
    }
    Ok(())
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
        crate::NAME,
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
    let open = MenuItem::with_id("open", format!("Open {}", crate::NAME), true, None);
    // Settings…, the Settings app's window, as Raycast's menu has its own settings.
    let settings = MenuItem::with_id("settings", "Settings…", true, None);
    let quit = MenuItem::with_id("quit", format!("Quit {}", crate::NAME), true, None);
    let menu = Menu::with_items(&[&open, &settings, &PredefinedMenuItem::separator(), &quit]).ok()?;
    #[cfg(windows)]
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            tell(Told::Toggle(std::time::Instant::now(), None));
        }
    }));
    let ring = TrayIconBuilder::new().with_icon(icon).with_icon_as_template(cfg!(target_os = "macos")).with_menu(Box::new(menu)).with_menu_on_left_click(cfg!(target_os = "macos")).with_tooltip(crate::NAME).build();
    ring.map_err(|error| eprintln!("the menu bar has no ring: {error}")).ok()
}
