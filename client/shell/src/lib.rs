//! The app: a window on the client's Rust half. Its own page (`page`) draws nothing of its own but a problem: it
//! holds the frames. The window always opens there, and asks where it starts (`start`): a device that isn't in its
//! network yet is sent on to the network's sign-in page, with the code it's let in by, until its person has. Each app is a page of its plugin's, drawn in a frame of its own, which
//! reaches nothing here but the bridge the window's page gives it: what an app asks comes to `ask`. The home pages
//! are an app too, the plugin `home`'s, which the window shows first and under every other.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ::client::{Client, NotIn, NotLetIn, Shown};
use data_encoding::BASE64;
use serde_json::{Value, json};
use tauri::http::{Response, StatusCode, Uri, header};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

mod notifications;

/// What an app's page may load, which is what an MCP App may by default: its own plugin's files and the files of
/// the plugins it uses, which are all at its own address, and nothing from anywhere else.
const APP_MAY_LOAD: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
                            img-src 'self' data:; media-src 'self' data:; font-src 'self' data:; connect-src 'none'; \
                            frame-src 'none'; object-src 'none'; base-uri 'self'";

/// The views in the window, by name: the window's own page (`client`) and each app that's open.
struct Views {
    client: Arc<Client>,
    app: AppHandle,
    shown: Mutex<HashMap<String, Arc<Shown>>>,
    /// The files each open app's page has loaded, by the app's name.
    loaded: Mutex<HashMap<String, HashSet<String>>>,
    started: Instant,
    /// Whether the window is on screen, so its views ask again when something changed.
    on_screen: AtomicBool,
    /// Whether this device hears its notifications yet: from when the window first shows its person's home.
    hearing: AtomicBool,
}

impl Views {
    /// What `view` has been answered. A view that hears `changed` asks again.
    fn shown(&self, view: &str) -> Arc<Shown> {
        let mut shown = self.shown.lock().expect("no view panics");
        let new = || {
            let (app, view) = (self.app.clone(), view.to_owned());
            // The window going away is the only way this fails, and its views with it.
            Shown::new(self.client.clone(), move || drop(app.emit("changed", &view)))
        };
        shown.entry(view.to_owned()).or_insert_with(new).clone()
    }

    /// Asks again for what every view was answered.
    async fn again(&self) {
        let shown: Vec<_> = self.shown.lock().expect("no view panics").values().cloned().collect();
        for shown in shown {
            shown.again().await
        }
    }

    /// The files of every plugin a server lists for `who`, a group or the network, or for this client's user, as MCP
    /// lists resources.
    async fn listed(&self, who: Option<&str>) -> anyhow::Result<Value> {
        let asked = who.map_or(json!({}), |who| json!({"_meta": {"for": who}}));
        self.shown("client").ask("resources/list", asked).await
    }

    /// One file a server listed: what kind it is, and what's in it, which is its text or, for a picture or a font,
    /// base64 as MCP's `blob`. A set of a plugin's files never changes, so a file that's kept is never asked for
    /// again.
    async fn read(&self, resource: &str) -> anyhow::Result<(String, Vec<u8>)> {
        let asked = json!({"uri": resource});
        let read = match self.client.kept("resources/read", &asked) {
            Some(read) => read,
            None => self.client.fresh("resources/read", &asked).await?,
        };
        let file = &read["contents"][0];
        let kind = file["mimeType"].as_str().unwrap_or_default().to_owned();
        match file["blob"].as_str() {
            Some(blob) => Ok((kind, BASE64.decode(blob.as_bytes())?)),
            None => Ok((kind, file["text"].as_str().unwrap_or_default().as_bytes().to_vec())),
        }
    }

    /// One file for an app's page: `oneapp://<app>.localhost/<plugin>/<path>` is `path` among the newest files of
    /// `plugin` this client knows of. An app opened for a group or the network says so after its name, last part
    /// first (`notes.family.groups.localhost`), and its files are the ones listed for them. One a server didn't
    /// list, as a page loads none of them (a sourcemap), is read by name from the same version.
    async fn file(&self, address: &Uri) -> anyhow::Result<(String, Vec<u8>)> {
        let file = address.path().trim_start_matches('/');
        // A large file a tool answered with, which the client keeps, at the address the answer gave an app for it.
        if let Some(kept) = file.strip_prefix(".file/") {
            return self.client.file(kept).ok_or_else(|| anyhow::anyhow!("this client keeps no such file"));
        }
        let none = || anyhow::anyhow!("no plugin here has {file}");
        let (plugin, path) = file.split_once('/').ok_or_else(none)?;
        let app = address.host().and_then(|host| host.strip_suffix(".localhost")).unwrap_or_default();
        // Android's and Windows' web views name the app after the scheme, in a host's name.
        let app = app.strip_prefix("oneapp.").unwrap_or(app);
        let who: Vec<&str> = app.split('.').skip(1).collect();
        let who = (!who.is_empty()).then(|| who.into_iter().rev().collect::<Vec<_>>().join("/"));
        let listed = self.listed(who.as_deref()).await?;
        let mut versions = resources(&listed).filter_map(parts).filter(|(of, ..)| *of == plugin);
        let (_, code, _) = versions.next().ok_or_else(none)?;
        let resource = format!("ui://{plugin}/{code}/{path}");
        if resources(&listed).any(|listed| listed == resource) {
            self.loaded.lock().expect("no view panics").entry(app.to_owned()).or_default().insert(resource.clone());
        }
        self.read(&resource).await
    }

    /// The open apps for `who` that loaded a file a server no longer lists: one of their plugins has a new version.
    /// They have loaded nothing once this has named them.
    fn outdated(&self, who: Option<&str>, listed: &Value) -> Vec<String> {
        let listed: HashSet<&str> = resources(listed).collect();
        let whose = who.map(|who| who.split('/').rev().collect::<Vec<_>>().join("."));
        let mut loaded = self.loaded.lock().expect("no view panics");
        let outdated: Vec<String> = loaded
            .iter()
            .filter(|(app, _)| app.split_once('.').map(|(_, whose)| whose) == whose.as_deref())
            .filter(|(_, files)| files.iter().any(|file| !listed.contains(file.as_str())))
            .map(|(app, _)| app.clone())
            .collect();
        loaded.retain(|app, _| !outdated.contains(app));
        outdated
    }
}

/// The address of each file in a list of them.
fn resources(listed: &Value) -> impl Iterator<Item = &str> {
    listed["resources"].as_array().into_iter().flatten().filter_map(|resource| resource["uri"].as_str())
}

/// A plugin's file by its address, `ui://<plugin>/<code>/<path>`: its plugin, the version's code, and its path.
fn parts(resource: &str) -> Option<(&str, &str, &str)> {
    let mut parts = resource.strip_prefix("ui://")?.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The files a server lists for `who`, a group or the network, or for this client's user, and with them the open
/// apps that are `outdated` and load again, each as its address names it. Every file listed is kept behind this, so
/// with no server up an app opens whole, even one that was never opened, or whose plugins have changed since.
#[tauri::command]
async fn apps(app: AppHandle, views: State<'_, Views>, who: Option<String>) -> Result<Value, String> {
    // A person is told what went wrong, and not what was behind it.
    let mut listed = views.listed(who.as_deref()).await.map_err(|error| error.to_string())?;
    listed["outdated"] = views.outdated(who.as_deref(), &listed).into();
    let keep = listed.clone();
    tauri::async_runtime::spawn(async move {
        for resource in resources(&keep) {
            // With no server answering, what's missing is asked for when an app loads it.
            let _ = app.state::<Views>().read(resource).await;
        }
    });
    Ok(listed)
}

/// Answers what a view asks, in MCP's words: the result of `method`.
#[tauri::command]
async fn ask(views: State<'_, Views>, view: String, method: String, params: Value) -> Result<Value, String> {
    views.shown(&view).ask(&method, params).await.map_err(|error| error.to_string())
}

/// Asks again for what a view was answered: its user returned to it.
#[tauri::command]
async fn again(views: State<'_, Views>, view: String) -> Result<(), ()> {
    views.shown(&view).again().await;
    Ok(())
}

/// This device's key, as its network knows it, and the network's address, if it has one.
#[tauri::command]
fn client(views: State<'_, Views>) -> Value {
    json!({"device": views.client.key(), "network": views.client.address()})
}

/// Says whether the window is on screen. While it is, its views ask again when something changed.
#[tauri::command]
fn shown(views: State<'_, Views>, on_screen: bool) {
    views.on_screen.store(on_screen, Ordering::Relaxed);
}

/// Sends a file an app's page chose, which is the request's body, to where the network keeps it, for a tool of
/// the app's plugin. Gives the name that tool takes for it.
#[tauri::command]
async fn upload(views: State<'_, Views>, request: Request<'_>) -> Result<Value, String> {
    let plugin = request.headers().get("plugin").and_then(|plugin| plugin.to_str().ok()).unwrap_or_default().to_owned();
    let file = match request.body() {
        InvokeBody::Raw(file) => file.clone(),
        // Android's web view shows nothing a request's body, so there the file comes as a list of its bytes.
        InvokeBody::Json(Value::Array(bytes)) => bytes.iter().filter_map(|byte| Some(byte.as_u64()? as u8)).collect(),
        InvokeBody::Json(_) => return Err("An upload is a file".into()),
    };
    let sent = views.client.upload(&plugin, file).await;
    sent.map(|upload| json!({"upload": upload})).map_err(|error| error.to_string())
}

/// The recovery key of this device's person, for the home screen to show them. It's the client's own page that
/// shows it, so no app's page is handed one.
#[tauri::command]
async fn recovery_key(views: State<'_, Views>) -> Result<String, String> {
    views.client.recovery_key().await.map_err(|error| error.to_string())
}

/// Lets this device in with the recovery key its person typed on the home screen, and has every view ask again.
#[tauri::command]
async fn recover(views: State<'_, Views>, key: String) -> Result<(), String> {
    views.client.recover(&key).await.map_err(|error| error.to_string())?;
    views.again().await;
    Ok(())
}

/// Forgets what a view was answered: its page is gone, or loading again.
#[tauri::command]
fn close(views: State<'_, Views>, view: String) {
    views.shown.lock().expect("no view panics").remove(&view);
}

/// Opens the client again, which couldn't be as the app started, and opens the window again where it starts now.
#[tauri::command]
async fn enter(app: AppHandle) {
    let not_in = starts(&app, None).await;
    home(&app);
    if let Some(not_in) = not_in {
        tauri::async_runtime::spawn(let_in(app, not_in));
    }
}

/// Where the window starts, as its page asks once it has opened: `{}` for its person's home, `{signIn}` for the
/// link this device is let in by while it's nobody's, or `{unreached}`, what went wrong as the network's address
/// didn't answer.
#[tauri::command]
fn start(app: AppHandle) -> Value {
    match app.state::<Start>().0.lock().expect("no view panics").clone() {
        Opened::In => json!({}),
        Opened::NotIn(link) => json!({"signIn": link.as_str()}),
        Opened::Unreached(why) => json!({"unreached": why}),
    }
}

/// Has this device hear its person's notifications, once their home is on screen: a phone asks them then whether
/// it may show any, over what they came for and not over signing in.
#[tauri::command]
fn hear(app: AppHandle, views: State<'_, Views>) {
    if !views.hearing.swap(true, Ordering::Relaxed) {
        tauri::async_runtime::spawn(notifications::hear(app, views.client.clone()));
    }
}

/// Says how long a view took to open: an app from the tap on its icon to its content on screen, and the home
/// screen from when its page began to load, which the time since the program started is beside.
#[tauri::command]
fn opened(views: State<'_, Views>, view: String, ms: u32) {
    eprintln!("{view} opened in {ms} ms, {} ms after the app started", views.started.elapsed().as_millis());
}

/// Where the window starts, as the client last opened (`starts`).
struct Start(Mutex<Opened>);

/// How the client opened: in its network, or not in it yet, or not at all, since the network's address didn't
/// answer, which says what went wrong. While the device isn't in, the window starts on the link it's let in by,
/// which is the network's own sign-in page with its code. A web view has no passkeys, so the page is told it's in the
/// app (`&app`), and shows the code, and its button opens the link in the device's own browser.
#[derive(Clone)]
enum Opened {
    In,
    NotIn(Url),
    Unreached(String),
}

/// The home screen, as the window is sent to it. Android's and Windows' web views load only http.
#[cfg(any(windows, target_os = "android"))]
const HOME: &str = "http://tauri.localhost/index.html";
#[cfg(not(any(windows, target_os = "android")))]
const HOME: &str = "tauri://localhost/index.html";

/// Opens the client and gives the window its views, or if this device isn't in its network yet, gives what lets it
/// in and the link it's let in by, which says `why` it shows a new code, if it does.
async fn entered(app: &AppHandle, why: Option<NotLetIn>) -> anyhow::Result<Option<(Url, NotIn)>> {
    // A phone gives each app a folder of its own, which only the app is told.
    #[cfg(mobile)]
    let home = app.path().app_data_dir()?;
    #[cfg(desktop)]
    let home = ::client::home()?;
    let client = match Client::open(home).await {
        Ok(client) => client,
        Err(error) => {
            let not_in = error.downcast::<NotIn>()?;
            let mut start: Url = not_in.link.parse()?;
            start.query_pairs_mut().append_key_only("app");
            if let Some(why) = why {
                start.query_pairs_mut().append_pair("why", why.why());
            }
            return Ok(Some((start, not_in)));
        }
    };
    app.manage(Views {
        client: client.clone(),
        app: app.clone(),
        shown: Mutex::default(),
        loaded: Mutex::default(),
        started: *app.state::<Instant>(),
        on_screen: AtomicBool::new(true),
        hearing: AtomicBool::new(false),
    });

    // The home screen says when no server is answering, which is what the last thing asked found.
    let (handle, mut answering) = (app.clone(), client.answering());
    tauri::async_runtime::spawn(async move {
        while answering.changed().await.is_ok() {
            let now = *answering.borrow_and_update();
            drop(handle.emit("answering", now));
        }
    });

    // A server said something changed, so every view on screen asks again. Servers tell this client for as long as
    // it runs, so a notification reaches it with its window hidden.
    let handle = app.clone();
    let mut told = client.told();
    tauri::async_runtime::spawn(async move {
        while let Ok(message) = told.recv().await {
            let views = handle.state::<Views>();
            if message["method"] == "notifications/one/changed" && views.on_screen.load(Ordering::Relaxed) {
                eprintln!("a server said something changed");
                views.again().await
            }
        }
    });
    tauri::async_runtime::spawn(async move { client.watch(true).await });
    Ok(None)
}

/// Opens the client (`entered`) and keeps where the window starts: its person's home, or while this device isn't
/// in its network, the link it's let in by, with what waits for its person to answer. If the network's address
/// didn't answer, the home screen says so, told what went wrong, and nothing more.
async fn starts(app: &AppHandle, why: Option<NotLetIn>) -> Option<NotIn> {
    let (opened, not_in) = match entered(app, why).await {
        Ok(None) => (Opened::In, None),
        Ok(Some((link, not_in))) => (Opened::NotIn(link), Some(not_in)),
        Err(error) => {
            eprintln!("{error:#}");
            (Opened::Unreached(error.to_string()), None)
        }
    };
    *app.state::<Start>().0.lock().expect("no view panics") = opened;
    not_in
}

/// Opens the window again at the home screen, which goes on to where it starts now, in place of where it was: what
/// it leaves (a code that's been answered, a network that didn't answer) is gone, so the device's own back doesn't
/// lead there.
fn home(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(format!("location.replace({})", Value::from(HOME)));
    }
}

/// Waits for this device's person to answer, and then the window goes to the home screen once the device is in, or
/// to a new code if they stopped it or the last one ended.
async fn let_in(app: AppHandle, mut not_in: NotIn) {
    loop {
        let why = match not_in.answered().await {
            Ok(()) => None,
            Err(error) => {
                eprintln!("{error:#}");
                error.downcast_ref::<NotLetIn>().copied()
            }
        };
        let next = starts(&app, why).await;
        home(&app);
        let Some(next) = next else { return };
        not_in = next;
    }
}

/// Whether the window goes where it's sent: to its own pages and its apps', and to the sign-in page it starts on.
/// Anywhere else is the device's own browser's to open, as the sign-in page's button asks.
fn stays(app: &AppHandle, to: &Url) -> bool {
    let own = ["tauri", "oneapp", "about", "data", "blob"].contains(&to.scheme())
        || to.host_str().is_some_and(|host| host.ends_with(".localhost"));
    let start = match &*app.state::<Start>().0.lock().expect("no view panics") {
        Opened::NotIn(start) => (start.origin(), start.path(), start.query()) == (to.origin(), to.path(), to.query()),
        _ => false,
    };
    if !own && !start {
        // A phone opens it from its own thread, which is the one asking here, so it's asked from another.
        let (app, to) = (app.clone(), to.to_string());
        std::thread::spawn(move || {
            if let Err(error) = app.opener().open_url(&to, None::<&str>) {
                eprintln!("couldn't open {to} in the device's browser: {error}");
            }
        });
    }
    own || start
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started = Instant::now();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(notifications::plugin())
        .setup(move |app| {
            app.manage(Start(Mutex::new(Opened::In)));
            app.manage(started);
            // The window opens at home, which a device that isn't in its network yet goes on from to the page that
            // signs it in. A phone opens the window again at the same address as it's made again (a change of
            // text size), so that's home too, which asks where to go now.
            if let Some(not_in) = tauri::async_runtime::block_on(starts(app.handle(), None)) {
                tauri::async_runtime::spawn(let_in(app.handle().clone(), not_in));
            }
            let goes = app.handle().clone();
            let page = HOME.parse().expect("the home screen has an address");
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(page))
                .on_navigation(move |to| stays(&goes, to));
            #[cfg(desktop)]
            let window = window.title(app.package_info().name.clone()).inner_size(1024.0, 720.0);
            window.build()?;
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("oneapp", |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let response = match app.state::<Views>().file(request.uri()).await {
                    Ok((kind, file)) => Response::builder()
                        .header(header::CONTENT_TYPE, kind)
                        .header(header::CONTENT_SECURITY_POLICY, APP_MAY_LOAD)
                        .header(header::CACHE_CONTROL, "no-store")
                        .body(file),
                    Err(error) => {
                        Response::builder().status(StatusCode::NOT_FOUND).body(format!("{error:#}").into_bytes())
                    }
                };
                responder.respond(response.expect("these headers are valid"))
            });
        })
        .invoke_handler(tauri::generate_handler![
            apps,
            ask,
            again,
            upload,
            recovery_key,
            recover,
            client,
            shown,
            close,
            enter,
            start,
            hear,
            opened
        ])
        .run(tauri::generate_context!())
        .expect("the app runs");
}
