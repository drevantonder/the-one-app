//! One app's own window on a computer, in a process of its own, which the system shows as that app: its own name and
//! icon in the Dock or taskbar, in Cmd-Tab or Alt-Tab, and on a Mac its own menu bar. It's this program started with
//! `ONE_APP` (and `ONE_SUBJECT` for a group or the network), as the app's window starts it (`hosts`), or as the
//! system opens the app from its own shortcut (`shortcuts`).
//!
//! It's only a window and a web view (tao and wry, which Tauri is built on), so it's light: it has no client of its
//! own. Its page is the app window's own (`page/`), which shows the one app, and asks what it asks of the app's
//! window, over the local channel (`hosts`), which answers it as it answers its Home Screen. The app's window is
//! started first if it isn't running. Closing this window ends this process, and the app's window ending ends it too.

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
use wry::{RequestAsyncResponder, WebViewBuilder, WebViewId};

/// What the window's page reaches of Tauri's in an app's own window, where there's no Tauri (`invoke` and `listen`),
/// by a key no app's frame knows, which replaces `__KEY__`.
const BRIDGE: &str = include_str!("host-initialization-script.js");

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

/// Starts the app's window and runs until it's closed: `entrypoint`, for `subject`, at `link`.
pub fn run(entrypoint: String, subject: Option<String>, link: String) -> ExitCode {
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

fn window(opened: Opened, class: &str) -> anyhow::Result<ExitCode> {
    let home = ::client::home()?;
    let event_loop = EventLoopBuilder::<Told>::with_user_event().build();
    // The window's class, which a Linux desktop matches to the app's entry (`StartupWMClass`), once GTK has started.
    #[cfg(target_os = "linux")]
    gtk::gdk::set_program_class(class);
    #[cfg(not(target_os = "linux"))]
    let _ = class;
    let proxy = event_loop.create_proxy();
    let key = format!("{:016x}{:016x}", random(), random());
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let to = channel(&home, opened.clone(), proxy.clone(), asked.clone())?;

    let name = opened.entrypoint.split('/').next().unwrap_or_default().to_owned();
    #[cfg(target_os = "macos")]
    let menu = menu(&name, proxy.clone());
    let window = WindowBuilder::new().with_title(&name).with_inner_size(tao::dpi::LogicalSize::new(1024.0, 720.0));
    // No title band, as the Home Screen's window has none (`titleBar`), the traffic lights kept.
    #[cfg(target_os = "macos")]
    let window = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        window.with_title_hidden(true).with_titlebar_transparent(true).with_fullsize_content_view(true)
    };
    let window = window.build(&event_loop)?;

    // The page's own files, which are this program's, as the app's window serves them.
    let context: &'static tauri::Context<tauri::Wry> = Box::leak(Box::new(crate::context()));
    #[cfg(target_os = "macos")]
    let home_screen = context.config().identifier.clone();
    let csp = context.config().app.security.csp.as_ref().map(ToString::to_string).unwrap_or_default();
    let assets = context.assets();
    let own = move |_: WebViewId, request: Request<Vec<u8>>| {
        let path = request.uri().path().to_owned();
        let response = match assets.get(&path.as_str().into()) {
            Some(file) => Response::builder()
                .header(header::CONTENT_TYPE, kind(&path))
                .header(header::CONTENT_SECURITY_POLICY, &csp)
                .body(file.into_owned().into()),
            None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new().into()),
        };
        response.expect("these headers are valid")
    };

    // An app's files, which the app's window has (`Views::file`).
    let (files, files_to, next) = (asked.clone(), to.clone(), Arc::new(AtomicU64::new(0)));
    let apps_file = move |_: WebViewId, request: Request<Vec<u8>>, responder: RequestAsyncResponder| {
        let id = format!("f{}", next.fetch_add(1, Ordering::Relaxed));
        files.lock().expect("nothing panics holding it").insert(id.clone(), Asked::File(responder));
        let _ = files_to.send(json!({"id": id, "file": request.uri().to_string()}));
    };

    // What the page asks: what's this window's own is answered here, and the rest by the app's window.
    let (pages, page_to, page_key, told) = (asked.clone(), to.clone(), key.clone(), proxy.clone());
    let started = opened.entrypoint.clone();
    let ipc = move |request: Request<String>| {
        let Ok(message) = serde_json::from_str::<Value>(request.body()) else { return };
        if message["key"] != page_key.as_str() {
            return;
        }
        let (id, command, args) = (message["id"].clone(), message["command"].as_str().unwrap_or_default(), &message["args"]);
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
            // A phone's status bar and haptics, which a computer has neither of.
            "status_bar" => Some(Told::Run(String::new())),
            command if command.starts_with("plugin:") => Some(Told::Run(String::new())),
            _ => None,
        };
        // Colour Mode is the device's, so the app's window is told it too.
        let shared = command == "theme";
        match mine {
            Some(mine) if !shared => {
                let _ = told.send_event(mine);
                let _ = told.send_event(Told::Run(answer(&id, Ok(Value::Null))));
            }
            mine => {
                if let Some(mine) = mine {
                    let _ = told.send_event(mine);
                }
                let at = format!("p{id}");
                pages.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Page(id, command == "start"));
                // `start` is told which app this window is, whose name it answers with.
                let args = if command == "start" { json!({"entrypoint": started}) } else { args.clone() };
                let _ = page_to.send(json!({"id": at, "command": command, "args": args}));
            }
        }
    };

    // Anywhere but its own pages and its apps' is the device's own browser's to open.
    let browser = to.clone();
    let stays = move |to: String| {
        let own = ["tauri:", "oneapp:", "about:", "data:", "blob:"].iter().any(|scheme| to.starts_with(scheme))
            || tauri::Url::parse(&to).is_ok_and(|to| to.host_str().is_some_and(|host| host.ends_with(".localhost")));
        if !own {
            let _ = browser.send(json!({"id": "browser", "command": "open_url", "args": {"url": to}}));
        }
        own
    };

    let titled = proxy.clone();
    // WebDriver drives it as it does Tauri's windows, which tauri-driver says by this.
    #[cfg(target_os = "linux")]
    let builder = {
        let mut context = wry::WebContext::default();
        context.set_allows_automation(std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true"));
        WebViewBuilder::new_with_web_context(Box::leak(Box::new(context)))
    };
    #[cfg(not(target_os = "linux"))]
    let builder = WebViewBuilder::new();
    let builder = builder
        .with_custom_protocol("tauri".into(), own)
        .with_asynchronous_custom_protocol("oneapp".into(), apps_file)
        .with_ipc_handler(ipc)
        .with_initialization_script_for_main_only(BRIDGE.replace("__KEY__", &key), true)
        .with_navigation_handler(stays)
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_document_title_changed_handler(move |title| drop(titled.send_event(Told::Run(format!("/*title*/{}", Value::from(title))))))
        .with_back_forward_navigation_gestures(true)
        .with_devtools(cfg!(debug_assertions))
        .with_url(crate::HOME);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let builder = builder.with_initialization_script_for_main_only(crate::INITIALIZATION_SCRIPT, false);
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window)?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        keys(&window, proxy.clone());
        builder.build_gtk(window.default_vbox().expect("tao gives a window a box"))?
    };

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        #[cfg(target_os = "macos")]
        let _ = &menu;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(Told::Quit) => *flow = ControlFlow::Exit,
            Event::UserEvent(Told::Run(script)) => match script.strip_prefix("/*title*/") {
                Some(title) => window.set_title(&serde_json::from_str::<String>(title).unwrap_or_default()),
                None if !script.is_empty() => drop(webview.evaluate_script(&script)),
                None => {}
            },
            Event::UserEvent(Told::Front) => {
                window.set_minimized(false);
                window.set_visible(true);
                window.set_focus();
            }
            Event::UserEvent(Told::Theme(theme)) => window.set_theme(theme),
            Event::UserEvent(Told::Gestures(on)) => gestures(&webview, on),
            Event::UserEvent(Told::Drag) => drop(window.drag_window()),
            #[cfg(target_os = "macos")]
            Event::UserEvent(Told::DoubleClick) => {
                use tao::platform::macos::WindowExtMacOS;
                crate::hosts::double_clicked(window.ns_window())
            }
            Event::UserEvent(Told::Home) => {
                // The Home Screen is The One App's, which comes in front only where this app lets it.
                #[cfg(target_os = "macos")]
                crate::hosts::yield_activation(&home_screen);
                let _ = to.send(json!({"id": "front", "command": "front"}));
            }
            _ => {}
        }
    })
}

/// Connects to the app's window, starting it if it isn't running, says hello, and talks with it on a thread of its
/// own. → where to send it a message.
fn channel(home: &std::path::Path, opened: Opened, proxy: EventLoopProxy<Told>, asked: Arc<Mutex<HashMap<String, Asked>>>) -> anyhow::Result<UnboundedSender<Value>> {
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
                    if event == "open" {
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
                                started["open"] = json!(opened.link);
                                started["window"] = json!({"entrypoint": opened.entrypoint, "subject": opened.subject, "name": started["name"]});
                            }
                            started
                        });
                        let _ = proxy.send_event(Told::Run(answer(&id, result)));
                    }
                    Asked::File(responder) => responder.respond(match result {
                        Ok(file) => Response::builder()
                            .header(header::CONTENT_TYPE, file["kind"].as_str().unwrap_or_default())
                            .header(header::CONTENT_SECURITY_POLICY, crate::APP_MAY_LOAD)
                            .header(header::CACHE_CONTROL, "no-store")
                            .body(std::borrow::Cow::Owned(BASE64.decode(file["body"].as_str().unwrap_or_default().as_bytes()).unwrap_or_default()))
                            .expect("these headers are valid"),
                        Err(error) => Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(std::borrow::Cow::Owned(error.as_str().unwrap_or_default().as_bytes().to_vec()))
                            .expect("a 404 is valid"),
                    }),
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
async fn connect(address: &str) -> anyhow::Result<impl AsyncRead + AsyncWrite + Send + 'static> {
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
    anyhow::bail!("The One App didn't start")
}

#[cfg(unix)]
async fn open(address: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(address).await
}

#[cfg(windows)]
async fn open(address: &str) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(address)
}

/// The script that answers the page's ask `id`.
fn answer(id: &Value, result: Result<Value, Value>) -> String {
    let (ok, value) = match result {
        Ok(value) => (true, value),
        Err(value) => (false, value),
    };
    format!("window.__ONE_HOST__.answer({id}, {ok}, {value})")
}

/// A random number, from the hasher the standard library seeds randomly.
fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

/// What kind of file one of the page's own is.
fn kind(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "text/plain",
    }
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
