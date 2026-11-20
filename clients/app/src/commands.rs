//! What the window's page asks, answered the same on a phone (Tauri's commands) and a computer (`hosts::command`).

use std::sync::atomic::Ordering;

#[cfg(desktop)]
use ::client::Connection;
use serde_json::{Value, json};

use crate::*;

/// The files a server lists for `subject`, a group or the network, or for this client's user, and with them the open
/// apps that are `outdated` and load again, each as its address names it. A list is kept only with every file it
/// lists (`Client::fresh`), so with no server up an app opens whole, even one that was never opened. The user's
/// apps with a screen are where the device lists its own apps too (`shortcuts`).
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn apps(subject: Option<String>) -> Result<Value, String> {
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
pub(crate) async fn request(view: String, method: String, params: Value) -> Result<Value, String> {
    views()?.subscription(&view).request(&method, params).await.map_err(|error| error.to_string())
}

/// Asks again for what a view was answered: its user returned to it.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn refresh(view: String) -> Result<(), String> {
    views()?.subscription(&view).refresh(None).await;
    Ok(())
}

/// This device's key, as its network knows it, the network's address, if it has one, the build it's on and its kind
/// of machine, as builds are named, its server's key, if it's a server, and whether a phone is woken by its
/// notifications (`notifications::Push`), whether it opens its own Settings page for the app (`settings`, an iPhone's;
/// Android's is opened by its activity), and whether its person turned Computer Use on, on a phone that has it.
#[cfg_attr(mobile, tauri::command)]
pub(crate) fn client() -> Result<Value, String> {
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
        "login": login(),
    }))
}

/// Whether the app opens when this device's person logs in, as the system says; null on a device that has no such
/// thing (a phone's), or whose build keeps none (`login`).
pub(crate) fn login() -> Option<bool> {
    #[cfg(desktop)]
    return login::enabled();
    #[cfg(mobile)]
    None
}

/// Makes this device a server of its person's, or with `on` false, no longer one, as `one serve on` and `one serve
/// off` do. Gives what they say.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn serve(on: bool) -> Result<String, String> {
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
pub(crate) async fn update() -> Result<String, String> {
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
pub(crate) async fn visible(on_screen: bool) -> Result<(), String> {
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
pub(crate) fn editing(editing: bool) {
    updater::editing("main", editing)
}

/// Sends a file an app's page chose, which is the request's body, to where the network keeps it, for a tool of
/// the app's app. Gives the name that tool takes for it.
#[cfg(mobile)]
#[tauri::command]
pub(crate) async fn upload(request: tauri::ipc::Request<'_>) -> Result<Value, String> {
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
pub(crate) async fn upload_file(app: &str, file: Vec<u8>) -> Result<Value, String> {
    let sent = views()?.client.upload(app, file).await;
    sent.map(|upload| json!({"upload": upload})).map_err(|error| error.to_string())
}

/// The recovery key of this device's person, or of a group or the network an app was opened for (`subject`), for the
/// client's own page to show them. It's that page that shows it, so no app's page is handed one.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn recovery_key(subject: Option<String>) -> Result<String, String> {
    encryption_client::recovery_key(&views()?.client, subject.as_deref()).await.map_err(|error| error.to_string())
}

/// Lets this device hold its person's key, or `subject`'s, with the recovery key they typed on the client's own page,
/// and has every view ask again.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn recover(key: String, subject: Option<String>) -> Result<(), String> {
    let views = views()?;
    encryption_client::recover(&views.client, &key, subject.as_deref()).await.map_err(|error| error.to_string())?;
    views.refresh(None).await;
    Ok(())
}

/// Forgets what a view was answered: its page is gone, or loading again.
#[cfg_attr(mobile, tauri::command)]
pub(crate) fn close(view: String) -> Result<(), String> {
    views()?.subscriptions.lock().expect("no view panics").remove(&view);
    Ok(())
}

/// Asks the network again now, which didn't answer or refused this client for being behind it, as its person does
/// with Try Again (`Client::retry`), and answers once it has, failing where it still isn't answered. A client that
/// couldn't open at all is opened again.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn retry() -> Result<(), String> {
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

/// The device is back online or the window in front again, so a client whose network didn't answer asks it again
/// now (`Client::reconnect`).
#[cfg_attr(mobile, tauri::command)]
pub(crate) fn reconnect() -> Result<(), String> {
    views()?.client.reconnect();
    Ok(())
}

/// Signs this device out once its person has removed it from their devices (Devices' Sign Out): what it kept of
/// theirs goes, and the window follows it to the sign-in page.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn sign_out() -> Result<(), String> {
    views()?.client.sign_out().await.map_err(|error| format!("{error:#}"))?;
    native::forget_snapshots();
    Ok(())
}

/// Where this device is with its network (`state`), as the window's page asks once it has opened, which waits for the
/// client to open, and whether a phone hears a tap on a notification, through its push plugin, which the page listens
/// to. The page follows where it is from there (`state`). A client that couldn't open at all has reached nothing.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn start() -> Result<Value, String> {
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

/// Whether this is an iPhone's app installed from TestFlight, which updates it, as Apple tells it: its App Store
/// receipt is the sandbox's. A build installed from a Mac has none, and an App Store one is `receipt`.
pub(crate) fn testflight() -> bool {
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

/// Has a phone whose push failed try again now (`notifications::Push`), as its person does in Notifications, and
/// says where it is once that try has ended.
#[cfg_attr(mobile, tauri::command)]
pub(crate) async fn push() -> Result<Value, String> {
    Ok(json!({"message": format!("{:?}", notifications::retry().await)}))
}

/// Opens this app's page in a phone's Settings, where its person allows what only they may, as an iPhone has it
/// (`client({ settings: "app" })`: Android's is opened by its activity).
#[cfg(mobile)]
#[tauri::command]
pub(crate) fn settings() -> Result<(), String> {
    app().shell.open_url("app-settings:")
}

/// Has this device hear its person's notifications, once their home is on screen: a phone asks them then whether
/// it may show any, over what they came for and not over signing in. Each time after, as its home is back in front,
/// a phone looks again at whether it may (`notifications::Push`), as its person may have allowed them in its Settings.
#[cfg_attr(mobile, tauri::command)]
pub(crate) fn listen() -> Result<(), String> {
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
pub(crate) fn push_token(token: String) {
    notifications::token(token);
}

/// A notification was tapped on a phone, which its push plugin said to the window's page (`notificationClicked`), so
/// the window opens the app it's from, as a tap on a computer's does.
#[cfg(mobile)]
#[tauri::command]
pub(crate) fn tapped(tapped: Value) {
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
pub(crate) async fn open_window(entrypoint: String, subject: Option<String>, link: String, icon: Option<[f32; 4]>) -> Result<bool, String> {
    let icon = icon.map(|[x, y, width, height]| renderer::geometry::Bounds { origin: renderer::geometry::point(x, y), size: renderer::geometry::Size { width, height } });
    hosts::open_window(entrypoint, subject, link, icon).await
}

/// Opens the launcher's page natively, as the phone's root window, under every app's (`native::computer::open`).
/// → whether it did; if not, the page shows the web launcher.
#[cfg(mobile)]
#[tauri::command]
pub(crate) async fn open_launcher(entrypoint: String) -> bool {
    native::computer::open(entrypoint).await
}

/// Whether the launcher's root window shows: not while the page shows an app over it, which it draws itself.
#[cfg(mobile)]
#[tauri::command]
pub(crate) fn launcher_shown(shown: bool) {
    native::computer::shown(shown);
}

/// An app's window was drawn: if a server said the app was deployed in the last minute, how long from when its deploy
/// began to this, as the app's log says and Telemetry hears (`one.client.deploy.window`), whose budget is 500 ms. The
/// deploy's start is the server's clock, as close to this device's as the internet's time keeps them.
pub(crate) fn deployed(app: &str) {
    let Ok(views) = views() else { return };
    let Some(began) = views.deploys.lock().expect("no view panics").remove(app) else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |now| now.as_millis() as u64);
    let ms = now.saturating_sub(began);
    if ms < 60_000 {
        eprintln!("{app} showed its deploy {ms} ms after it began");
        views.client.report(json!({"name": "one.client.deploy.window", "value": ms, "attributes": {"one.app": app}}));
    }
}

#[cfg_attr(mobile, tauri::command)]
pub(crate) fn opened(view: String, ms: u32) -> Result<(), String> {
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
pub(crate) fn navigation_gestures(window: tauri::WebviewWindow, on: bool, home: bool) -> Result<(), String> {
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
pub(crate) fn status_bar(window: tauri::WebviewWindow, shown: bool) -> Result<(), String> {
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
pub(crate) fn theme(theme: Option<Theme>) {
    chose(theme);
    app().shell.theme(theme);
}

/// Keeps the Colour Mode chosen on this device for each page that opens after (`start`), and tells every page that's
/// open.
pub(crate) fn chose(theme: Option<Theme>) {
    *app().chosen.lock().expect("nothing panics holding it") = Some(theme);
    app().shell.emit("theme", json!(theme));
}

/// Reports what the window's page saw go wrong (`Client::report`): what it showed its person, a page's error, an
/// app that never drew.
#[cfg_attr(mobile, tauri::command)]
pub(crate) fn report(record: Value) -> Result<(), String> {
    views()?.client.report(record);
    Ok(())
}
