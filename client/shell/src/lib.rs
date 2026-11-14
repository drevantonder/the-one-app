//! The app: a window on the client's Rust half. Its own page (`page`) is the home screen. Each app is a page of its
//! plugin's, drawn in a frame of its own, which reaches nothing here but the bridge the home screen's page gives it:
//! what an app asks comes to `ask`, as what the home screen asks does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

    /// One file of a plugin, for an app's page: `oneapp://<app>.localhost/<plugin>/<path>` is `path` among the
    /// newest files of `plugin` this client knows of. A set of files never changes, so one that's kept is never
    /// asked for again.
    async fn file(&self, address: &Uri) -> anyhow::Result<(String, String)> {
        let file = address.path().trim_start_matches('/');
        let listed = self.shown("home").ask("resources/list", json!({})).await?;
        let resources =
            listed["resources"].as_array().into_iter().flatten().filter_map(|resource| resource["uri"].as_str());
        let resource = resources
            .filter(|resource| {
                let mut parts = resource.trim_start_matches("ui://").splitn(3, '/');
                let (plugin, _code, path) = (parts.next(), parts.next(), parts.next());
                plugin.zip(path).is_some_and(|(plugin, path)| file.strip_prefix(plugin) == Some(&format!("/{path}")))
            })
            .next();
        let asked = json!({"uri": resource.ok_or_else(|| anyhow::anyhow!("no plugin here has {file}"))?});
        let read = match self.client.kept("resources/read", &asked) {
            Some(read) => read,
            None => self.client.fresh("resources/read", &asked).await?,
        };
        let file = &read["contents"][0];
        Ok((
            file["mimeType"].as_str().unwrap_or_default().to_owned(),
            file["text"].as_str().unwrap_or_default().to_owned(),
        ))
    }
}

/// Answers what a view asks, in MCP's words: the result of `method`.
#[tauri::command]
async fn ask(views: State<'_, Views>, view: String, method: String, params: Value) -> Result<Value, String> {
    views.shown(&view).ask(&method, params).await.map_err(|error| format!("{error:#}"))
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
}

/// Says how long an app took to open, from the tap on its icon to its content on screen.
#[tauri::command]
fn opened(view: String, ms: u32) {
    eprintln!("{view} opened in {ms} ms");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // A phone gives each app a folder of its own, which only the app is told.
            #[cfg(mobile)]
            let home = app.path().app_data_dir()?;
            #[cfg(desktop)]
            let home = client::home()?;
            let client = Arc::new(tauri::async_runtime::block_on(Client::open(home))?);
            app.manage(Views { client: client.clone(), app: app.handle().clone(), shown: Mutex::default() });

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
        .invoke_handler(tauri::generate_handler![ask, again, close, opened])
        .run(tauri::generate_context!())
        .expect("the app runs");
}
