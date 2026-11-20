//! A phone's window, Tauri's: what's here tells it and asks it (`Shell`).

use std::sync::Arc;

use ::client::{Client, State};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;

use crate::*;

/// What an app's page may load, which is what an MCP App may by default: its own app's files and the files of
/// the apps it uses, which are all at its own address, and nothing from anywhere else; and in a frame, another app's
/// page at that app's own address (`oneapp://<app>.localhost`, or `http://oneapp.<app>.localhost` where a web view
/// has no scheme of its own), which the window answers as that app's (the UI library's `Frame`).
#[cfg(mobile)]
pub(crate) const APP_MAY_LOAD: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
                            img-src 'self' data:; media-src 'self' data:; font-src 'self' data:; connect-src 'none'; \
                            frame-src oneapp: http://*.localhost; object-src 'none'; base-uri 'self'";

/// Opens what each link the device hands the app links to, over whatever's showing, as a tap on a notification
/// does, and brings the window forward. The link the app was started with is the page's to ask for (`start`). A
/// computer's program that isn't a developer's build takes the device's `one:` links: Linux and Windows have a
/// program say so itself (a Mac's and an iPhone's say so in their bundles, and Android's in its manifest). A phone
/// takes some of its network's own links too, its app links (`APP_LINKS` in `apps/sign-in/edge/wrangler.jsonc`).
#[cfg(mobile)]
pub(crate) fn links(app: &tauri::App) -> tauri::Result<()> {
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
pub(crate) fn open_link(url: String) {
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

/// An iPhone's page runs under its status bar and home indicator, as its `viewport-fit=cover` asks, and keeps clear of
/// them itself (`env(safe-area-inset-*)`), as a page in Safari does: its web view doesn't inset it as well, which left
/// it that much short of the screen's foot, with the web view's own white under it.
#[cfg(target_os = "ios")]
pub(crate) fn cover(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|webview| {
        use objc2_ui_kit::{UIScrollView, UIScrollViewContentInsetAdjustmentBehavior, UIView};
        // Tauri hands over the window's WKWebView on the main thread.
        let web = unsafe { &*webview.inner().cast::<UIView>() };
        let page: objc2::rc::Retained<UIScrollView> = unsafe { objc2::msg_send![web, scrollView] };
        page.setContentInsetAdjustmentBehavior(UIScrollViewContentInsetAdjustmentBehavior::Never);
    })
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

/// A phone's window, Tauri's: what's here tells it and asks it (`Shell`).
#[cfg(mobile)]
pub(crate) struct Phone(pub(crate) AppHandle);

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

/// The Home Screen's window, made at its own page, which draws the mark until the client has opened behind it, and
/// then follows where the device is: a device that isn't in its network to the page that signs it in. A phone opens
/// the window again at the same address as it's made again (a change of text size), so that's home too, which asks
/// where the device is now.
#[cfg(mobile)]
pub(crate) fn window(app: &AppHandle) -> Result<tauri::WebviewWindow, Box<dyn std::error::Error>> {
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
    cover(&window)?;
    #[cfg(target_os = "ios")]
    status_bar::watch(&window)?;
    #[cfg(target_os = "ios")]
    edge::watch(&window)?;
    navigation_gestures(window.clone(), true, false)?;
    Ok(window)
}
