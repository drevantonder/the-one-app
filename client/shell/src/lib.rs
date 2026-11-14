//! The app: a window on the client's Rust half. Its own page (`page`) is the home screen. Each app is a page of its
//! plugin's, drawn in a frame of its own, which reaches nothing here but the bridge the home screen's page gives it:
//! what an app asks comes to `ask`, as what the home screen asks does.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use client::{Client, Shown};
use serde_json::{Value, json};
use tauri::http::{Response, StatusCode, Uri, header};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// What an app's page may load, which is what an MCP App may by default: its own plugin's files and the files of
/// the plugins it uses, which are all at its own address, and nothing from anywhere else.
const APP_MAY_LOAD: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
                            img-src 'self' data:; media-src 'self' data:; font-src 'self' data:; connect-src 'none'; \
                            frame-src 'none'; object-src 'none'; base-uri 'self'";

/// The views in the window, by name: the home screen (`home`) and each app that's open, by its plugin's name.
struct Views {
    client: Arc<Client>,
    app: AppHandle,
    shown: Mutex<HashMap<String, Arc<Shown>>>,
    /// The files each open app's page has loaded, by the app's name.
    loaded: Mutex<HashMap<String, HashSet<String>>>,
    started: Instant,
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

    /// The files of every plugin a server lists, as MCP lists resources.
    async fn listed(&self) -> anyhow::Result<Value> {
        self.shown("home").ask("resources/list", json!({})).await
    }

    /// One file a server listed: what kind it is, and its text. A set of a plugin's files never changes, so a file
    /// that's kept is never asked for again.
    async fn read(&self, resource: &str) -> anyhow::Result<(String, String)> {
        let asked = json!({"uri": resource});
        let read = match self.client.kept("resources/read", &asked) {
            Some(read) => read,
            None => self.client.fresh("resources/read", &asked).await?,
        };
        let file = &read["contents"][0];
        let text = |part: &str| file[part].as_str().unwrap_or_default().to_owned();
        Ok((text("mimeType"), text("text")))
    }

    /// One file of a plugin, for an app's page: `oneapp://<app>.localhost/<plugin>/<path>` is `path` among the
    /// newest files of `plugin` this client knows of.
    async fn file(&self, address: &Uri) -> anyhow::Result<(String, String)> {
        let file = address.path().trim_start_matches('/');
        let listed = self.listed().await?;
        let resource = resources(&listed).find(|resource| {
            let mut parts = resource.trim_start_matches("ui://").splitn(3, '/');
            let (plugin, _version, path) = (parts.next(), parts.next(), parts.next());
            plugin.zip(path).is_some_and(|(plugin, path)| file.strip_prefix(plugin) == Some(&format!("/{path}")))
        });
        let resource = resource.ok_or_else(|| anyhow::anyhow!("no plugin here has {file}"))?;
        let app = address.host().and_then(|host| host.strip_suffix(".localhost")).unwrap_or_default();
        // Android's and Windows' web views name the app after the scheme, in a host's name.
        let app = app.strip_prefix("oneapp.").unwrap_or(app);
        self.loaded.lock().expect("no view panics").entry(app.to_owned()).or_default().insert(resource.to_owned());
        self.read(resource).await
    }

    /// The open apps that loaded a file a server no longer lists: one of their plugins has a new version. They
    /// have loaded nothing once this has named them.
    fn outdated(&self, listed: &Value) -> Vec<String> {
        let listed: HashSet<&str> = resources(listed).collect();
        let mut loaded = self.loaded.lock().expect("no view panics");
        let outdated: Vec<String> = loaded
            .iter()
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

/// The apps on the home screen: the files a server lists, of which an app is a page, and with them the open apps
/// that are `outdated` and load again. Every file listed is kept behind this, so with no server up an app opens
/// whole, even one that was never opened, or whose plugins have changed since.
#[tauri::command]
async fn apps(app: AppHandle, views: State<'_, Views>) -> Result<Value, String> {
    // A person is told what went wrong, and not what was behind it.
    let mut listed = views.listed().await.map_err(|error| error.to_string())?;
    listed["outdated"] = views.outdated(&listed).into();
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

/// Forgets what a view was answered: its page is gone, or loading again.
#[tauri::command]
fn close(views: State<'_, Views>, view: String) {
    views.shown.lock().expect("no view panics").remove(&view);
    views.loaded.lock().expect("no view panics").remove(&view);
}

/// Says how long a view took to open: an app from the tap on its icon to its content on screen, and the home
/// screen from when its page began to load, which the time since the program started is beside.
#[tauri::command]
fn opened(views: State<'_, Views>, view: String, ms: u32) {
    eprintln!("{view} opened in {ms} ms, {} ms after the app started", views.started.elapsed().as_millis());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started = Instant::now();
    tauri::Builder::default()
        .setup(move |app| {
            // A phone gives each app a folder of its own, which only the app is told.
            #[cfg(mobile)]
            let home = app.path().app_data_dir()?;
            #[cfg(desktop)]
            let home = client::home()?;
            let client = Arc::new(tauri::async_runtime::block_on(Client::open(home))?);
            app.manage(Views {
                client: client.clone(),
                app: app.handle().clone(),
                shown: Mutex::default(),
                loaded: Mutex::default(),
                started,
            });

            // A server said something changed, so every view asks again.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut told = client.told();
                while let Ok(message) = told.recv().await {
                    if message["method"] == "notifications/one/changed" {
                        handle.state::<Views>().again().await
                    }
                }
            });

            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default());
            #[cfg(desktop)]
            let window = window.title(app.package_info().name.clone()).inner_size(1024.0, 720.0);
            window.build()?;
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("oneapp", |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let response = match app.state::<Views>().file(request.uri()).await {
                    Ok((kind, text)) => Response::builder()
                        .header(header::CONTENT_TYPE, kind)
                        .header(header::CONTENT_SECURITY_POLICY, APP_MAY_LOAD)
                        .header(header::CACHE_CONTROL, "no-store")
                        .body(text.into_bytes()),
                    Err(error) => {
                        Response::builder().status(StatusCode::NOT_FOUND).body(format!("{error:#}").into_bytes())
                    }
                };
                responder.respond(response.expect("these headers are valid"))
            });
        })
        .invoke_handler(tauri::generate_handler![apps, ask, again, close, opened])
        .run(tauri::generate_context!())
        .expect("the app runs");
}
