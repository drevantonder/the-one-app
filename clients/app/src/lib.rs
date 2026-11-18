//! The app: a window on One Core in a client (`core/client`). Its own page (`page`) draws nothing of its own but its
//! device's session, whether its network answers, and what's under way or wrong with it: it holds the frames. The
//! window always opens there, and follows the client's session (`Session`) and connection (`Connection`): a device
//! that isn't in its network is sent on to the network's sign-in page, with the code it's let in by, until its person
//! has let it in. Each app is a page of its app's, drawn in a frame of its own, which
//! reaches nothing here but the bridge the window's page gives it: what an app asks comes to `ask`. The launcher
//! is an app too, the one a network lists in the category `launcher`, which the window shows first and under every other.

#![cfg(feature = "window")]
// What an app's own window asks is answered by the futures the Home Screen's commands are, a deep nest of them (`hosts`).
#![recursion_limit = "256"]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ::client::{Client, Connection, Session, Subscription};
use data_encoding::BASE64;
use serde_json::{Value, json};
use tauri::http::{Response, StatusCode, Uri, header};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

/// Notifications' native code, which shows them on this device.
use notifications_client as notifications;

pub mod phone;
mod shortcuts;
#[cfg(desktop)]
pub mod host;
#[cfg(desktop)]
mod hosts;

/// What an app's page may load, which is what an MCP App may by default: its own app's files and the files of
/// the apps it uses, which are all at its own address, and nothing from anywhere else.
pub(crate) const APP_MAY_LOAD: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
                            img-src 'self' data:; media-src 'self' data:; font-src 'self' data:; connect-src 'none'; \
                            frame-src 'none'; object-src 'none'; base-uri 'self'";

/// The views in the window, by name: the window's own page (`client`) and each app that's open.
struct Views {
    client: Arc<Client>,
    app: AppHandle,
    subscriptions: Mutex<HashMap<String, Arc<Subscription>>>,
    /// The files each open app's page has loaded, by the app's name.
    loaded: Mutex<HashMap<String, HashSet<String>>>,
    started: Instant,
    /// The windows that are on screen, the Home Screen's (`main`) and on a computer each app's (`hosts`), so its
    /// views ask again when something changed.
    on_screen: Mutex<HashSet<String>>,
    /// Whether a server said something changed while the window wasn't on screen, so its views ask again once it is.
    missed: AtomicBool,
    /// Whether this device hears its notifications yet: from when the window first shows its person's home.
    hearing: AtomicBool,
}

impl Views {
    /// What `view` has been answered. A view that hears `changed` asks again.
    fn subscription(&self, view: &str) -> Arc<Subscription> {
        let mut shown = self.subscriptions.lock().expect("no view panics");
        let new = || {
            let (app, view) = (self.app.clone(), view.to_owned());
            // The window going away is the only way this fails, and its views with it.
            Subscription::new(self.client.clone(), move || drop(app.emit("changed", &view)))
        };
        shown.entry(view.to_owned()).or_insert_with(new).clone()
    }

    /// Says whether the window `label` is on screen.
    fn shown(&self, label: &str, on_screen: bool) {
        let mut shown = self.on_screen.lock().expect("no view panics");
        if on_screen {
            shown.insert(label.to_owned());
        } else {
            shown.remove(label);
        }
    }

    /// Whether any window is on screen.
    fn on_screen(&self) -> bool {
        !self.on_screen.lock().expect("no view panics").is_empty()
    }

    /// Asks again, as a window is back on screen, if something changed while none was.
    async fn caught_up(&self) {
        if self.on_screen() && self.missed.swap(false, Ordering::Relaxed) {
            self.refresh().await
        }
    }

    /// Asks again for what every view was answered.
    async fn refresh(&self) {
        let shown: Vec<_> = self.subscriptions.lock().expect("no view panics").values().cloned().collect();
        for shown in shown {
            shown.refresh().await
        }
    }

    /// The files of every app a server lists for `subject`, a group or the network, or for this client's user, as MCP
    /// lists resources.
    async fn listed(&self, subject: Option<&str>) -> anyhow::Result<Value> {
        let request = subject.map_or(json!({}), |subject| json!({"_meta": {"subject": subject}}));
        self.subscription("client").request("resources/list", request).await
    }

    /// One file a server listed: what kind it is, and what's in it, which is its text or, for a picture or a font,
    /// base64 as MCP's `blob`. A set of an app's files never changes, so a file that's kept is never asked for
    /// again.
    async fn read(&self, resource: &str) -> anyhow::Result<(String, Vec<u8>)> {
        let request = json!({"uri": resource});
        let read = match self.client.cached("resources/read", &request) {
            Some(read) => read,
            None => self.client.fresh("resources/read", &request).await?,
        };
        let file = &read["contents"][0];
        let kind = file["mimeType"].as_str().unwrap_or_default().to_owned();
        match file["blob"].as_str() {
            Some(blob) => Ok((kind, BASE64.decode(blob.as_bytes())?)),
            None => Ok((kind, file["text"].as_str().unwrap_or_default().as_bytes().to_vec())),
        }
    }

    /// One file for an app's page: `oneapp://<app>.localhost/<app>/<path>` is `path` among the latest files of
    /// `app` this client knows of. An app opened for a group or the network says so after its name, last part
    /// first (`notes.family.groups.localhost`), and its files are the ones listed for them. One a server didn't
    /// list, as a page loads none of them (a sourcemap), is read by name from the same version.
    async fn file(&self, address: &Uri) -> anyhow::Result<(String, Vec<u8>)> {
        let file = address.path().trim_start_matches('/');
        // A large file a tool answered with, which the client keeps, at the address the answer gave an app for it.
        if let Some(path) = file.strip_prefix(".file/") {
            return self.client.file(path).ok_or_else(|| anyhow::anyhow!("this client keeps no such file"));
        }
        let none = || anyhow::anyhow!("no app here has {file}");
        let (name, path) = file.split_once('/').ok_or_else(none)?;
        let app = address.host().and_then(|host| host.strip_suffix(".localhost")).unwrap_or_default();
        // Android's and Windows' web views name the app after the scheme, in a host's name.
        let app = app.strip_prefix("oneapp.").unwrap_or(app);
        let subject: Vec<&str> = app.split('.').skip(1).collect();
        let subject = (!subject.is_empty()).then(|| subject.into_iter().rev().collect::<Vec<_>>().join("/"));
        let listed = self.listed(subject.as_deref()).await?;
        let mut versions = resources(&listed).filter_map(parts).filter(|(of, ..)| *of == name);
        let (_, code, _) = versions.next().ok_or_else(none)?;
        let resource = format!("ui://{name}/{code}/{path}");
        if resources(&listed).any(|listed| listed == resource) {
            self.loaded.lock().expect("no view panics").entry(app.to_owned()).or_default().insert(resource.clone());
        }
        self.read(&resource).await
    }

    /// The open apps loaded against a version of their app, or a build of the UI library, that `changed` says
    /// isn't the one there is now, each as its address names it: a server says the UI library's build (`library`),
    /// and the version each app of the owners whose apps moved runs at (`apps`, by owner). A version that's
    /// still one of them is current, as an app opened for another owner may be on its own. Another app's files an
    /// app shows, as the launcher shows every app's icon, aren't what it runs. They have loaded nothing once this has
    /// named them.
    fn stale(&self, changed: &Value) -> Vec<String> {
        let gone = |name: &str, code: &str| {
            if name == "ui" {
                return changed["library"].as_str().is_some_and(|library| library != code);
            }
            let owners = changed["apps"].as_object().into_iter().flat_map(|owners| owners.values());
            let now: Vec<&str> = owners.filter_map(|apps| apps[name].as_str()).collect();
            !now.is_empty() && !now.contains(&code)
        };
        let mut loaded = self.loaded.lock().expect("no view panics");
        let stale: Vec<String> = loaded
            .iter()
            .filter(|(app, files)| runs(app, files).any(|(_, name, code)| gone(name, code)))
            .map(|(app, _)| app.clone())
            .collect();
        loaded.retain(|app, _| !stale.contains(app));
        stale
    }

    /// The open apps for `subject` that run a file a server no longer lists (`runs`): their app or the UI library has
    /// a new version. They have loaded nothing once this has named them.
    fn outdated(&self, subject: Option<&str>, listed: &Value) -> Vec<String> {
        let listed: HashSet<&str> = resources(listed).collect();
        let whose = subject.map(|subject| subject.split('/').rev().collect::<Vec<_>>().join("."));
        let mut loaded = self.loaded.lock().expect("no view panics");
        let outdated: Vec<String> = loaded
            .iter()
            .filter(|(app, _)| app.split_once('.').map(|(_, whose)| whose) == whose.as_deref())
            .filter(|(app, files)| runs(app, files).any(|(file, ..)| !listed.contains(file)))
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

/// What an open app, by its address's name, runs of the `files` it loaded: its own app's and the UI library's, each
/// as its address, app and version's code. Another app's file it shows, as the launcher shows every app's
/// icon, isn't what it runs.
fn runs<'a>(app: &'a str, files: &'a HashSet<String>) -> impl Iterator<Item = (&'a str, &'a str, &'a str)> {
    let own = app.split('.').next().unwrap_or(app);
    files.iter().filter_map(|file| Some((file.as_str(), parts(file)?))).filter(move |(_, (name, ..))| [own, "ui"].contains(name)).map(|(file, (name, code, _))| (file, name, code))
}

/// An app's file by its address, `ui://<app>/<code>/<path>`: its app, the version's code, and its path.
fn parts(resource: &str) -> Option<(&str, &str, &str)> {
    let mut parts = resource.strip_prefix("ui://")?.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The files a server lists for `subject`, a group or the network, or for this client's user, and with them the open
/// apps that are `outdated` and load again, each as its address names it. Every file listed is kept behind this, so
/// with no server up an app opens whole, even one that was never opened, or whose apps have changed since. The user's
/// apps with a screen are where the device lists its own apps too (`shortcuts`).
#[tauri::command]
async fn apps(app: AppHandle, views: State<'_, Views>, subject: Option<String>) -> Result<Value, String> {
    // A person is told what went wrong, and not what was behind it.
    let mut listed = views.listed(subject.as_deref()).await.map_err(|error| error.to_string())?;
    listed["outdated"] = views.outdated(subject.as_deref(), &listed).into();
    if subject.is_none() {
        shortcuts::keep(&app, shortcuts::listed(&listed));
    }
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
async fn request(views: State<'_, Views>, view: String, method: String, params: Value) -> Result<Value, String> {
    views.subscription(&view).request(&method, params).await.map_err(|error| error.to_string())
}

/// Asks again for what a view was answered: its user returned to it.
#[tauri::command]
async fn refresh(views: State<'_, Views>, view: String) -> Result<(), ()> {
    views.subscription(&view).refresh().await;
    Ok(())
}

/// This device's key, as its network knows it, the network's address, if it has one, the build it's on and its kind
/// of machine, as builds are named, its server's key, if it's a server, and whether a phone is woken by its
/// notifications (`notifications::Push`), and whether its person turned Computer Use on, on a phone that has it.
#[tauri::command]
fn client(views: State<'_, Views>) -> Value {
    json!({
        "device": views.client.key(),
        "network": views.client.address(),
        "build": ::client::BUILD,
        "target": ::client::target(),
        "server": cli::serving::server(),
        "push": notifications::push(),
        "computerUse": computer_use_client::phone(),
    })
}

/// Makes this device a server of its person's, or with `on` false, no longer one, as `one serve on` and `one serve
/// off` do. Gives what they say.
#[tauri::command]
async fn serve(views: State<'_, Views>, on: bool) -> Result<String, String> {
    let done = if on { cli::serving::on(&views.client).await } else { cli::serving::off(&views.client).await };
    done.map_err(|error| format!("{error:#}"))
}

/// Takes the latest builds for this device, as `one update` does, and starts again on them, as an app does once
/// it has updated. Gives what it says, which it gives only where it isn't starting again (a restart never returns):
/// the window then asks its network whether it's still behind. On a device that's also a server, the program keeping its server running
/// takes them, this program's among them, and says where that is in `builds`: this starts again once its file is a
/// newer build than it runs. An iPhone's app from TestFlight (`testflight`) opens TestFlight instead, which updates it.
#[tauri::command]
async fn update(app: AppHandle, views: State<'_, Views>) -> Result<String, String> {
    if testflight() {
        app.opener().open_url("itms-beta://", None::<&str>).map_err(|error| error.to_string())?;
        return Ok(String::new());
    }
    let message = cli::serving::update(&views.client).await.map_err(|error| format!("{error:#}"))?;
    if cli::serving::server().is_none() || cli::serving::replaced() {
        app.restart()
    }
    Ok(message)
}

/// Says whether the window is on screen. While one is, its views ask again when something changed, and as one's back,
/// if something changed while none was.
#[tauri::command]
async fn visible(window: tauri::Window, views: State<'_, Views>, on_screen: bool) -> Result<(), ()> {
    views.shown(window.label(), on_screen);
    views.caught_up().await;
    Ok(())
}

/// Sends a file an app's page chose, which is the request's body, to where the network keeps it, for a tool of
/// the app's app. Gives the name that tool takes for it.
#[tauri::command]
async fn upload(views: State<'_, Views>, request: Request<'_>) -> Result<Value, String> {
    let app = request.headers().get("app").and_then(|app| app.to_str().ok()).unwrap_or_default().to_owned();
    let file = match request.body() {
        InvokeBody::Raw(file) => file.clone(),
        // Android's web view shows nothing a request's body, so there the file comes as a list of its bytes.
        InvokeBody::Json(Value::Array(bytes)) => bytes.iter().filter_map(|byte| Some(byte.as_u64()? as u8)).collect(),
        InvokeBody::Json(_) => return Err("An upload is a file".into()),
    };
    let sent = views.client.upload(&app, file).await;
    sent.map(|upload| json!({"upload": upload})).map_err(|error| error.to_string())
}

/// The recovery key of this device's person, or of a group or the network an app was opened for (`subject`), for the
/// client's own page to show them. It's that page that shows it, so no app's page is handed one.
#[tauri::command]
async fn recovery_key(views: State<'_, Views>, subject: Option<String>) -> Result<String, String> {
    encryption_client::recovery_key(&views.client, subject.as_deref()).await.map_err(|error| error.to_string())
}

/// Lets this device hold its person's key, or `subject`'s, with the recovery key they typed on the client's own page,
/// and has every view ask again.
#[tauri::command]
async fn recover(views: State<'_, Views>, key: String, subject: Option<String>) -> Result<(), String> {
    encryption_client::recover(&views.client, &key, subject.as_deref()).await.map_err(|error| error.to_string())?;
    views.refresh().await;
    Ok(())
}

/// Forgets what a view was answered: its page is gone, or loading again.
#[tauri::command]
fn close(views: State<'_, Views>, view: String) {
    views.subscriptions.lock().expect("no view panics").remove(&view);
}

/// Asks the network again now, which didn't answer or refused this client for being behind it, as its person does
/// with Try Again (`Client::retry`), and answers once it has, failing where it still isn't answered. A client that
/// couldn't open at all is opened again.
#[tauri::command]
async fn retry(app: AppHandle) -> Result<(), String> {
    if app.try_state::<Views>().is_none() {
        tauri::async_runtime::spawn(opens(app));
        return Ok(());
    }
    app.state::<Views>().client.retry().await.map_err(|error| error.to_string())
}

/// The device is back online or the window in front again, so a client whose network didn't answer asks it again
/// now (`Client::reconnect`).
#[tauri::command]
fn reconnect(views: State<'_, Views>) {
    views.client.reconnect();
}

/// Signs this device out once its person has removed it from their devices (Devices' Sign Out): what it kept of
/// theirs goes, and the window follows it to the sign-in page.
#[tauri::command]
async fn sign_out(views: State<'_, Views>) -> Result<(), String> {
    views.client.sign_out().await.map_err(|error| format!("{error:#}"))
}

/// Where this device is with its network, as the window's page asks once it has opened (`told`), which waits for
/// the client to open, whether its network answers, and whether a phone hears a tap on a notification, through its
/// push plugin, which the page listens to. The page follows the session and the connection from there (`session`,
/// `connection`). A client that couldn't open at all has reached nothing.
#[tauri::command]
async fn start(app: AppHandle) -> Result<Value, ()> {
    let mut session = app.state::<Start>().0.subscribe();
    let (seq, session) = session.wait_for(Option::is_some).await.map_err(drop)?.clone().expect("it waited for one");
    let mut told = told(seq, &session);
    told["taps"] = json!(cfg!(mobile));
    told["testflight"] = json!(testflight());
    // A computer opens each app in a window of its own (`hosts`).
    told["windows"] = json!(cfg!(desktop));
    if let Some(theme) = *app.state::<Chosen>().0.lock().expect("nothing panics holding it") {
        told["theme"] = json!(theme);
    }
    let connection = app.try_state::<Views>().map(|views| *views.client.connection().borrow());
    told["connection"] = json!(connection.unwrap_or(Connection::Offline));
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let started = app.deep_link().get_current().ok().flatten().unwrap_or_default();
        told["open"] = json!(started.iter().find_map(link));
    }
    Ok(told)
}

/// Whether this is an iPhone's app installed from TestFlight, which updates it, as Apple tells it: its App Store
/// receipt is the sandbox's. A build installed from a Mac has none, and an App Store one is `receipt`.
fn testflight() -> bool {
    #[cfg(target_os = "ios")]
    unsafe {
        use objc2::runtime::AnyObject;
        let bundle: *mut AnyObject = objc2::msg_send![objc2::class!(NSBundle), mainBundle];
        let url: *mut AnyObject = objc2::msg_send![bundle, appStoreReceiptURL];
        if url.is_null() {
            return false;
        }
        let name: *mut AnyObject = objc2::msg_send![url, lastPathComponent];
        let utf8: *const std::ffi::c_char = objc2::msg_send![name, UTF8String];
        return !utf8.is_null() && std::ffi::CStr::from_ptr(utf8).to_bytes() == b"sandboxReceipt";
    }
    #[cfg(not(target_os = "ios"))]
    false
}

/// What a `one:` link opens, as the window's page opens it (`go`): `<app>[/<page>][#<fragment>]`, an app, or a page
/// of one, and the place in it its fragment names, which the app follows (`one:home-screen#library`).
fn link(url: &Url) -> Option<String> {
    (url.scheme() == "one").then(|| url.as_str()["one:".len()..].to_owned())
}

/// Opens what each `one:` link the device hands the app links to, over whatever's showing, as a tap on a notification
/// does, and brings the window forward. The link the app was started with is the page's to ask for (`start`). A
/// computer's program that isn't a developer's build takes the device's `one:` links: Linux and Windows have a
/// program say so itself (a Mac's and an iPhone's say so in their bundles, and Android's in its manifest).
fn links(app: &tauri::App) -> tauri::Result<()> {
    use tauri_plugin_deep_link::DeepLinkExt;
    #[cfg(all(any(windows, target_os = "linux"), not(debug_assertions)))]
    if let Err(error) = app.deep_link().register_all() {
        eprintln!("this device's one: links don't open this app: {error}");
    }
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for opens in event.urls().iter().filter_map(link) {
            drop(handle.emit("open", opens));
        }
    });
    Ok(())
}

/// What the window's page is told of where this device is now: its session, the session's place in the order the
/// page hears them in, and while the device is signing in, where the window goes: the network's sign-in page with its
/// code, told it's in the app (`&app`), which has no passkeys, and why it shows a new code, if it says.
fn told(seq: u64, session: &Session) -> Value {
    let link = match session {
        Session::SignIn { code, why } => code.link.parse::<Url>().ok().map(|mut link| {
            link.query_pairs_mut().append_key_only("app");
            if let Some(why) = why {
                link.query_pairs_mut().append_pair("why", why.why());
            }
            link.to_string()
        }),
        _ => None,
    };
    json!({"seq": seq, "session": session, "link": link})
}

/// Has a phone whose push failed try again now (`notifications::Push`), as its person does in Notifications, and
/// says where it is once that try has ended.
#[tauri::command]
async fn push() -> Result<Value, ()> {
    Ok(json!({"message": format!("{:?}", notifications::retry().await)}))
}

/// Has this device hear its person's notifications, once their home is on screen: a phone asks them then whether
/// it may show any, over what they came for and not over signing in. Each time after, as its home is back in front,
/// a phone looks again at whether it may (`notifications::Push`), as its person may have allowed them in its Settings.
#[tauri::command]
fn listen(app: AppHandle, views: State<'_, Views>) {
    if views.hearing.swap(true, Ordering::Relaxed) {
        return notifications::look_again();
    }
    tauri::async_runtime::spawn(notifications::listen(app, views.client.clone()));
}

/// A phone's push service gave it a push token, as its push plugin said to the window's page (`push-token`), which
/// it registers if it isn't the one it registered.
#[tauri::command]
fn push_token(token: String) {
    notifications::token(token);
}

/// A notification was tapped on a phone, which its push plugin said to the window's page (`notificationClicked`), so
/// the window opens the app it's from, as a tap on a computer's does.
#[tauri::command]
fn tapped(app: AppHandle, tapped: Value) {
    if let Some(name) = notifications::opens(&tapped) {
        drop(app.emit("open", name));
    }
}

/// Says how long a view took to open: an app from the tap on its icon to its content on screen, and the home
/// screen from when its page began to load, which the time since the program started is beside. Telemetry is told.
#[tauri::command]
fn opened(views: State<'_, Views>, view: String, ms: u32) {
    eprintln!("{view} opened in {ms} ms, {} ms after the app started", views.started.elapsed().as_millis());
    views.client.report(json!({"name": "one.client.open.duration", "value": ms, "attributes": {"one.view": view}}));
}

/// Turns the device's own back and forward gestures on, or off: an iPhone's swipe in from the edge, which follows the
/// finger as a pop does, a Mac's two-finger swipe, and a Linux touchpad's. They go back and forward through the window's
/// history, as its Back buttons do, and its page has them off while something is open over the app that's showing,
/// which they'd go back behind, and at an app's first screen, where the swipe takes the app home (`home`): an
/// iPhone's swipe in from the edge there is the window's own (`edge`), and a computer's is its page's. A web view has
/// them off unless it's asked; Android's has none, and its back is `MainActivity`'s.
#[tauri::command]
fn navigation_gestures(window: tauri::WebviewWindow, on: bool, home: bool) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    edge::take(home);
    #[cfg(not(target_os = "ios"))]
    let _ = home;
    window
        .with_webview(move |webview| {
            // It's the window's WKWebView, which Tauri hands over on the main thread, where it's used.
            #[cfg(target_vendor = "apple")]
            unsafe {
                let view = &*webview.inner().cast::<objc2::runtime::AnyObject>();
                let () = objc2::msg_send![view, setAllowsBackForwardNavigationGestures: on];
            }
            #[cfg(target_os = "linux")]
            {
                use webkit2gtk::{SettingsExt, WebViewExt};
                if let Some(settings) = webview.inner().settings() {
                    settings.set_enable_back_forward_navigation_gestures(on)
                }
            }
            #[cfg(any(windows, target_os = "android"))]
            let _ = (webview, on);
        })
        .map_err(|error| error.to_string())
}

/// Shows or hides an iPhone's status bar, as the app that's showing asks: its Home Screen hides it while it's edited, as
/// iOS's own does, and puts + and Done where it was. Tao's view controller says whether it's hidden, and has iOS ask
/// again as it changes. A computer has none, and Android's window is already clear of its bars.
#[tauri::command]
fn status_bar(window: tauri::WebviewWindow, shown: bool) -> Result<(), String> {
    window
        .with_webview(move |webview| {
            #[cfg(target_os = "ios")]
            unsafe {
                // Tauri hands over the window's view controller on the main thread.
                let controller = &*webview.view_controller().cast::<objc2::runtime::AnyObject>();
                let () = objc2::msg_send![controller, setPrefersStatusBarHidden: !shown];
            }
            #[cfg(not(target_os = "ios"))]
            let _ = (webview, shown);
        })
        .map_err(|error| error.to_string())
}

/// Light or dark, as Colour Mode has it on this device, or as the device is with neither: what's the window's own and
/// not its page follows it too, a computer's title bar and an iPhone's status bar, which would otherwise follow the
/// device and show dark on dark. Android's bars are `MainActivity`'s (`androidTheme` in host.js). Every page is told
/// (`chose`), as each of a computer's app windows has a page of its own.
#[tauri::command]
fn theme(app: AppHandle, window: tauri::WebviewWindow, theme: Option<tauri::Theme>) -> Result<(), String> {
    chose(&app, theme);
    #[cfg(desktop)]
    return window.set_theme(theme).map_err(|error| error.to_string());
    #[cfg(target_os = "ios")]
    return window
        .with_webview(move |webview| {
            use objc2_ui_kit::{UIUserInterfaceStyle, UIView};
            // Tauri hands over the window's WKWebView on the main thread.
            let web = unsafe { &*webview.inner().cast::<UIView>() };
            let style = match theme {
                Some(tauri::Theme::Light) => UIUserInterfaceStyle::Light,
                Some(tauri::Theme::Dark) => UIUserInterfaceStyle::Dark,
                _ => UIUserInterfaceStyle::Unspecified,
            };
            if let Some(window) = web.window() {
                window.setOverrideUserInterfaceStyle(style);
            }
        })
        .map_err(|error| error.to_string());
    #[cfg(target_os = "android")]
    {
        let _ = (window, theme);
        Ok(())
    }
}

/// The Colour Mode chosen on this device, as the last page to choose it said, for each page that opens after
/// (`start`), and told to every page that's open.
struct Chosen(Mutex<Option<Option<tauri::Theme>>>);

fn chose(app: &AppHandle, theme: Option<tauri::Theme>) {
    *app.state::<Chosen>().0.lock().expect("nothing panics holding it") = Some(theme);
    drop(app.emit("theme", theme));
}

/// A tap on an iPhone's status bar scrolls the app that's showing to its top, as it does in every iPhone app. iOS
/// scrolls the one scroll view that asks to be (`scrollsToTop`), which in the web view is its page, and that doesn't
/// scroll: its apps' frames do. So the page's says no, and an unseen scroll view a point high along the window's top
/// is asked instead (`scrollViewShouldScrollToTop:`). It says no too, and has the page scroll the app that's
/// showing to its top (`scrollToTop` in client.js).
#[cfg(target_os = "ios")]
mod status_bar {
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_ui_kit::{UIScrollView, UIScrollViewDelegate, UIView, UIViewAutoresizing};

    static WINDOW: OnceLock<tauri::WebviewWindow> = OnceLock::new();

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OneStatusBarTap"]
        struct Tap;

        unsafe impl NSObjectProtocol for Tap {}

        unsafe impl UIScrollViewDelegate for Tap {
            #[unsafe(method(scrollViewShouldScrollToTop:))]
            fn scroll_view_should_scroll_to_top(&self, _view: &UIScrollView) -> bool {
                if let Some(window) = WINDOW.get() {
                    let _ = window.eval("scrollToTop()");
                }
                false
            }
        }
    );

    pub fn watch(window: &tauri::WebviewWindow) -> tauri::Result<()> {
        let _ = WINDOW.set(window.clone());
        window.with_webview(|webview| {
            // Tauri hands over the window's WKWebView on the main thread.
            let main = unsafe { MainThreadMarker::new_unchecked() };
            let web = unsafe { &*webview.inner().cast::<UIView>() };
            let page: Retained<UIScrollView> = unsafe { msg_send![web, scrollView] };
            page.setScrollsToTop(false);
            let width = web.bounds().size.width;
            let tapped = UIScrollView::initWithFrame(UIScrollView::alloc(main), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, 1.0)));
            tapped.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth);
            // Never at its top, so it's always one there is to scroll.
            tapped.setContentSize(CGSize::new(width, 2.0));
            tapped.setContentOffset(CGPoint::new(0.0, 1.0));
            tapped.setScrollsToTop(true);
            let tap: Retained<Tap> = unsafe { msg_send![Tap::alloc(main), init] };
            unsafe { tapped.setDelegate(Some(ProtocolObject::from_ref(&*tap))) };
            web.addSubview(&tapped);
            // A scroll view holds its delegate weakly, and this one lasts as long as the app.
            std::mem::forget(tap);
        })
    }
}

/// An iPhone's swipe in from the left edge, at an app's first screen, takes it home as its Back does: the page's move
/// follows the finger as far across the window as it's come (`grab`, `drag` and `release` in client.js), as a pop
/// does, and let go, it carries on if it's past halfway or flicked, and otherwise springs back. It's on only where the
/// page says the swipe takes an app home (`navigation_gestures`); deeper in an app, the web view's own swipe goes back
/// through the app's screens.
#[cfg(target_os = "ios")]
mod edge {
    use std::cell::OnceCell;
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_ui_kit::{UIGestureRecognizerState, UIRectEdge, UIScreenEdgePanGestureRecognizer, UIView};

    static WINDOW: OnceLock<tauri::WebviewWindow> = OnceLock::new();
    thread_local! {
        // UIKit's, on the main thread, as long as the app.
        static PAN: OnceCell<Retained<UIScreenEdgePanGestureRecognizer>> = const { OnceCell::new() };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OneEdgeSwipe"]
        struct Swipe;

        unsafe impl NSObjectProtocol for Swipe {}

        impl Swipe {
            #[unsafe(method(panned:))]
            fn panned(&self, pan: &UIScreenEdgePanGestureRecognizer) {
                let (Some(window), Some(view)) = (WINDOW.get(), pan.view()) else { return };
                let width = view.bounds().size.width.max(1.0);
                let state: UIGestureRecognizerState = unsafe { msg_send![pan, state] };
                let script = match state {
                    UIGestureRecognizerState::Began => "grab()".to_owned(),
                    UIGestureRecognizerState::Changed => format!("drag({})", pan.translationInView(Some(&*view)).x / width),
                    UIGestureRecognizerState::Ended => format!("release({})", pan.velocityInView(Some(&*view)).x / width),
                    // Taken away, as by a call coming in: it goes back where it was.
                    UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => "release(0, 0)".to_owned(),
                    _ => return,
                };
                let _ = window.eval(script);
            }
        }
    );

    pub fn watch(window: &tauri::WebviewWindow) -> tauri::Result<()> {
        let _ = WINDOW.set(window.clone());
        window.with_webview(|webview| {
            // Tauri hands over the window's WKWebView on the main thread.
            let main = unsafe { MainThreadMarker::new_unchecked() };
            let web = unsafe { &*webview.inner().cast::<UIView>() };
            let swipe: Retained<Swipe> = unsafe { msg_send![Swipe::alloc(main), init] };
            let target: &AnyObject = &swipe;
            let pan = unsafe { UIScreenEdgePanGestureRecognizer::initWithTarget_action(UIScreenEdgePanGestureRecognizer::alloc(main), Some(target), Some(sel!(panned:))) };
            pan.setEdges(UIRectEdge::Left);
            pan.setEnabled(false);
            web.addGestureRecognizer(&pan);
            // A gesture recognizer holds its target weakly, and this one lasts as long as the app.
            std::mem::forget(swipe);
            PAN.with(|kept| drop(kept.set(pan)));
        })
    }

    /// Whether the swipe in from the edge is the window's.
    pub fn take(on: bool) {
        // Tauri runs a command off the main thread, and UIKit's are the main thread's.
        if let Some(window) = WINDOW.get() {
            let _ = window.run_on_main_thread(move || PAN.with(|pan| pan.get().map(|pan| pan.setEnabled(on))).unwrap_or_default());
        }
    }
}

/// A developer's build writes what its pages say to their console, warnings and errors among them, to its stdout,
/// every frame's, as WebKitGTK offers: its web view has no other console to read without a window of its own.
#[cfg(all(target_os = "linux", debug_assertions))]
fn console_to_stdout(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use webkit2gtk::{SettingsExt, WebViewExt};
    window.with_webview(|webview| {
        if let Some(settings) = webview.inner().settings() {
            settings.set_enable_write_console_messages_to_stdout(true);
        }
    })
}

/// Ctrl-Q quits and Ctrl-W closes the window, as they do in a Linux app's, before the page hears them. A Mac's
/// Cmd-Q and Cmd-W are its menu's, and Windows' Alt-F4 the system's.
#[cfg(target_os = "linux")]
fn linux_keys(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use gtk::gdk::{ModifierType, keys::constants as key};
    use gtk::glib::Propagation;
    use gtk::prelude::*;
    let closes = window.clone();
    window.gtk_window()?.connect_key_press_event(move |_, pressed| {
        let held = pressed.state() & (ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK | ModifierType::MOD1_MASK | ModifierType::SUPER_MASK);
        if held != ModifierType::CONTROL_MASK {
            return Propagation::Proceed;
        }
        match pressed.keyval().to_lower() {
            key::q => closes.app_handle().exit(0),
            key::w => {
                let _ = closes.close();
            }
            _ => return Propagation::Proceed,
        }
        Propagation::Stop
    });
    Ok(())
}

/// What a computer's app does in every page of its window, which Mac's and Linux's web views leave to it: a browser
/// window's keys and mouse buttons for back and forward, and no menu of the web view's own on a right-click but for
/// text.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) const INITIALIZATION_SCRIPT: &str = include_str!("initialization-script.js");

/// Reports what the window's page saw go wrong (`Client::report`): what it showed its person, a page's error, an
/// app that never drew.
#[tauri::command]
fn report(views: State<'_, Views>, record: Value) {
    views.client.report(record);
}

/// Where this device is with its network, as the window follows it (`follows`), with how many moves it has made:
/// nothing until the client has opened, and a client that couldn't open at all is still opening.
struct Start(tokio::sync::watch::Sender<Option<(u64, Session)>>);

/// Opens the client, gives the window its views, and has the window follow the client's session and connection from
/// then on (`follows`). A client that couldn't open at all, as one whose home can't be read, is said as one still
/// opening that reaches nothing (`start`), which trying again opens again.
async fn opens(app: AppHandle) {
    // A phone gives each app a folder of its own, which only the app is told. An iPhone's app keeps its client in
    // the folder it shares with its notification extension instead, its App Group, which `main.mm` says as ONE_HOME
    // when iOS gives it one, as it does only to an app signed with it, and otherwise in its own (`phone::home`).
    let opened = async {
        #[cfg(target_os = "android")]
        let home = app.path().app_data_dir()?;
        #[cfg(target_os = "ios")]
        let home = phone::home(
            &app.path().app_data_dir()?,
            std::env::var_os("ONE_HOME").as_deref().map(std::path::Path::new),
        )?;
        #[cfg(desktop)]
        let home = ::client::home()?;
        Client::open(home, cli::resolvers(), computer_use_client::tools()).await
    };
    let client = match opened.await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error:#}");
            app.state::<Start>().0.send_modify(|now| *now = Some((now.as_ref().map_or(0, |(seq, _)| seq + 1), Session::Opening)));
            return home(&app);
        }
    };
    eprintln!("the client opened {} ms after the app started", app.state::<Instant>().elapsed().as_millis());
    client.service("app");
    // A move into or out of being a server that the machine went off during is settled as the app opens, whether or
    // not the system still has the server to start (`cli::serving::settle`).
    #[cfg(desktop)]
    tauri::async_runtime::spawn({
        let client = client.clone();
        async move {
            if let Err(error) = cli::serving::settle(&client).await {
                eprintln!("{error:#}")
            }
        }
    });
    app.manage(Views {
        client: client.clone(),
        app: app.clone(),
        subscriptions: Mutex::default(),
        loaded: Mutex::default(),
        started: *app.state::<Instant>(),
        on_screen: Mutex::new(HashSet::from(["main".to_owned()])),
        missed: AtomicBool::new(false),
        hearing: AtomicBool::new(false),
    });

    // The window follows whether its network answers, which is what the last thing asked found, and once it answers
    // again, every view asks again, so it picks up what it missed where it was.
    let (handle, mut connection) = (app.clone(), client.connection());
    tauri::async_runtime::spawn(async move {
        while connection.changed().await.is_ok() {
            let now = *connection.borrow_and_update();
            eprintln!("this client's connection: {now:?}");
            drop(handle.emit("connection", now));
            if now == Connection::Online {
                handle.state::<Views>().refresh().await
            }
        }
    });

    // Where a phone is with its push is what this device says of itself (`client`), so its views ask again as it
    // moves, as Notifications shows it.
    let (handle, mut push) = (app.clone(), notifications::pushes());
    tauri::async_runtime::spawn(async move {
        while push.changed().await.is_ok() {
            let views = handle.state::<Views>();
            let shown: Vec<_> = views.subscriptions.lock().expect("no view panics").keys().cloned().collect();
            for view in shown.iter().filter(|view| *view != "client") {
                drop(handle.emit("changed", view));
            }
        }
    });

    // A server refused this client for being behind it: the window says only that it needs updating, as an app that
    // requires an update does.
    let (handle, mut behind) = (app.clone(), client.behind());
    tauri::async_runtime::spawn(async move {
        while behind.changed().await.is_ok() {
            if let Some(why) = behind.borrow_and_update().clone() {
                eprintln!("this client is behind its servers: {why}");
                drop(handle.emit("behind", why));
            }
        }
    });

    // A server said something changed, so every view on screen asks again, and the window is told which open apps
    // were loaded against a version it says is gone (`stale`). Servers tell this client for as long as it runs, so a
    // notification reaches it with its window hidden.
    let handle = app.clone();
    let mut notifications = client.notifications();
    tauri::async_runtime::spawn(async move {
        while let Ok(message) = notifications.recv().await {
            let views = handle.state::<Views>();
            if message["method"] != "notifications/one/changed" {
                continue;
            }
            let stale = views.stale(&message["params"]);
            if !stale.is_empty() {
                eprintln!("a server said {stale:?} were loaded against what's gone");
                drop(handle.emit("stale", &stale));
            }
            if views.on_screen() {
                eprintln!("a server said something changed");
                views.refresh().await
            } else {
                views.missed.store(true, Ordering::Relaxed)
            }
        }
    });
    client.watch(true).await;
    follows(app, client).await
}

/// Has the window follow the client's session as it moves (`Session`). The window's own page draws each state but
/// one, and hears each move (`session`). The one is signing in, which is the network's own sign-in page with this
/// device's code: a web view has no passkeys, so the page is told it's in the app (`&app`), and shows the code, and
/// its button opens the link in the device's own browser. The window goes there from its own page as it opens, and
/// is brought back to its own page once the device has moved on, which asks where it is then. A device that leaves
/// its network, or is found to be out of it, starts the window again at its own page, as a web app goes back to its
/// start once its person is signed out, which opens from there.
async fn follows(app: AppHandle, client: Arc<Client>) {
    let mut session = client.session();
    loop {
        let now = session.borrow_and_update().clone();
        let seq = app.state::<Start>().0.borrow().as_ref().map_or(0, |(seq, _)| seq + 1);
        eprintln!("this device's session: {now:?}");
        let was = app.state::<Start>().0.send_replace(Some((seq, now.clone())));
        let starts_again = match (&was, &now) {
            (Some((_, Session::SignIn { .. })), _) => true,
            (Some((_, Session::SignedIn)), Session::Opening) => true,
            (Some(_), Session::SignIn { .. }) => true,
            _ => false,
        };
        // A device that isn't in its network lists none of its apps.
        if matches!(now, Session::SignIn { .. }) {
            shortcuts::keep(&app, Vec::new());
        }
        if starts_again {
            home(&app);
        } else {
            drop(app.emit("session", told(seq, &now)));
        }
        if session.changed().await.is_err() {
            return;
        }
    }
}

/// The home screen, as the window is sent to it. Android's and Windows' web views load only http.
#[cfg(any(windows, target_os = "android"))]
pub(crate) const HOME: &str = "http://tauri.localhost/index.html";
#[cfg(not(any(windows, target_os = "android")))]
pub(crate) const HOME: &str = "tauri://localhost/index.html";

/// Opens the window again at its own page, which asks where the device is now, in place of where it was: what it
/// leaves (a code that's been answered) is gone, so the device's own back doesn't lead there. Each app's own window
/// closes, as what it showed was the session's that's gone.
fn home(app: &AppHandle) {
    #[cfg(desktop)]
    if let Some(hosts) = app.try_state::<hosts::Hosts>() {
        hosts.close_all();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(format!("location.replace({})", Value::from(HOME)));
    }
}

/// Whether the window goes where it's sent: to its own pages and its apps', and to the sign-in page it starts on.
/// Anywhere else is the device's own browser's to open, as the sign-in page's button asks.
fn stays(app: &AppHandle, to: &Url) -> bool {
    let own = ["tauri", "oneapp", "about", "data", "blob"].contains(&to.scheme())
        || to.host_str().is_some_and(|host| host.ends_with(".localhost"));
    let start = match &*app.state::<Start>().0.borrow() {
        Some((_, Session::SignIn { code, .. })) => {
            code.link.parse::<Url>().is_ok_and(|start| (start.origin(), start.path()) == (to.origin(), to.path()))
        }
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

/// What of the window is kept for the next time the app opens.
#[cfg(desktop)]
const WINDOW_STATE: tauri_plugin_window_state::StateFlags = tauri_plugin_window_state::StateFlags::SIZE
    .union(tauri_plugin_window_state::StateFlags::POSITION)
    .union(tauri_plugin_window_state::StateFlags::MAXIMIZED)
    .union(tauri_plugin_window_state::StateFlags::FULLSCREEN);

/// Opens the window as it was when the app last quit, on the display it was on. If where it was is on no display now
/// (one that's been unplugged), it opens in the middle of one, at the size it was: the app leaves a position it
/// can't show where it is, so the window is centred at its own size between the two.
#[cfg(desktop)]
fn opens_as_it_was(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use tauri_plugin_window_state::{StateFlags, WindowExt};
    window.restore_state(StateFlags::SIZE | StateFlags::POSITION)?;
    window.center()?;
    window.restore_state(WINDOW_STATE)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started = Instant::now();
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init()).plugin(notifications::plugin());
    // The window keeps where it is, how large, and whether it's maximized or full screen, for the next time the app
    // opens (`opens_as_it_was`).
    // One app at a time: a second start, with a link (`links`) or without, brings the first forward.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _, _| front(app)));
    let builder = builder.plugin(tauri_plugin_deep_link::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_window_state::Builder::new().with_state_flags(WINDOW_STATE).skip_initial_state("main").build());
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_haptics::init());
    builder
        .setup(move |app| {
            app.manage(Start(tokio::sync::watch::Sender::new(None)));
            app.manage(Chosen(Mutex::default()));
            app.manage(started);
            // Windows groups this app's windows under its identifier, and each app's here under its own
            // (`shortcuts::app_id`), which a start from that app's shortcut would otherwise give it.
            #[cfg(windows)]
            unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(&app.config().identifier)) }?;
            // The window opens at its own page at once, which draws the mark until the client has opened behind it,
            // and then follows where the device is: a device that isn't in its network to the page that signs it
            // in. A phone opens the window again at the same address as it's made again (a change of text size),
            // so that's home too, which asks where the device is now.
            tauri::async_runtime::spawn(opens(app.handle().clone()));
            // Each app's own window asks through this one (`hosts`).
            #[cfg(desktop)]
            hosts::serve(app.handle(), &::client::home()?)?;
            links(app)?;
            // Started for an app's own window alone, the Home Screen is there but put away, until it's asked for
            // (`front`).
            let window = window(app.handle())?;
            #[cfg(desktop)]
            if std::env::var_os("ONE_HIDDEN").is_some() {
                window.hide()?;
            }
            #[cfg(mobile)]
            let _ = window;
            eprintln!("the window was made {} ms after the app started", started.elapsed().as_millis());
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
            request,
            refresh,
            upload,
            recovery_key,
            recover,
            client,
            serve,
            update,
            visible,
            close,
            retry,
            reconnect,
            sign_out,
            start,
            push,
            listen,
            push_token,
            tapped,
            opened,
            navigation_gestures,
            status_bar,
            theme,
            report,
            #[cfg(desktop)]
            hosts::open_window,
            #[cfg(desktop)]
            hosts::front
        ])
        .build(context())
        .expect("the app starts")
        .run(|app, event| match event {
            // The Home Screen closed while an app's own window is open is put away, as it routes links and
            // notifications to them, and comes back as the app is opened again (`front`).
            #[cfg(desktop)]
            tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::CloseRequested { api, .. }, .. }
                if label == "main" && app.state::<hosts::Hosts>().any() =>
            {
                api.prevent_close();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            // The Dock's icon clicked with no window of the app's own showing.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { has_visible_windows: false, .. } => front(app),
            _ => {}
        });
}

/// The app's own files and settings, which its page and each app's own window's are served from.
pub(crate) fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

/// The Home Screen's window, made at its own page, which draws the mark until the client has opened behind it, and
/// then follows where the device is: a device that isn't in its network to the page that signs it in. A phone opens
/// the window again at the same address as it's made again (a change of text size), so that's home too, which asks
/// where the device is now.
fn window(app: &AppHandle) -> Result<tauri::WebviewWindow, Box<dyn std::error::Error>> {
    let goes = app.clone();
    let page = HOME.parse().expect("the home screen has an address");
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(page)).on_navigation(move |to| stays(&goes, to));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let window = window.initialization_script_for_all_frames(INITIALIZATION_SCRIPT);
    #[cfg(desktop)]
    let window = window.title(app.package_info().name.clone()).inner_size(1024.0, 720.0).center().build()?;
    #[cfg(desktop)]
    opens_as_it_was(&window)?;
    #[cfg(mobile)]
    let window = window.build()?;
    #[cfg(target_os = "ios")]
    status_bar::watch(&window)?;
    #[cfg(target_os = "ios")]
    edge::watch(&window)?;
    #[cfg(target_os = "linux")]
    linux_keys(&window)?;
    #[cfg(all(target_os = "linux", debug_assertions))]
    console_to_stdout(&window)?;
    navigation_gestures(window.clone(), true, false)?;
    Ok(window)
}

/// Brings the Home Screen forward.
#[cfg(desktop)]
fn front(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
