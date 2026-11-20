//! The app: a window on One Core in a client (`core/client`). Its own page (`page`) draws nothing of its own but its
//! device's session, whether its network answers, and what's under way or wrong with it: it holds the frames. The
//! window always opens there, and follows where the client is with its network (`State`): a device
//! that isn't in its network is sent on to the network's sign-in page, with the code it's let in by, until its person
//! has let it in. Each app is a page of its app's, drawn in a frame of its own, which
//! reaches nothing here but the bridge the window's page gives it: what an app asks comes to `ask`. The launcher
//! is an app too, the one a network lists in the category `launcher`, which the window shows first and under every other.

//!
//! The window is Tauri's on a phone, and on a computer a window and a web view alone (`computer`), as each app's own
//! is (`host`). Either is a `Shell`, which what's here tells and asks of it; what the page asks (`invoke`) is answered
//! by the same commands on both.

#![cfg(feature = "window")]
// What an app's own window asks is answered by the futures the Home Screen's commands are, a deep nest of them (`hosts`).
#![recursion_limit = "256"]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ::client::{Client, Connection, State, Subscription};
use data_encoding::BASE64;
use serde_json::{Value, json};
use url::Url;
#[cfg(mobile)]
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Notifications' native code, which shows them on this device.
use notifications_client as notifications;

pub mod phone;
mod shortcuts;
#[cfg(desktop)]
pub mod computer;
#[cfg(desktop)]
pub mod host;
mod hosts;
mod native;

#[cfg(mobile)]
pub(crate) mod page;
// The app's name, as `tauri.conf.json` has it, and whatever a network's build merges over it (`TAURI_CONFIG`), as a
// phone's Tauri build has it (`build.rs`).
#[cfg(desktop)]
include!(concat!(env!("OUT_DIR"), "/name.rs"));
mod updater;

/// What an app's page may load, which is what an MCP App may by default: its own app's files and the files of
/// the apps it uses, which are all at its own address, and nothing from anywhere else; and in a frame, another app's
/// page at that app's own address (`oneapp://<app>.localhost`, or `http://oneapp.<app>.localhost` where a web view
/// has no scheme of its own), which the window answers as that app's (the UI library's `Frame`).
#[cfg(mobile)]
pub(crate) const APP_MAY_LOAD: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
                            img-src 'self' data:; media-src 'self' data:; font-src 'self' data:; connect-src 'none'; \
                            frame-src oneapp: http://*.localhost; object-src 'none'; base-uri 'self'";

/// The views in the window, by name: the window's own page (`client`) and each app that's open.
struct Views {
    client: Arc<Client>,
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
            let view = view.to_owned();
            Subscription::new(self.client.clone(), move || {
                app().shell.emit("changed", json!(view));
                // The client's list of files is taken once a new version's files are all kept (`keep_whole`): the open
                // apps it no longer names load again then, on it.
                if view == "client" {
                    spawn(async {
                        let Ok(views) = views() else { return };
                        let Ok(listed) = views.listed(None).await else { return };
                        let outdated = views.outdated(None, &listed);
                        if !outdated.is_empty() {
                            eprintln!("{outdated:?} were loaded against a version this client no longer lists");
                            app().shell.emit("stale", json!(outdated));
                        }
                    });
                }
            })
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
            self.refresh(None).await
        }
    }

    /// Asks again for what every view was answered, all at once, or only of `app`'s tools.
    async fn refresh(&self, app: Option<&str>) {
        let shown: Vec<_> = self.subscriptions.lock().expect("no view panics").values().cloned().collect();
        let mut asks = tokio::task::JoinSet::new();
        for shown in shown {
            let app = app.map(str::to_owned);
            asks.spawn(async move { shown.refresh(app.as_deref()).await });
        }
        while asks.join_next().await.is_some() {}
    }

    /// The files of every app a server lists for `subject`, a group or the network, or for this client's user, as MCP
    /// lists resources.
    async fn listed(&self, subject: Option<&str>) -> anyhow::Result<Value> {
        self.subscription("client").request("resources/list", listing(subject)).await
    }

    /// One file a server listed: what kind it is, and what's in it, which is its text or, for a picture or a font,
    /// base64 as MCP's `blob`. An app's is read for whoever it was listed for, `subject`, as a server finds the app
    /// among theirs. A set of an app's files never changes, so a file that's kept is never asked for again.
    async fn read(&self, resource: &str, subject: Option<&str>) -> anyhow::Result<(String, Vec<u8>)> {
        let mut request = listing(subject);
        request["uri"] = json!(resource);
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
    async fn file(&self, address: &str) -> anyhow::Result<(String, Vec<u8>)> {
        let address = Url::parse(address)?;
        let file = address.path().trim_start_matches('/');
        // A large file a tool answered with, which the client keeps, at the address the answer gave an app for it.
        if let Some(path) = file.strip_prefix(".file/") {
            return self.client.file(path).ok_or_else(|| anyhow::anyhow!("this client keeps no such file"));
        }
        let none = || anyhow::anyhow!("no app here has {file}");
        let (name, path) = file.split_once('/').ok_or_else(none)?;
        // `<app>@<version>` is that version's file, as the edge has it for a browser: an icon a page shows, by what's
        // in it (`ui://icons/<sha256>/…`), which never changes.
        if let Some((name, code)) = name.split_once('@') {
            return self.read(&format!("ui://{name}/{code}/{path}"), None).await;
        }
        let app = address.host_str().and_then(|host| host.strip_suffix(".localhost")).unwrap_or_default();
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
        self.read(&resource, subject.as_deref()).await
    }

    /// The open apps loaded against a version of their app, or a build of the UI library, that `changed` says
    /// isn't the one there is now, each as its address names it: a server says the UI library's build (`library`),
    /// and the version each app of the owners whose apps moved runs at (`apps`, by owner). A version that's
    /// still one of them is current, as an app opened for another owner may be on its own, and so is one this client's
    /// list (`listed`) still names, as its new version's files aren't all kept yet: it loads again once they are, as
    /// the list that names them is taken (`subscription`), not before, onto the same version again. Another app's
    /// files an app shows, as the launcher shows every app's icon, aren't what it runs. They have loaded nothing once
    /// this has named them.
    fn stale(&self, changed: &Value, listed: &Value) -> Vec<String> {
        let listed: HashSet<&str> = resources(listed).collect();
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
            .filter(|(app, files)| runs(app, files).any(|(file, name, code)| gone(name, code) && !listed.contains(file)))
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

/// What's the window's own, which differs by device: Tauri's on a phone (`Phone`), and on a computer the launcher's
/// (`computer`). What's here tells it and asks it only this.
pub(crate) trait Shell: Send + Sync + 'static {
    /// The folder the client is kept in.
    fn home(&self) -> anyhow::Result<std::path::PathBuf>;
    /// Tells the window's pages `event`, as Tauri's events: on a computer the launcher's, and each app's own window's
    /// those it hears too (`hosts`).
    fn emit(&self, event: &str, payload: Value);
    /// Opens the window again at its own page (`home`), and on a computer closes each app's own window.
    fn reload(&self);
    /// Opens `url` in the device's own browser.
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// Starts the app again, on the build it was just updated to.
    fn restart(&self);
    /// The `one:` link the window opens once its page asks where it is (`start`): what the app was started with.
    fn opened_at(&self) -> Option<String>;
    /// Has the device hear its person's notifications (`listen`).
    fn listen(&self, client: Arc<Client>);
    /// Light or dark, or as the device is, for what's the window's own and not its page (`theme`).
    fn theme(&self, theme: Option<Theme>);
}

/// Light or dark, as Colour Mode has it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    Light,
    Dark,
}

/// The app as it runs: its window (`Shell`), where the device is with its network (`start`), the Colour Mode chosen
/// on it (`chosen`), when it started, and its views once its client has opened.
pub(crate) struct App {
    shell: Box<dyn Shell>,
    /// Where this device is with its network, as the window follows it (`follows`), with how many moves it has
    /// made: nothing until the client has opened, and a client that couldn't open at all is still opening.
    start: tokio::sync::watch::Sender<Option<(u64, State)>>,
    /// The Colour Mode chosen on this device, as the last page to choose it said, for each page that opens after
    /// (`start`), and told to every page that's open.
    chosen: Mutex<Option<Option<Theme>>>,
    started: Instant,
    views: OnceLock<Views>,
}

static APP: OnceLock<App> = OnceLock::new();

/// The app, once it has started (`begin`).
pub(crate) fn app() -> &'static App {
    APP.get().expect("the app has started")
}

/// Starts the app in `shell`, as it began at `started`.
pub(crate) fn begin(shell: impl Shell, started: Instant) {
    let app = App { shell: Box::new(shell), start: tokio::sync::watch::Sender::new(None), chosen: Mutex::default(), started, views: OnceLock::new() };
    if APP.set(app).is_err() {
        panic!("the app starts once");
    }
}

/// The app's views, once its client has opened.
fn views() -> Result<&'static Views, String> {
    app().views.get().ok_or_else(|| "the app hasn't opened".to_owned())
}

/// Runs `future` on the app's runtime: Tauri's on a phone, and on a computer its own (`computer`).
pub(crate) fn spawn(future: impl std::future::Future<Output = ()> + Send + 'static) {
    #[cfg(mobile)]
    tauri::async_runtime::spawn(future);
    #[cfg(desktop)]
    computer::runtime().spawn(future);
}

/// What asks a server for the files of `subject`'s apps, or this client's user's.
fn listing(subject: Option<&str>) -> Value {
    subject.map_or(json!({}), |subject| json!({"_meta": {"subject": subject}}))
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
/// apps that are `outdated` and load again, each as its address names it. A list is kept only with every file it
/// lists (`Client::fresh`), so with no server up an app opens whole, even one that was never opened. The user's
/// apps with a screen are where the device lists its own apps too (`shortcuts`).
#[cfg_attr(mobile, tauri::command)]
async fn apps(subject: Option<String>) -> Result<Value, String> {
    let views = views()?;
    // A person is told what went wrong, and not what was behind it.
    let mut listed = views.listed(subject.as_deref()).await.map_err(|error| error.to_string())?;
    listed["outdated"] = views.outdated(subject.as_deref(), &listed).into();
    if subject.is_none() {
        shortcuts::keep(shortcuts::listed(&listed));
    }
    Ok(listed)
}

/// Answers what a view asks, in MCP's words: the result of `method`.
#[cfg_attr(mobile, tauri::command)]
async fn request(view: String, method: String, params: Value) -> Result<Value, String> {
    views()?.subscription(&view).request(&method, params).await.map_err(|error| error.to_string())
}

/// Asks again for what a view was answered: its user returned to it.
#[cfg_attr(mobile, tauri::command)]
async fn refresh(view: String) -> Result<(), String> {
    views()?.subscription(&view).refresh(None).await;
    Ok(())
}

/// This device's key, as its network knows it, the network's address, if it has one, the build it's on and its kind
/// of machine, as builds are named, its server's key, if it's a server, and whether a phone is woken by its
/// notifications (`notifications::Push`), whether it opens its own Settings page for the app (`settings`, an iPhone's;
/// Android's is opened by its activity), and whether its person turned Computer Use on, on a phone that has it.
#[cfg_attr(mobile, tauri::command)]
fn client() -> Result<Value, String> {
    let views = views()?;
    Ok(json!({
        "device": views.client.key(),
        "network": views.client.address(),
        "build": ::client::BUILD,
        "target": ::client::target(),
        "server": cli::serving::server(),
        "push": notifications::push(),
        "settings": cfg!(target_os = "ios"),
        "computerUse": computer_use_client::phone(),
    }))
}

/// Makes this device a server of its person's, or with `on` false, no longer one, as `one serve on` and `one serve
/// off` do. Gives what they say.
#[cfg_attr(mobile, tauri::command)]
async fn serve(on: bool) -> Result<String, String> {
    let views = views()?;
    let done = if on { cli::serving::on(&views.client).await } else { cli::serving::off(&views.client).await };
    done.map_err(|error| format!("{error:#}"))
}

/// Takes the latest builds for this device, as `one update` does, and starts again on them, as an app does once
/// it has updated. Gives what it says, which it gives only where it isn't starting again (a restart never returns):
/// the window then asks its network whether it's still behind. On a device that's also a server, the program keeping its server running
/// takes them, this program's among them, and says where that is in `builds`: this starts again once its file is a
/// newer build than it runs. An iPhone's app from TestFlight (`testflight`) opens TestFlight instead, which updates it.
#[cfg_attr(mobile, tauri::command)]
async fn update() -> Result<String, String> {
    if testflight() {
        app().shell.open_url("itms-beta://")?;
        return Ok(String::new());
    }
    let message = cli::serving::update(&views()?.client).await.map_err(|error| format!("{error:#}"))?;
    if cli::serving::server().is_none() || cli::serving::replaced() {
        app().shell.restart()
    }
    Ok(message)
}

/// Says whether the Home Screen's window is on screen. While one is, its views ask again when something changed, and
/// as one's back, if something changed while none was.
#[cfg(mobile)]
#[tauri::command]
async fn visible(on_screen: bool) -> Result<(), String> {
    let views = views()?;
    views.shown("main", on_screen);
    if on_screen {
        updater::looked();
    }
    spawn(updater::ready());
    views.caught_up().await;
    Ok(())
}

/// Says whether the Home Screen's window holds something its person typed and hasn't sent (`updater`).
#[cfg(mobile)]
#[tauri::command]
fn editing(editing: bool) {
    updater::editing("main", editing)
}

/// Sends a file an app's page chose, which is the request's body, to where the network keeps it, for a tool of
/// the app's app. Gives the name that tool takes for it.
#[cfg(mobile)]
#[tauri::command]
async fn upload(request: tauri::ipc::Request<'_>) -> Result<Value, String> {
    use tauri::ipc::InvokeBody;
    let app = request.headers().get("app").and_then(|app| app.to_str().ok()).unwrap_or_default().to_owned();
    let file = match request.body() {
        InvokeBody::Raw(file) => file.clone(),
        // Android's web view shows nothing a request's body, so there the file comes as a list of its bytes.
        InvokeBody::Json(Value::Array(bytes)) => bytes.iter().filter_map(|byte| Some(byte.as_u64()? as u8)).collect(),
        InvokeBody::Json(_) => return Err("An upload is a file".into()),
    };
    upload_file(&app, file).await
}

/// Sends `file` where the network keeps it, for a tool of `app`'s (`upload`).
async fn upload_file(app: &str, file: Vec<u8>) -> Result<Value, String> {
    let sent = views()?.client.upload(app, file).await;
    sent.map(|upload| json!({"upload": upload})).map_err(|error| error.to_string())
}

/// The recovery key of this device's person, or of a group or the network an app was opened for (`subject`), for the
/// client's own page to show them. It's that page that shows it, so no app's page is handed one.
#[cfg_attr(mobile, tauri::command)]
async fn recovery_key(subject: Option<String>) -> Result<String, String> {
    encryption_client::recovery_key(&views()?.client, subject.as_deref()).await.map_err(|error| error.to_string())
}

/// Lets this device hold its person's key, or `subject`'s, with the recovery key they typed on the client's own page,
/// and has every view ask again.
#[cfg_attr(mobile, tauri::command)]
async fn recover(key: String, subject: Option<String>) -> Result<(), String> {
    let views = views()?;
    encryption_client::recover(&views.client, &key, subject.as_deref()).await.map_err(|error| error.to_string())?;
    views.refresh(None).await;
    Ok(())
}

/// Forgets what a view was answered: its page is gone, or loading again.
#[cfg_attr(mobile, tauri::command)]
fn close(view: String) -> Result<(), String> {
    views()?.subscriptions.lock().expect("no view panics").remove(&view);
    Ok(())
}

/// Asks the network again now, which didn't answer or refused this client for being behind it, as its person does
/// with Try Again (`Client::retry`), and answers once it has, failing where it still isn't answered. A client that
/// couldn't open at all is opened again.
#[cfg_attr(mobile, tauri::command)]
async fn retry() -> Result<(), String> {
    let Ok(views) = views() else {
        spawn(opens());
        return Ok(());
    };
    // A computer's launcher, which said Update Required for being behind, shows the Home Screen again.
    #[cfg(desktop)]
    let behind = matches!(views.client.state().borrow().connection(), Connection::Behind(_));
    views.client.retry().await.map_err(|error| error.to_string())?;
    #[cfg(desktop)]
    if behind {
        computer::talks();
    }
    Ok(())
}

/// Names whose this device is, while it signs in, which asks their devices to let it in, as the network's sign-in page
/// does (`/device/claim`, apps/sign-in/edge/auth.ts). → the number they enter on one of them, or what the network
/// said instead (nobody goes by that, asked too often).
async fn claim(person: String) -> Result<Value, String> {
    let Some((_, State::SignIn { code, .. })) = app().start.borrow().clone() else { return Err("This device is signed in".into()) };
    let link = Url::parse(&code.link).map_err(|error| error.to_string())?;
    let user_code = link.query_pairs().find(|(name, _)| name == "user_code").map(|(_, code)| code.into_owned()).unwrap_or_default();
    let at = link.join("/sign-in/auth/device/claim").map_err(|error| error.to_string())?;
    let web = ::client::web().map_err(|error| format!("{error:#}"))?;
    let response = web.post(at).json(&json!({"userCode": user_code, "person": person})).send().await.map_err(|_| "Can't reach your network. Try again.".to_owned())?;
    let ok = response.status().is_success();
    let said: Value = response.json().await.unwrap_or_default();
    if !ok {
        return Err(said["message"].as_str().unwrap_or("That didn't work. Try again.").to_owned());
    }
    Ok(json!({"number": said["number"]}))
}

/// The device is back online or the window in front again, so a client whose network didn't answer asks it again
/// now (`Client::reconnect`).
#[cfg_attr(mobile, tauri::command)]
fn reconnect() -> Result<(), String> {
    views()?.client.reconnect();
    Ok(())
}

/// Signs this device out once its person has removed it from their devices (Devices' Sign Out): what it kept of
/// theirs goes, and the window follows it to the sign-in page.
#[cfg_attr(mobile, tauri::command)]
async fn sign_out() -> Result<(), String> {
    views()?.client.sign_out().await.map_err(|error| format!("{error:#}"))?;
    native::forget_snapshots();
    Ok(())
}

/// Where this device is with its network (`state`), as the window's page asks once it has opened, which waits for the
/// client to open, and whether a phone hears a tap on a notification, through its push plugin, which the page listens
/// to. The page follows where it is from there (`state`). A client that couldn't open at all has reached nothing.
#[cfg_attr(mobile, tauri::command)]
async fn start() -> Result<Value, String> {
    let app = app();
    let mut session = app.start.subscribe();
    let waited = session.wait_for(Option::is_some).await.map_err(|_| "the app has ended".to_owned())?;
    let (seq, session) = waited.clone().expect("it waited for one");
    let mut told = json!({"state": state(seq, &session)});
    told["taps"] = json!(cfg!(mobile));
    told["testflight"] = json!(testflight());
    // A computer opens each app in a window of its own (`hosts`).
    told["windows"] = json!(cfg!(desktop));
    // A phone draws an app's window natively, over the page, where it needs this program's runtime version or an
    // older one (`renderer`).
    #[cfg(mobile)]
    {
        told["runtimeVersion"] = json!(::renderer::RUNTIME_VERSION);
    }
    if let Some(theme) = *app.chosen.lock().expect("nothing panics holding it") {
        told["theme"] = json!(theme);
    }
    told["open"] = json!(app.shell.opened_at());
    Ok(told)
}

/// Once the device is signed in to its network, which it's told only once its views are open (`follows`).
#[cfg(desktop)]
/// Waits until this device's session is settled: in its network, or signing in, which it is once its client has opened
/// and asked.
#[cfg(desktop)]
pub(crate) async fn settled() {
    let mut session = app().start.subscribe();
    let _ = session.wait_for(|now| matches!(now, Some((_, State::SignedIn { .. } | State::SignIn { .. })))).await;
}

pub(crate) async fn signed_in() {
    let mut session = app().start.subscribe();
    let _ = session.wait_for(|now| matches!(now, Some((_, State::SignedIn { .. })))).await;
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
pub(crate) fn link(url: &str) -> Option<String> {
    url.strip_prefix("one:").map(str::to_owned)
}

/// Opens what each link the device hands the app links to, over whatever's showing, as a tap on a notification
/// does, and brings the window forward. The link the app was started with is the page's to ask for (`start`). A
/// computer's program that isn't a developer's build takes the device's `one:` links: Linux and Windows have a
/// program say so itself (a Mac's and an iPhone's say so in their bundles, and Android's in its manifest). A phone
/// takes some of its network's own links too, its app links (`APP_LINKS` in `apps/sign-in/edge/wrangler.jsonc`).
#[cfg(mobile)]
fn links(app: &tauri::App) -> tauri::Result<()> {
    use tauri_plugin_deep_link::DeepLinkExt;
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            open_link(url.to_string());
        }
    });
    Ok(())
}

/// Opens what `url` links to: a `one:` link says so itself, and one of the network's own pages leads there, as the
/// network's edge sends it on to the web client's link to an app's place (`/?open=<one: link>`), as a request's link
/// in Vault does (`apps/vault/edge/worker.ts`).
#[cfg(mobile)]
fn open_link(url: String) {
    if let Some(opens) = link(&url) {
        return app().shell.emit("open", json!(opens));
    }
    spawn(async move {
        let led = async { anyhow::Ok(::client::web()?.get(&url).send().await?.url().clone()) };
        match led.await {
            Ok(to) => match to.query_pairs().find(|(name, _)| name == "open") {
                Some((_, opens)) => app().shell.emit("open", json!(opens)),
                None => eprintln!("{url} leads to no app's place, but {to}"),
            },
            Err(error) => eprintln!("couldn't learn where {url} leads: {error:#}"),
        }
    });
}

/// Where this device is now, as the window's page hears it (`state`): its session, the session's place in the order
/// the page hears them in, whether its network answers or its servers say it's behind them, and while the device is
/// signing in, where the window goes: the network's sign-in page with its code, told it's in the app (`&app`), which
/// has no passkeys, and why it shows a new code, if it says.
fn state(seq: u64, session: &State) -> Value {
    let link = match session {
        State::SignIn { code, why } => code.link.parse::<Url>().ok().map(|mut link| {
            link.query_pairs_mut().append_key_only("app");
            if let Some(why) = why {
                link.query_pairs_mut().append_pair("why", why.why());
            }
            link.to_string()
        }),
        _ => None,
    };
    json!({"seq": seq, "session": session, "link": link, "connection": session.connection()})
}

/// Has a phone whose push failed try again now (`notifications::Push`), as its person does in Notifications, and
/// says where it is once that try has ended.
#[cfg_attr(mobile, tauri::command)]
async fn push() -> Result<Value, String> {
    Ok(json!({"message": format!("{:?}", notifications::retry().await)}))
}

/// Opens this app's page in a phone's Settings, where its person allows what only they may, as an iPhone has it
/// (`client({ settings: "app" })`: Android's is opened by its activity).
#[cfg(mobile)]
#[tauri::command]
fn settings() -> Result<(), String> {
    app().shell.open_url("app-settings:")
}

/// Has this device hear its person's notifications, once their home is on screen: a phone asks them then whether
/// it may show any, over what they came for and not over signing in. Each time after, as its home is back in front,
/// a phone looks again at whether it may (`notifications::Push`), as its person may have allowed them in its Settings.
#[cfg_attr(mobile, tauri::command)]
fn listen() -> Result<(), String> {
    let views = views()?;
    if views.hearing.swap(true, Ordering::Relaxed) {
        notifications::look_again();
        return Ok(());
    }
    app().shell.listen(views.client.clone());
    Ok(())
}

/// A phone's push service gave it a push token, as its push plugin said to the window's page (`push-token`), which
/// it registers if it isn't the one it registered.
#[cfg(mobile)]
#[tauri::command]
fn push_token(token: String) {
    notifications::token(token);
}

/// A notification was tapped on a phone, which its push plugin said to the window's page (`notificationClicked`), so
/// the window opens the app it's from, as a tap on a computer's does.
#[cfg(mobile)]
#[tauri::command]
fn tapped(tapped: Value) {
    if let Some(name) = notifications::opens(&tapped) {
        app().shell.emit("open", json!(name));
    }
}

/// Says how long a view took to open: an app from the tap on its icon to its content on screen, and the home
/// screen from when its page began to load, which the time since the program started is beside. Telemetry is told.
/// Opens an app's window natively over the page, where its page is of a version this program speaks
/// (`hosts::open_window`), from its icon at `icon` (x, y, width and height, in points), where it was opened from one.
/// → whether it did; if not, the page opens it in a frame, as it needs a newer host. One that should have and didn't
/// is an error, which the page shows as its problem.
#[cfg(mobile)]
#[tauri::command]
async fn open_window(entrypoint: String, subject: Option<String>, link: String, icon: Option<[f32; 4]>) -> Result<bool, String> {
    let icon = icon.map(|[x, y, width, height]| renderer::geometry::Bounds { origin: renderer::geometry::point(x, y), size: renderer::geometry::Size { width, height } });
    hosts::open_window(entrypoint, subject, link, icon).await
}

/// Opens the launcher's page natively, as the phone's root window, under every app's (`native::computer::open`).
/// → whether it did; if not, the page shows the web launcher.
#[cfg(mobile)]
#[tauri::command]
async fn open_launcher(entrypoint: String) -> bool {
    native::computer::open(entrypoint).await
}

/// Whether the launcher's root window shows: not while the page shows an app over it, which it draws itself.
#[cfg(mobile)]
#[tauri::command]
fn launcher_shown(shown: bool) {
    native::computer::shown(shown);
}

#[cfg_attr(mobile, tauri::command)]
fn opened(view: String, ms: u32) -> Result<(), String> {
    let views = views()?;
    eprintln!("{view} opened in {ms} ms, {} ms after the app started", views.started.elapsed().as_millis());
    views.client.report(json!({"name": "one.client.open.duration", "value": ms, "attributes": {"one.view": view}}));
    Ok(())
}

/// Turns the device's own back and forward gestures on, or off: an iPhone's swipe in from the left edge, a Mac's
/// two-finger swipe, and a Linux touchpad's. They go back and forward through the window's history, as its Back buttons
/// do, and its page has them off while something is open over the app that's showing, which they'd go back behind, and
/// at an app's first screen, where the swipe takes the app home (`home`), which a computer's page does itself. An
/// iPhone's is always the window's own (`edge`): WebKit's would go forward from the right edge too, where the rail's
/// line is. A web view has them off unless it's asked; Android's has none, and its back is `MainActivity`'s.
#[cfg(mobile)]
#[tauri::command]
fn navigation_gestures(window: tauri::WebviewWindow, on: bool, home: bool) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        let _ = window;
        edge::take(on || home);
        Ok(())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = home;
        window
            .with_webview(move |webview| {
                // Android's back is `MainActivity`'s.
                let _ = (webview, on);
            })
            .map_err(|error| error.to_string())
    }
}

/// Shows or hides an iPhone's status bar, as the app that's showing asks: its Home Screen hides it while it's edited, as
/// iOS's own does, and puts + and Done where it was. Tao's view controller says whether it's hidden, and has iOS ask
/// again as it changes. A computer has none, and Android's window is already clear of its bars.
#[cfg(mobile)]
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
/// device and show dark on dark (`Shell::theme`). Android's bars are `MainActivity`'s (`androidTheme` in host.js).
/// Every page is told (`chose`), as each of a computer's app windows has a page of its own.
#[cfg_attr(mobile, tauri::command)]
fn theme(theme: Option<Theme>) {
    chose(theme);
    app().shell.theme(theme);
}

/// Keeps the Colour Mode chosen on this device for each page that opens after (`start`), and tells every page that's
/// open.
fn chose(theme: Option<Theme>) {
    *app().chosen.lock().expect("nothing panics holding it") = Some(theme);
    app().shell.emit("theme", json!(theme));
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

/// An iPhone's swipe in from the left edge goes back as its Back does: at an app's first screen, the page's move home
/// follows the finger as far across the window as it's come (`grab`, `drag` and `release` in client.js), as a pop
/// does, and let go, it carries on if it's past halfway or flicked, and otherwise springs back; deeper in, let go far
/// enough, it goes back a screen. It's on wherever the page says back has somewhere to go (`navigation_gestures`).
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

/// Reports what the window's page saw go wrong (`Client::report`): what it showed its person, a page's error, an
/// app that never drew.
#[cfg_attr(mobile, tauri::command)]
fn report(record: Value) -> Result<(), String> {
    views()?.client.report(record);
    Ok(())
}

/// Opens the client, gives the window its views, and has the window follow the client's session and connection from
/// then on (`follows`). A client that couldn't open at all, as one whose home can't be read, is said as one still
/// opening that reaches nothing (`start`), which trying again opens again.
async fn opens() {
    let app = app();
    let client = match async { open(app.shell.home()?).await }.await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error:#}");
            let state = State::Opening { connection: Connection::Offline };
            app.start.send_modify(|now| *now = Some((now.as_ref().map_or(0, |(seq, _)| seq + 1), state)));
            return app.shell.reload();
        }
    };
    eprintln!("the client opened {} ms after the app started", app.started.elapsed().as_millis());
    #[cfg(desktop)]
    computer::opened();
    // A move into or out of being a server that the machine went off during is settled as the app opens, whether or
    // not the system still has the server to start (`cli::serving::settle`).
    #[cfg(desktop)]
    spawn({
        let client = client.clone();
        async move {
            if let Err(error) = cli::serving::settle(&client).await {
                eprintln!("{error:#}")
            }
        }
    });
    let views = Views {
        client: client.clone(),
        subscriptions: Mutex::default(),
        loaded: Mutex::default(),
        started: app.started,
        on_screen: Mutex::new(HashSet::from(["main".to_owned()])),
        missed: AtomicBool::new(false),
        hearing: AtomicBool::new(false),
    };
    if app.views.set(views).is_err() {
        return;
    }
    let views = app.views.get().expect("it was just set");
    // A newer build or page, which it's kept on from now on (`updater`).
    updater::start();

    // Where a phone is with its push is what this device says of itself (`client`), so its views ask again as it
    // moves, as Notifications shows it.
    let mut push = notifications::pushes();
    spawn(async move {
        while push.changed().await.is_ok() {
            let shown: Vec<_> = views.subscriptions.lock().expect("no view panics").keys().cloned().collect();
            for view in shown.iter().filter(|view| *view != "client") {
                app.shell.emit("changed", json!(view));
            }
        }
    });

    // A server said something changed, so every view on screen asks again, and the window is told which open apps
    // were loaded against a version it says is gone (`stale`). Servers tell this client for as long as it runs, so a
    // notification reaches it with its window hidden.
    let mut notifications = client.notifications();
    spawn(async move {
        while let Ok(message) = notifications.recv().await {
            if message["method"] == "notifications/one/update" {
                spawn(updater::heard(message["params"].clone()));
                continue;
            }
            if message["method"] != "notifications/one/changed" {
                continue;
            }
            let listed = views.listed(None).await.unwrap_or_default();
            let stale = views.stale(&message["params"], &listed);
            if !stale.is_empty() {
                eprintln!("a server said {stale:?} were loaded against what's gone");
                app.shell.emit("stale", json!(stale));
            }
            if views.on_screen() {
                eprintln!("a server said something changed");
                let app = message["params"]["app"].as_str();
                if app.is_none() {
                    views.client.refresh_listed();
                }
                views.refresh(app).await
            } else {
                views.missed.store(true, Ordering::Relaxed)
            }
        }
    });
    // Its servers are told it's watching behind it: that connects to each of them, and waits for the slowest, which is
    // no reason for a window to wait to start its page, which waits for its session (`start`): 3 to 9 s on the
    // owner's Mac, as one of its servers restarted (20 Nov).
    let watching = client.clone();
    spawn(async move { watching.watch(true).await });
    follows(client).await
}

/// The app's client, opened once for its process, kept in `home`: by its window as it opens, or before that by a push
/// that woke it (`wake`), whose client the window then has.
async fn open(home: std::path::PathBuf) -> anyhow::Result<Arc<Client>> {
    static CLIENT: tokio::sync::OnceCell<Arc<Client>> = tokio::sync::OnceCell::const_new();
    let open = || async {
        #[allow(unused_mut)]
        let mut tools = computer_use_client::tools();
        // A native window's inspector, which is this program's own.
        tools.push(Arc::new(native::Inspector));
        let client = Client::open(home, cli::resolvers(), tools).await?;
        client.service("app");
        anyhow::Ok(client)
    };
    CLIENT.get_or_try_init(open).await.cloned()
}

/// Starts the app's client without its window, for Android's messaging service (`Notifications.kt`), which a push that
/// wakes the phone starts, as a server asks its app something while it isn't running (Notifications' hook `wake`), and
/// for Computer Use's accessibility service, which Android keeps running while it's on (`ComputerUse.kt`):
/// it opens the client kept in `home`, the app's folder, as the window would, and says it's watching to each server,
/// which the one that woke it waits for before it asks. It returns once it has, or couldn't, within the time Android
/// gives a high-priority message, and the client stays for as long as the app's process does, as it would once its
/// window was closed. `context` is the app's.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_client_Notifications_wake<'local>(
    mut env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
    context: jni::objects::JObject<'local>,
    home: jni::objects::JString<'local>,
) {
    let started = Instant::now();
    let mut given = || -> anyhow::Result<std::path::PathBuf> {
        let application = env.call_method(&context, "getApplicationContext", "()Landroid/content/Context;", &[])?.l()?;
        // Kept until the process ends, as what it gives native code has to be.
        let application = env.new_global_ref(application)?;
        let vm = env.get_java_vm()?.get_java_vm_pointer();
        unsafe { ::client::started_without_window(vm.cast(), application.as_obj().as_raw().cast()) };
        std::mem::forget(application);
        Ok(String::from(env.get_string(&home)?).into())
    };
    let home = match given() {
        Ok(home) => home,
        Err(error) => return eprintln!("the app's client couldn't start without its window: {error:#}"),
    };
    // Android gives a high-priority message about 10 seconds before it may stop the process.
    let woken = tauri::async_runtime::block_on(async {
        let watching = async {
            let client = open(home).await?;
            let mut state = client.state();
            state.wait_for(|state| !matches!(state, State::Opening { .. })).await?;
            client.watch(true).await;
            anyhow::Ok(())
        };
        tokio::time::timeout(std::time::Duration::from_secs(9), watching).await
    });
    match woken {
        Ok(Ok(())) => eprintln!("the app's client started without its window in {} ms", started.elapsed().as_millis()),
        Ok(Err(error)) => eprintln!("the app's client couldn't start without its window: {error:#}"),
        Err(_) => eprintln!("the app's client didn't start without its window within 9 seconds"),
    }
}

/// Has the window follow where the client is with its network as that moves (`State`), each move of which its page
/// hears (`state`): its session, in order, whether its network answers, and its servers saying it's behind them. The
/// page draws each session but one. The one is signing in, which is the network's own sign-in page with this device's
/// code: a web view has no passkeys, so the page is told it's in the app (`&app`), and shows the code, and its button
/// opens the link in the device's own browser. The window goes there from its own page as it opens, and is brought
/// back to its own page once the device has moved on, which asks where it is then. A device that leaves its network,
/// or is found to be out of it, starts the window again at its own page, as a web app goes back to its start once its
/// person is signed out, which opens from there. Once its network answers again, every view asks again, so it picks up
/// what it missed where it was; and behind its servers, the window says only that it needs updating, as an app that
/// requires an update does.
async fn follows(client: Arc<Client>) {
    let app = app();
    let mut state = client.state();
    loop {
        let now = state.borrow_and_update().clone();
        let was = app.start.borrow().clone();
        let seq = match &was {
            Some((seq, was)) if was.same_session(&now) => *seq,
            Some((seq, _)) => seq + 1,
            None => 0,
        };
        app.start.send_replace(Some((seq, now.clone())));
        let was = was.map(|(_, was)| was);
        let moved = was.as_ref().is_none_or(|was| !was.same_session(&now));
        let starts_again = match (&was, &now) {
            (Some(State::SignIn { .. }), _) => true,
            (Some(State::SignedIn { .. }), State::Opening { .. }) => true,
            (Some(_), State::SignIn { .. }) => true,
            _ => false,
        };
        if moved {
            eprintln!("this device's session: {now:?}");
        }
        // A device that isn't in its network lists none of its apps.
        if moved && matches!(now, State::SignIn { .. }) {
            shortcuts::keep(Vec::new());
            native::forget_snapshots();
        }
        // The page was told where it is as it started (`start`), and hears each change from there.
        if starts_again {
            app.shell.reload();
        } else if was.as_ref() != Some(&now) {
            app.shell.emit("state", self::state(seq, &now));
        }
        let (connection, before) = (now.connection(), was.map(|was| was.connection()));
        if let Some(before) = before.filter(|before| *before != connection) {
            eprintln!("this client's connection: {connection:?}");
            if connection.answers()
                && !before.answers()
                && let Some(views) = app.views.get()
            {
                // What it missed: what every view was answered, and a newer build or page (`updater`).
                spawn(updater::caught_up());
                spawn(views.refresh(None));
            }
        }
        if state.changed().await.is_err() {
            return;
        }
    }
}

/// The home screen, as a phone's window is sent to it. Android's web view loads only http.
#[cfg(target_os = "android")]
pub(crate) const HOME: &str = "http://tauri.localhost/index.html";
#[cfg(target_os = "ios")]
pub(crate) const HOME: &str = "tauri://localhost/index.html";

/// Whether the window goes where it's sent: to its own pages and its apps', and to the sign-in page it starts on.
/// Anywhere else is the device's own browser's to open, as the sign-in page's button asks.
#[cfg(mobile)]
pub(crate) fn stays(to: &str) -> bool {
    let Ok(to) = Url::parse(to) else { return false };
    let own = ["tauri", "oneapp", "about", "data", "blob"].contains(&to.scheme())
        || to.host_str().is_some_and(|host| host.ends_with(".localhost"));
    let start = match &*app().start.borrow() {
        Some((_, State::SignIn { code, .. })) => {
            code.link.parse::<Url>().is_ok_and(|start| (start.origin(), start.path()) == (to.origin(), to.path()))
        }
        _ => false,
    };
    if !own && !start {
        // A phone opens it from its own thread, which is the one asking here, so it's asked from another.
        let to = to.to_string();
        std::thread::spawn(move || {
            if let Err(error) = app().shell.open_url(&to) {
                eprintln!("couldn't open {to} in the device's browser: {error}");
            }
        });
    }
    own || start
}

/// The app on a computer: the launcher and its client (`computer`).
#[cfg(desktop)]
pub fn run() {
    computer::run()
}

/// A phone's window, Tauri's: what's here tells it and asks it (`Shell`).
#[cfg(mobile)]
struct Phone(AppHandle);

#[cfg(mobile)]
impl Shell for Phone {
    /// A phone gives each app a folder of its own, which only the app is told. An iPhone's app keeps its client in
    /// the folder it shares with its notification extension instead, its App Group, which `main.mm` says as ONE_HOME
    /// when iOS gives it one, as it does only to an app signed with it, and otherwise in its own (`phone::home`).
    fn home(&self) -> anyhow::Result<std::path::PathBuf> {
        #[cfg(target_os = "android")]
        return Ok(self.0.path().app_data_dir()?);
        #[cfg(target_os = "ios")]
        return phone::home(&self.0.path().app_data_dir()?, std::env::var_os("ONE_HOME").as_deref().map(std::path::Path::new));
    }

    fn emit(&self, event: &str, payload: Value) {
        // An app's native window over the page hears what an app's own window would on a computer.
        if hosts::EVENTS.contains(&event) {
            hosts::tell_all(json!({"event": event, "payload": payload}));
        }
        drop(self.0.emit(event, payload));
    }

    fn reload(&self) {
        hosts::close_all();
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.eval(format!("location.replace({})", Value::from(HOME)));
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        use tauri_plugin_opener::OpenerExt;
        self.0.opener().open_url(url, None::<&str>).map_err(|error| error.to_string())
    }

    fn restart(&self) {
        self.0.restart()
    }

    fn opened_at(&self) -> Option<String> {
        use tauri_plugin_deep_link::DeepLinkExt;
        let started = self.0.deep_link().get_current().ok().flatten().unwrap_or_default();
        let opens = started.iter().find_map(|url| link(url.as_str()));
        // One of the network's own links is opened once it's learned where it leads.
        if opens.is_none() {
            started.iter().for_each(|url| open_link(url.to_string()));
        }
        opens
    }

    fn listen(&self, client: Arc<Client>) {
        spawn(notifications::listen(self.0.clone(), client));
    }

    /// An iPhone's status bar, which would otherwise follow the device and show dark on dark. Android's bars are
    /// `MainActivity`'s (`androidTheme` in host.js).
    fn theme(&self, theme: Option<Theme>) {
        #[cfg(target_os = "ios")]
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.with_webview(move |webview| {
                use objc2_ui_kit::{UIUserInterfaceStyle, UIView};
                // Tauri hands over the window's WKWebView on the main thread.
                let web = unsafe { &*webview.inner().cast::<UIView>() };
                let style = match theme {
                    Some(Theme::Light) => UIUserInterfaceStyle::Light,
                    Some(Theme::Dark) => UIUserInterfaceStyle::Dark,
                    None => UIUserInterfaceStyle::Unspecified,
                };
                if let Some(window) = web.window() {
                    window.setOverrideUserInterfaceStyle(style);
                }
            });
        }
        let _ = theme;
    }
}

#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn run() {
    use tauri::http::{Response, StatusCode, header};
    let started = Instant::now();
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init()).plugin(notifications::plugin());
    let builder = builder.plugin(tauri_plugin_deep_link::init());
    #[cfg(target_os = "ios")]
    let builder = builder.plugin(tauri_plugin_haptics::init());
    builder
        .setup(move |app| {
            // An app's window is drawn natively, in a tao window over this one (`native`).
            #[cfg(target_os = "ios")]
            app.wry_plugin(native::Ios);
            begin(Phone(app.handle().clone()), started);
            // The window opens at its own page at once, which draws the mark until the client has opened behind it,
            // and then follows where the device is: a device that isn't in its network to the page that signs it
            // in. A phone opens the window again at the same address as it's made again (a change of text size),
            // so that's home too, which asks where the device is now.
            spawn(opens());
            links(app)?;
            window(app.handle())?;
            eprintln!("the window was made {} ms after the app started", started.elapsed().as_millis());
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("oneapp", |_, request, responder| {
            spawn(async move {
                let file = match views() {
                    Ok(views) => views.file(&request.uri().to_string()).await.map_err(|error| format!("{error:#}")),
                    Err(error) => Err(error),
                };
                let response = match file {
                    Ok((kind, file)) => Response::builder()
                        .header(header::CONTENT_TYPE, kind)
                        .header(header::CONTENT_SECURITY_POLICY, APP_MAY_LOAD)
                        .header(header::CACHE_CONTROL, "no-store")
                        .body(file),
                    Err(error) => Response::builder().status(StatusCode::NOT_FOUND).body(error.into_bytes()),
                };
                responder.respond(response.expect("these headers are valid"))
            });
        })
        .invoke_handler(tauri::generate_handler![
            apps,
            open_window,
            open_launcher,
            launcher_shown,
            request,
            refresh,
            upload,
            recovery_key,
            recover,
            client,
            serve,
            update,
            visible,
            editing,
            close,
            retry,
            reconnect,
            sign_out,
            start,
            push,
            settings,
            listen,
            push_token,
            tapped,
            opened,
            navigation_gestures,
            status_bar,
            theme,
            report,
        ])
        .run(tauri::generate_context!())
        .expect("the app starts");
}

/// The Home Screen's window, made at its own page, which draws the mark until the client has opened behind it, and
/// then follows where the device is: a device that isn't in its network to the page that signs it in. A phone opens
/// the window again at the same address as it's made again (a change of text size), so that's home too, which asks
/// where the device is now.
#[cfg(mobile)]
fn window(app: &AppHandle) -> Result<tauri::WebviewWindow, Box<dyn std::error::Error>> {
    let page = HOME.parse().expect("the home screen has an address");
    // Tauri answers with the page built in, and the network's kept one goes in its place (`page`).
    if let Ok(home) = self::app().shell.home() {
        page::load(&home);
    }
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(page))
        .on_navigation(|to| stays(to.as_str()))
        .on_web_resource_request(|request, response| {
            use tauri::http::{HeaderValue, StatusCode, header};
            let path = request.uri().path();
            if let Some(file) = page::kept(path) {
                *response.status_mut() = StatusCode::OK;
                response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(page::kind(path)));
                *response.body_mut() = file.into();
            }
        });
    let window = window.build()?;
    #[cfg(target_os = "ios")]
    status_bar::watch(&window)?;
    #[cfg(target_os = "ios")]
    edge::watch(&window)?;
    navigation_gestures(window.clone(), true, false)?;
    Ok(window)
}
