//! One app's own window on a computer, in a process of its own, which the system shows as that app: its own name and
//! icon in the Dock or taskbar, in Cmd-Tab or Alt-Tab, and on a Mac its own menu bar. It's this program started with
//! `ONE_APP` (and `ONE_SUBJECT` for a group or the network), as the app's window starts it (`hosts`), or as the
//! system opens the app from its own shortcut (`shortcuts`).
//!
//! It's only a window and a web view (tao and wry, which Tauri is built on), so it's light: it has no client of its
//! own. Its page is the app window's own (`page/`), which shows the one app, and asks what it asks of the app's
//! window, over the local channel (`hosts`), which answers it as it answers its Home Screen. The app's window is
//! started first if it isn't running. Closing this window ends this process, and the app's window ending ends it too.
//! Minimised or hidden a while, it discards its page (`discard`), which loads again as it's brought back, at the
//! link it was last opened at.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use data_encoding::BASE64;
use serde_json::{Value, json};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Theme, WindowBuilder};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use wry::http::{Request, Response, StatusCode, header};
use wry::{RequestAsyncResponder, WebViewId};

use crate::discard::{Discarding, Page};

/// What the window's page reaches of Tauri's in an app's own window, where there's no Tauri (`invoke` and `listen`),
/// by a key no app's frame knows, which replaces `__KEY__`.
pub(crate) const BRIDGE: &str = include_str!("host-initialization-script.js");

/// What the window is told to do on its own thread, where its window and web view are.
enum Told {
    /// Runs script in its page.
    Run(String),
    /// Comes forward, as the app was opened again.
    Front,
    /// Light or dark, or as the device is, for its title bar.
    Theme(Option<Theme>),
    /// Turns the web view's back and forward swipes on or off.
    Gestures(bool),
    /// Moves with the pointer, held down on the page's top, where a Mac's window has no title band.
    Drag,
    /// Does what a double-click on the title bar does (`hosts::double_clicked`).
    DoubleClick,
    /// Brings the Home Screen forward, its window kept open (the page's Home and Window → Home Screen).
    Home,
    /// Whether its page is on screen.
    Shown(bool),
    Quit,
}

/// Something asked over the channel that hasn't been answered: the page's ask, by its id there, and whether it's
/// `start`, or a file a frame loads.
enum Asked {
    Page(Value, bool),
    File(RequestAsyncResponder),
}

/// Who this window is: its app and who it's for, and where it was opened at.
#[derive(Clone)]
struct Opened {
    entrypoint: String,
    subject: Option<String>,
    link: String,
}

/// When this process started, as near as it knows: how long its window took to open counts from here.
fn started() -> std::time::Instant {
    static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *STARTED.get_or_init(std::time::Instant::now)
}

/// Starts the app's window and runs until it's closed: `entrypoint`, for `subject`, at `link`.
pub fn run(entrypoint: String, subject: Option<String>, link: String) -> ExitCode {
    started();
    let opened = Opened { entrypoint, subject, link };
    // The name the system knows it by, as the app's shortcut says it too (`shortcuts`): Windows groups its windows
    // under it in the taskbar.
    let class = crate::shortcuts::app_id(&opened.entrypoint, opened.subject.as_deref());
    #[cfg(windows)]
    if let Err(error) = unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(&class)) } {
        eprintln!("this window's AppUserModelID isn't its app's: {error}");
    }
    #[cfg(target_os = "linux")]
    gtk::glib::set_prgname(Some(class.as_str()));
    match window(opened, &class) {
        Ok(never) => never,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Asks the One App, started if it isn't running, to open `opened`'s window, and waits for its answer.
#[cfg(target_os = "linux")]
fn hand_over(home: &std::path::Path, opened: &Opened) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    runtime.block_on(async {
        let stream = connect(&crate::hosts::address(home)).await?;
        let (read, mut write) = tokio::io::split(stream);
        let ask = json!({"id": "open", "command": "open_window", "args": {"entrypoint": opened.entrypoint, "subject": opened.subject, "link": opened.link}});
        write.write_all(format!("{ask}\n").as_bytes()).await?;
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await? {
            let answer: Value = serde_json::from_str(&line).unwrap_or_default();
            if answer["id"] == "open" {
                return match answer.get("error") {
                    Some(error) => Err(anyhow::anyhow!("{error}")),
                    None => Ok(()),
                };
            }
        }
        anyhow::bail!("the One App ended")
    })
}

fn window(opened: Opened, class: &str) -> anyhow::Result<ExitCode> {
    let home = ::client::home()?;
    // Started by the system, from the app's own shortcut, it asks the One App to open it, which draws a strict app's
    // window itself (`strict`) and starts a web view's, as this, for any other. Where that can't be, it's a web view.
    #[cfg(target_os = "linux")]
    if std::env::var_os("ONE_WEB_VIEW").is_none() {
        match hand_over(&home, &opened) {
            Ok(()) => return Ok(ExitCode::SUCCESS),
            Err(error) => eprintln!("{} opens here: {error:#}", opened.entrypoint),
        }
    }
    crate::page::load(&home);
    let event_loop = EventLoopBuilder::<Told>::with_user_event().build();
    // The window's class, which a Linux desktop matches to the app's entry (`StartupWMClass`), once GTK has started.
    #[cfg(target_os = "linux")]
    gtk::gdk::set_program_class(class);
    #[cfg(not(target_os = "linux"))]
    let _ = class;
    let proxy = event_loop.create_proxy();
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    // Where the window's title bar is over its page (`title_bar`), which it's told as it starts and as that changes.
    let title_bar: Arc<Mutex<Option<crate::title_bar::Area>>> = Arc::default();
    let link = Arc::new(Mutex::new(opened.link.clone()));
    let to = channel(&home, opened.clone(), link.clone(), proxy.clone(), asked.clone(), title_bar.clone())?;

    let name = opened.entrypoint.split('/').next().unwrap_or_default().to_owned();
    #[cfg(target_os = "macos")]
    let menu = menu(&name, proxy.clone());
    // A Linux window is shown once it has its header bar (`title_bar`).
    let window = WindowBuilder::new()
        .with_title(&name)
        .with_inner_size(tao::dpi::LogicalSize::new(1024.0, 720.0))
        .with_visible(cfg!(not(target_os = "linux")));
    // No title band, as the Home Screen's window has none (`titleBar`), the traffic lights kept.
    #[cfg(target_os = "macos")]
    let window = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        window.with_title_hidden(true).with_titlebar_transparent(true).with_fullsize_content_view(true)
    };
    let window = window.build(&event_loop)?;
    // It opens where it was last, as a strict app's window does (`strict::platform`), so one opened again as the app
    // starts again on a new build is where it was (`updater`).
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowExtMacOS;
        crate::hosts::remember_frame(window.ns_window(), &format!("one {}", json!({"entrypoint": opened.entrypoint, "subject": opened.subject})));
    }

    #[cfg(target_os = "macos")]
    let home_screen = ::client::NAME;
    let pages = Pages { entrypoint: opened.entrypoint, to: to.clone(), asked, proxy: proxy.clone(), next: Arc::default(), loaded: 0 };
    // Windows' WebView2 keeps its data in the launcher's folder, so one browser process, with its GPU and network
    // processes, serves the launcher and every app window rather than each starting its own.
    #[cfg(windows)]
    let data_directory = crate::shortcuts::local_data_dir().ok();
    #[cfg(not(windows))]
    let data_directory = None;
    let mut page = Discarding::new(&window, pages, data_directory)?;
    #[cfg(target_os = "linux")]
    keys(&window, proxy.clone());

    let (area, told) = (title_bar.clone(), proxy.clone());
    let title_bar_changed = move |changed: crate::title_bar::Area| {
        *area.lock().expect("nothing panics holding it") = Some(changed);
        let _ = told.send_event(Told::Run(format!("window.__ONE_HOST__.event(\"title_bar\", {})", Value::from(changed))));
    };
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        crate::title_bar::extend(window.hwnd(), title_bar_changed)?;
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewExtUnix;
        let webview = page.webview().expect("it's just loaded").webview();
        let overlay = crate::title_bar::header_bar(window.gtk_window().upcast_ref(), &webview, title_bar_changed);
        // A page loaded again goes under the header bar too.
        page.load_in(overlay.upcast());
        window.set_visible(true);
    }
    #[cfg(target_os = "macos")]
    title_bar_changed(crate::title_bar::MAC);

    event_loop.run(move |event, _, flow| {
        #[cfg(target_os = "macos")]
        let _ = &menu;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(Told::Quit) => {
                *flow = ControlFlow::Exit
            }
            Event::WindowEvent { event, .. } => page.heard(&window, &event),
            Event::UserEvent(Told::Run(script)) => match script.strip_prefix("/*title*/") {
                Some(title) => window.set_title(&serde_json::from_str::<String>(title).unwrap_or_default()),
                None if !script.is_empty() => {
                    if let Some(webview) = page.webview() {
                        drop(webview.evaluate_script(&script))
                    }
                }
                None => {}
            },
            Event::UserEvent(Told::Front) => {
                window.set_minimized(false);
                window.set_visible(true);
                window.set_focus();
                page.shown(&window, true);
            }
            Event::UserEvent(Told::Theme(theme)) => window.set_theme(theme),
            Event::UserEvent(Told::Gestures(on)) => {
                if let Some(webview) = page.webview() {
                    gestures(webview, on)
                }
            }
            Event::UserEvent(Told::Shown(on)) => page.shown(&window, on),
            Event::UserEvent(Told::Drag) => drop(window.drag_window()),
            #[cfg(target_os = "macos")]
            Event::UserEvent(Told::DoubleClick) => {
                use tao::platform::macos::WindowExtMacOS;
                crate::hosts::double_clicked(window.ns_window())
            }
            #[cfg(target_os = "linux")]
            Event::UserEvent(Told::DoubleClick) => {
                use gtk::prelude::*;
                use tao::platform::unix::WindowExtUnix;
                crate::title_bar::double_clicked(window.gtk_window().upcast_ref())
            }
            Event::UserEvent(Told::Home) => {
                // The Home Screen is the One App's, which comes in front only where this app lets it.
                #[cfg(target_os = "macos")]
                crate::hosts::yield_activation(home_screen);
                let _ = to.send(json!({"id": "front", "command": "front"}));
            }
            _ => {}
        }
        if *flow != ControlFlow::Exit {
            *flow = page.discard_due(&window);
        }
    })
}

/// What the window's page is made from, each time it's loaded.
struct Pages {
    entrypoint: String,
    to: UnboundedSender<Value>,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    proxy: EventLoopProxy<Told>,
    next: Arc<AtomicU64>,
    /// How many times it's been loaded, which names what each asks, so an answer to a page that's gone is dropped.
    loaded: u64,
}

impl Page for Pages {
    /// The app's page, made new.
    fn make<'a>(&mut self, builder: wry::WebViewBuilder<'a>) -> wry::WebViewBuilder<'a> {
        self.discarded();
        self.loaded += 1;
        let key = format!("{:016x}{:016x}", random(), random());
        // The page's own files, which are this program's, as the launcher serves them.
        let own = |_: WebViewId, request: Request<Vec<u8>>| crate::page::respond(request.uri().path());

        // An app's files, which the app's window has (`Views::file`).
        let (files, files_to, next, loaded) = (self.asked.clone(), self.to.clone(), self.next.clone(), self.loaded);
        let apps_file = move |_: WebViewId, request: Request<Vec<u8>>, responder: RequestAsyncResponder| {
            let id = format!("f{loaded}.{}", next.fetch_add(1, Ordering::Relaxed));
            files.lock().expect("nothing panics holding it").insert(id.clone(), Asked::File(responder));
            let _ = files_to.send(json!({"id": id, "file": request.uri().to_string()}));
        };

        // What the page asks: what's this window's own is answered here, and the rest by the app's window.
        let (pages, page_to, page_key, told) = (self.asked.clone(), self.to.clone(), key.clone(), self.proxy.clone());
        let started = self.entrypoint.clone();
        let ipc = move |request: Request<String>| {
            let Ok(message) = serde_json::from_str::<Value>(request.body()) else { return };
            if message["key"] != page_key.as_str() {
                return;
            }
            let (id, command, args) =
                (message["id"].clone(), message["command"].as_str().unwrap_or_default(), &message["args"]);
            let mine = match command {
                "theme" => Some(Told::Theme(match args["theme"].as_str() {
                    Some("dark") => Some(Theme::Dark),
                    Some("light") => Some(Theme::Light),
                    _ => None,
                })),
                "navigation_gestures" => Some(Told::Gestures(args["on"].as_bool() == Some(true))),
                "plugin:window|start_dragging" => Some(Told::Drag),
                "title_bar_double_click" => Some(Told::DoubleClick),
                "front" => Some(Told::Home),
                // Whether it's on screen, which the app's window hears too.
                "visible" => Some(Told::Shown(args["onScreen"].as_bool() == Some(true))),
                // A phone's status bar and haptics, which a computer has neither of.
                "status_bar" => Some(Told::Run(String::new())),
                command if command.starts_with("plugin:") => Some(Told::Run(String::new())),
                _ => None,
            };
            // Colour Mode is the device's, and what's on screen counts there, so the app's window is told them too.
            let shared = command == "theme" || command == "visible";
            match mine {
                Some(mine) if !shared => {
                    let _ = told.send_event(mine);
                    let _ = told.send_event(Told::Run(answer(&id, Ok(Value::Null))));
                }
                mine => {
                    if let Some(mine) = mine {
                        let _ = told.send_event(mine);
                    }
                    let at = format!("p{loaded}.{id}");
                    pages
                        .lock()
                        .expect("nothing panics holding it")
                        .insert(at.clone(), Asked::Page(id, command == "start"));
                    // `start` is told which app this window is, whose name it answers with.
                    let args = if command == "start" { json!({"entrypoint": started}) } else { args.clone() };
                    let _ = page_to.send(json!({"id": at, "command": command, "args": args}));
                }
            }
        };

        // Anywhere but its own pages and its apps' is the device's own browser's to open.
        let browser = self.to.clone();
        let stays = move |to: String| {
            let own = ["tauri:", "oneapp:", "about:", "data:", "blob:"].iter().any(|scheme| to.starts_with(scheme))
                || url::Url::parse(&to)
                    .is_ok_and(|to| to.host_str().is_some_and(|host| host.ends_with(".localhost")));
            if !own {
                let _ = browser.send(json!({"id": "browser", "command": "open_url", "args": {"url": to}}));
            }
            own
        };

        let titled = self.proxy.clone();
        let builder = builder
            .with_custom_protocol("tauri".into(), own)
            .with_asynchronous_custom_protocol("oneapp".into(), apps_file)
            .with_ipc_handler(ipc)
            .with_initialization_script_for_main_only(BRIDGE.replace("__KEY__", &key), true)
            .with_navigation_handler(stays)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_document_title_changed_handler(move |title| {
                drop(titled.send_event(Told::Run(format!("/*title*/{}", Value::from(title)))))
            })
            .with_back_forward_navigation_gestures(true)
            .with_devtools(cfg!(debug_assertions))
            .with_url(crate::HOME);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let builder = builder.with_initialization_script_for_main_only(crate::INITIALIZATION_SCRIPT, false);
        builder
    }

    /// Forgets what the page that's gone asked and hasn't been answered, which no one is there to hear.
    fn discarded(&mut self) {
        self.asked.lock().expect("nothing panics holding it").clear();
    }
}

/// Connects to the app's window, starting it if it isn't running, says hello, and talks with it on a thread of its
/// own. → where to send it a message.
fn channel(
    home: &std::path::Path,
    opened: Opened,
    link: Arc<Mutex<String>>,
    proxy: EventLoopProxy<Told>,
    asked: Arc<Mutex<HashMap<String, Asked>>>,
    title_bar: Arc<Mutex<Option<crate::title_bar::Area>>>,
) -> anyhow::Result<UnboundedSender<Value>> {
    let address = crate::hosts::address(home);
    let (to, mut out) = unbounded_channel::<Value>();
    let _ = to.send(json!({"hello": {"entrypoint": opened.entrypoint, "subject": opened.subject, "link": opened.link}}));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    let stream = runtime.block_on(connect(&address))?;
    let ended = proxy.clone();
    std::thread::spawn(move || {
        runtime.block_on(async move {
            let (read, mut write) = tokio::io::split(stream);
            tokio::spawn(async move {
                while let Some(message) = out.recv().await {
                    let mut line = message.to_string();
                    line.push('\n');
                    if write.write_all(line.as_bytes()).await.is_err() {
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
                    // Opened again, it comes forward, and loads at that link if its page is discarded.
                    if event == "open" {
                        if let Some(opens) = message["payload"].as_str() {
                            *link.lock().expect("nothing panics holding it") = opens.to_owned();
                        }
                        let _ = proxy.send_event(Told::Front);
                    }
                    let _ = proxy.send_event(Told::Run(format!("window.__ONE_HOST__.event({}, {})", Value::from(event), message["payload"])));
                    continue;
                }
                let Some(id) = message["id"].as_str() else { continue };
                let Some(asked) = asked.lock().expect("nothing panics holding it").remove(id) else { continue };
                let result = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
                match asked {
                    Asked::Page(id, start) => {
                        // Where the app's window is with its network, and this window's own app.
                        let result = result.map(|mut started| {
                            if start {
                                started["open"] = json!(*link.lock().expect("nothing panics holding it"));
                                started["window"] = json!({"entrypoint": opened.entrypoint, "subject": opened.subject, "name": started["name"]});
                                if let Some(area) = *title_bar.lock().expect("nothing panics holding it") {
                                    started["titleBar"] = area.into();
                                }
                            }
                            started
                        });
                        let _ = proxy.send_event(Told::Run(answer(&id, result)));
                    }
                    Asked::File(responder) => responder.respond(app_file(match result {
                        Ok(file) => Ok((
                            file["kind"].as_str().unwrap_or_default().to_owned(),
                            BASE64.decode(file["body"].as_str().unwrap_or_default().as_bytes()).unwrap_or_default(),
                        )),
                        Err(error) => Err(error.as_str().unwrap_or_default().to_owned()),
                    })),
                }
            }
        });
        // The app's window ended, or said this one should.
        let _ = ended.send_event(Told::Quit);
    });
    Ok(to)
}

/// The channel to the app's window. One that isn't running is started, with its Home Screen hidden, as only this app
/// was opened, and connected to once it's listening: a program starting is the outside world, so this asks again
/// every tenth of a second for up to half a minute.
pub(crate) async fn connect(address: &str) -> anyhow::Result<impl AsyncRead + AsyncWrite + Send + 'static> {
    if let Ok(stream) = open(address).await {
        return Ok(stream);
    }
    let program = std::env::var_os("ONE_PROGRAM").map(std::path::PathBuf::from).map_or_else(std::env::current_exe, Ok)?;
    std::process::Command::new(program)
        .env_remove("ONE_APP")
        .env_remove("ONE_SUBJECT")
        .env_remove("ONE_OPEN")
        .env("ONE_HIDDEN", "1")
        .stdin(std::process::Stdio::null())
        .spawn()?;
    for _ in 0..300 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(stream) = open(address).await {
            return Ok(stream);
        }
    }
    anyhow::bail!("the One App didn't start")
}

#[cfg(unix)]
async fn open(address: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(address).await
}

#[cfg(windows)]
async fn open(address: &str) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(address)
}

/// An app's file, or why there isn't one, as the web view's answer for it, with what an app's page may load.
pub(crate) fn app_file(file: Result<(String, Vec<u8>), String>) -> Response<std::borrow::Cow<'static, [u8]>> {
    let response = match file {
        Ok((kind, file)) => Response::builder()
            .header(header::CONTENT_TYPE, kind)
            .header(header::CONTENT_SECURITY_POLICY, crate::APP_MAY_LOAD)
            .header(header::CACHE_CONTROL, "no-store")
            .body(file.into()),
        Err(error) => Response::builder().status(StatusCode::NOT_FOUND).body(error.into_bytes().into()),
    };
    response.expect("these headers are valid")
}

/// The script that answers the page's ask `id`.
pub(crate) fn answer(id: &Value, result: Result<Value, Value>) -> String {
    let (ok, value) = match result {
        Ok(value) => (true, value),
        Err(value) => (false, value),
    };
    format!("window.__ONE_HOST__.answer({id}, {ok}, {value})")
}

/// A random number, from the hasher the standard library seeds randomly.
pub(crate) fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

/// Ctrl-W closes the window and Ctrl-Q quits, as they do in a Linux app's (`linux_keys`), both ending the app's own
/// window here.
#[cfg(target_os = "linux")]
fn keys(window: &tao::window::Window, proxy: EventLoopProxy<Told>) {
    use gtk::gdk::{ModifierType, keys::constants as key};
    use gtk::glib::Propagation;
    use gtk::prelude::*;
    use tao::platform::unix::WindowExtUnix;
    window.gtk_window().connect_key_press_event(move |_, pressed| {
        let held = pressed.state() & (ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK | ModifierType::MOD1_MASK | ModifierType::SUPER_MASK);
        if held != ModifierType::CONTROL_MASK || ![key::q, key::w].contains(&pressed.keyval().to_lower()) {
            return Propagation::Proceed;
        }
        let _ = proxy.send_event(Told::Quit);
        Propagation::Stop
    });
}

/// The web view's own back and forward swipes, as the Home Screen's window has them (`navigation_gestures`).
fn gestures(webview: &wry::WebView, on: bool) {
    #[cfg(target_os = "macos")]
    unsafe {
        use wry::WebViewExtMacOS;
        let view = webview.webview();
        let () = objc2::msg_send![&*view, setAllowsBackForwardNavigationGestures: on];
    }
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{SettingsExt, WebViewExt};
        use wry::WebViewExtUnix;
        if let Some(settings) = webview.webview().settings() {
            settings.set_enable_back_forward_navigation_gestures(on)
        }
    }
    #[cfg(windows)]
    let _ = (webview, on);
}

/// A Mac app's menu bar, named after the app: its app menu (About, Hide, Quit), Edit, which a web view's copy and
/// paste keys need, and Window, which has Home Screen too, at ⇧⌘H, as Finder's and Safari's Home are.
#[cfg(target_os = "macos")]
fn menu(name: &str, proxy: EventLoopProxy<Told>) -> muda::Menu {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    let menu = Menu::new();
    let app = Submenu::with_items(
        name,
        true,
        &[
            &PredefinedMenuItem::about(None, None),
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
    let home = MenuItem::with_id("home-screen", "Home Screen", true, Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyH)));
    let proxy = Mutex::new(proxy);
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == "home-screen" {
            let _ = proxy.lock().expect("nothing panics holding it").send_event(Told::Home);
        }
    }));
    let window = Submenu::with_items(
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::close_window(None), &PredefinedMenuItem::separator(), &home],
    );
    if let (Ok(app), Ok(edit), Ok(window)) = (app, edit, window) {
        let _ = menu.append_items(&[&app, &edit, &window]);
        menu.init_for_nsapp();
    }
    menu
}
