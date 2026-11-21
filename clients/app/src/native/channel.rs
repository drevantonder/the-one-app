//! A page's channel to its window and its client: what it asks, answered here as a web view's window page answers it, or by the client, and what it was told as it opened.

use super::*;

/// What the window keeps of where it is.
pub(super) struct State {
    /// The Colour Mode chosen on this device, or none for as the device is.
    pub(super) chosen: Option<String>,
    /// It's the launcher's page, not an app's window, or a page in another's frame.
    pub(super) launcher: bool,
    pub(super) framed: bool,
    /// The place in its page the link it was opened at names: what follows its `#`.
    pub(super) fragment: Option<String>,
    pub(super) trusted: bool,
    pub(super) entrypoint: String,
    /// Who it was opened for, a group or the network, or none for its person: whoever its requests are for.
    pub(super) subject: Option<String>,
    /// It's the client's recovery key window (`recovery`).
    pub(super) sheet: Option<Sheet>,
    /// It's the client's window in place of an app's that didn't open (`unopened`).
    pub(super) unopened: Option<Unopened>,
    /// It's the launcher's sign-in page, or its Update Required (`Instead`).
    pub(super) signing_in: bool,
    pub(super) behind: bool,
    /// The window it's in, once it's open.
    pub(super) window: Arc<OnceLock<WindowId>>,
    pub(super) view: String,
    /// Its page's number (`Started::page`), which what it asks is named by with `next`, so what a page loaded again in
    /// its place (`reloaded`) is answered is never the one before's.
    pub(super) page: u64,
    pub(super) next: u64,
}

pub(super) fn theme(chosen: Option<&str>) -> Option<Appearance> {
    match chosen {
        Some("dark") => Some(Appearance::Dark),
        Some("light") => Some(Appearance::Light),
        _ => None,
    }
}

/// Where this device signs in, while it isn't in its network, as its sign-in page shows it: the link its person opens
/// where they're signed in, as short as it's typed, the code it shows there, why there's a new one if there is, and the
/// network's mark.
pub(super) fn signing_in() -> Option<Value> {
    let ::client::State::SignIn { code, why } = crate::app().start.borrow().as_ref()?.1.clone() else { return None };
    let link = Url::parse(&code.link).ok()?;
    let typed = format!("{}{}{}", link.host_str().unwrap_or_default(), link.port().map(|port| format!(":{port}")).unwrap_or_default(), &link[url::Position::BeforePath..]);
    let mark = format!("data:image/png;base64,{}", BASE64.encode(include_bytes!("../../../../icons/Logo.png")));
    Some(json!({"link": code.link, "typed": typed, "code": code.code, "why": why.map(|why| why.why()), "mark": mark}))
}

/// Answers what the app's JS posted its client: what the window page answers itself, here, at once, and the rest by
/// the app's window, as from the app, which comes back as it's heard (`channel`).
pub(super) fn respond(
    message: Value,
    appearance: Appearance,
    state: &Arc<Mutex<State>>,
    to: &UnboundedSender<Value>,
    asked: &Arc<Mutex<HashMap<String, Asked>>>,
) -> Option<Value> {
    let method = message["method"].as_str()?;
    let launcher = state.lock().expect("nothing panics holding it").launcher;
    // The app asks to be left (`leave`, Escape on its first screen): the launcher is put away, the keyboard back with the
    // app that had it, as Spotlight and Raycast are; a phone's app goes home; a computer's app window stays as it is, as
    // a Mac's and Windows' apps do.
    let framed = state.lock().expect("nothing panics holding it").framed;
    // A page in a frame is its frame's page's, which stays: it leaves nothing.
    if method == "ui/notifications/request-teardown" && !framed {
        if launcher {
            computer::hide(true)
        } else if cfg!(mobile) {
            computer::summon()
        }
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
    // (`exception` in clients/app/page/util.js).
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
    // Its page has drawn what it asked for as it opened, which its opening is timed to.
    if method == "notifications/one/drawn" {
        let view = state.lock().expect("nothing panics holding it").view.clone();
        Opening::settled(&view);
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
                // Whether its window's title row names it, so its first screen doesn't: a page in a frame is named by
                // nothing above it, nor is the recovery key window's.
                "titled": cfg!(desktop) && !state.framed && state.sheet.is_none(),
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
            // A phone's launcher opens it in the page, which draws it over the launcher, for whoever it's for, out of
            // its icon (`from`), or with `behind` only starts it, as a finger lands on its icon (`go` in client.js).
            #[cfg(mobile)]
            url if url.starts_with("one:") && state.launcher => {
                let go = json!({"link": &url["one:".len()..], "subject": params["subject"], "behind": params["behind"] == true, "from": params["from"]});
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
                // What the client says of itself, as the window page answers (`respond` in bridge.js); a computer's
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
                let at = format!("m{}.{}", state.page, state.next);
                asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
                let _ = to.send(json!({"id": at, "command": "change", "args": params}));
                return None;
            }
        }
        // A shortcut on the device's own home screen is Android's page's to pin: a native window has none yet, which it
        // says, as a device without them does (bridge.js).
        "one/pin-shortcut" => Some(Err(json!({"code": -32000, "message": "This device can't put that app on its home screen"}))),
        // A person's recovery key is shown and typed in the client's own window over the app (`recovery`), for whoever
        // the app asks for, and the app is told only whether it was done (`whom` in bridge.js).
        "one/recovery-key" if state.sheet.is_none() => {
            let said = params["subject"].as_str().filter(|_| state.trusted).map(str::to_owned);
            let subject = said.or_else(|| state.subject.clone());
            state.next += 1;
            let at = format!("m{}.{}", state.page, state.next);
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
                    let at = format!("m{}.{}", state.page, state.next);
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
                                close_window(windows, *id);
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
            Some(unopened) if method == "one/unopened" => Some(Ok(json!({"name": unopened.name, "why": unopened.why, "crashed": unopened.crashed}))),
            // The launcher opens again in its panel's place, as it does once its page is updated.
            #[cfg(desktop)]
            Some(_) if state.launcher => {
                computer::reload_native();
                Some(Ok(json!({})))
            }
            Some(unopened) => {
                close(&state.window);
                if let Err(error) = crate::hosts::reopen(unopened.entrypoint.clone(), unopened.subject.clone(), unopened.link.clone()) {
                    eprintln!("{} didn't open again: {error}", unopened.entrypoint);
                }
                Some(Ok(json!({})))
            }
        },
        // What the launcher's sign-in page asks, which only it may: where the device signs in, and to open its link in
        // the device's browser, where their passkey is.
        "one/sign-in" if state.signing_in => Some(signing_in().ok_or_else(|| json!({"code": -32000, "message": "This device is signed in"}))),
        "one/sign-in/browser" if state.signing_in => {
            let (command, args) = ("open_url", json!({"url": signing_in().map(|signing_in| signing_in["link"].clone())}));
            state.next += 1;
            let at = format!("m{}.{}", state.page, state.next);
            asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
            let _ = to.send(json!({"id": at, "command": command, "args": args}));
            return None;
        }
        // What the launcher's Update Required asks, which only it may: Update, which starts again on the new build, and
        // Try Again, which asks its network afresh.
        "one/update" | "one/update/retry" if state.behind => {
            let command = if method == "one/update" { "update" } else { "retry" };
            state.next += 1;
            let at = format!("m{}.{}", state.page, state.next);
            asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
            let _ = to.send(json!({"id": at, "command": command, "args": {}}));
            return None;
        }
        // The client's own windows ask nothing else: they're no app's, and call no tool.
        method if (state.sheet.is_some() || state.unopened.is_some() || state.signing_in || state.behind) && !method.starts_with("ui/") && method != "ping" && method != "one/client" => {
            Some(Err(json!({"code": -32601, "message": format!("This window doesn't ask {method}")})))
        }
        method if method.starts_with("ui/") => Some(Err(json!({"code": -32601, "message": format!("This client has no {method}")}))),
        _ => None,
    };
    // A file the page chose goes where the network keeps it, for a tool of this window's app (`upload` in hosts.rs).
    if answered.is_none() && method == "one/upload" {
        state.next += 1;
        let at = format!("m{}.{}", state.page, state.next);
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
            let at = format!("m{}.{}", state.page, state.next);
            let app = state.entrypoint.split('/').next().unwrap_or_default().to_owned();
            let mut params = params.clone();
            let mut meta = if state.trusted { params["_meta"].as_object().cloned().unwrap_or_default() } else { Default::default() };
            meta.insert("app".into(), json!(app));
            // It asks for whoever its window was opened for, a group or the network, unless a trusted app's page says
            // whom (`whom` in bridge.js).
            if let Some(subject) = state.subject.as_ref().filter(|_| !meta.get("subject").is_some_and(Value::is_string)) {
                meta.insert("subject".into(), json!(subject));
            }
            params["_meta"] = Value::Object(meta);
            asked.lock().expect("nothing panics holding it").insert(at.clone(), Asked::Message(id));
            let what = params["name"].as_str().or(params["uri"].as_str()).map_or_else(|| method.to_owned(), |name| format!("{method} {name}"));
            Opening::asked(&state.view, &at, what);
            let _ = to.send(json!({"id": at, "command": "request", "args": {"view": state.view, "method": method, "params": params}}));
            None
        }
    }
}

/// Talks with this process's own channel for the app's windows (`hosts::talk`), as a web view's window does over its
/// socket, saying hello as it does: what it hears is done to the window on the native thread. → where to send it a
/// message.
pub(super) fn channel(
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
    // Held weakly, as what reads the channel holds it for as long as the channel runs: the window, as it closes, by
    // its system's close too, drops what it holds, and the channel ends with it. Held strongly, it kept it open, so
    // its app's window was never forgotten and wouldn't open again (21 Nov).
    let (to_page, asked_page) = (to.downgrade(), asked.clone());
    let (entrypoint, subject, link) = (entrypoint.to_owned(), subject.map(str::to_owned), link.to_owned());
    // Its app's host, as a server says which were loaded against what's gone (`stale`): `<app>[.<subject, last part first>]`.
    let app_host = origin(&entrypoint, subject.as_deref()).trim_start_matches("oneapp://").trim_end_matches(".localhost/").to_owned();
    crate::spawn(async move {
        let mut lines = BufReader::new(read).lines();
        // Reload, in its notice that its page was updated: an app's window loads its page's new version in place
        // (`reloaded`), and one whose new version this host can't draw opens again, as a new window would, saying why.
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
                let (window, native, asked) = (window.clone(), native.clone(), asked_page.clone());
                let Some(to) = to_page.upgrade() else { return };
                crate::spawn(async move {
                    match reloaded(window.clone(), entrypoint.clone(), subject.clone(), link.clone(), to, asked).await {
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("{entrypoint} didn't load again in its window, which opens again: {error:#}");
                            native.run(move |windows| {
                                if let Some(id) = window.get() {
                                    close_window(windows, *id);
                                }
                                if let Err(error) = crate::hosts::reopen(entrypoint, subject, link) {
                                    eprintln!("{error}");
                                }
                            })
                        }
                    }
                });
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
                    // Behind its servers, the launcher says Update Required in the Home Screen's place.
                    "state" if launcher && payload["connection"] == "behind" => computer::behind(),
                    // Over its page, while its network doesn't answer, as over an app on a phone (DESIGN.md).
                    "state" if framed.is_none() => on(Box::new(move |screen| {
                        screen.notice = match payload["connection"].as_str() {
                            Some("reconnecting") => Some(renderer::Notice { text: "Reconnecting…".into(), action: None }),
                            Some("offline") => Some(renderer::Notice { text: "Can't reach your network".into(), action: None }),
                            _ => screen.notice.take().filter(|notice| notice.action.is_some()),
                        }
                    })),
                    // Its page, or the UI library it runs, was loaded against a version that's gone: it loads again by
                    // itself as soon as nothing is under way on it (`Screen::idle`), never under a press, a drag or
                    // what its person is typing (adf8f19b8). A frame's page waits on the window it's in.
                    "stale" if payload.as_array().is_some_and(|stale| stale.iter().filter_map(Value::as_str).any(|host| host.split('.').next() == Some("ui") || host == app_host)) => {
                        let (reload, window) = (reload.clone(), window.clone());
                        native.run(move |windows| {
                            if let Some(id) = window.get() {
                                windows.with(*id, |screen| {
                                    screen.when_unused.push(Box::new(reload));
                                    screen.unused();
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
            Opening::answered(&view, id);
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
                close_window(windows, *id);
            }
        });
    });
    Ok(to_window)
}
