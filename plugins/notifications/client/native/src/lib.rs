//! Notifications' native code in the app: notifications on this device. The plugin keeps its person's inbox, and a server tells this client
//! when something changed, which may be a new one there.
//!
//! - A computer shows what's new in the inbox as the system's own notification while the app runs, and a tap on one
//!   opens the app it's from. One that isn't running shows nothing: its person sees the inbox when they open it.
//! - A phone is woken by its push service instead, even when the app isn't running, so it gives the plugin where
//!   that reaches it, and the key what it's sent is encrypted to. What's sent is opened on the phone before it
//!   shows: by the app's notification extension on an iPhone (`clients/app/gen/apple/Notifications`), and on
//!   Android by the app's own messaging service (`clients/app/gen/android/.../Notifications.kt`), which asks
//!   `open` below.

use std::path::Path;
use std::sync::{Arc, LazyLock};

use client::Client;
use encryption_client::cipher;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Runtime};
use tokio::sync::{Notify, watch};

/// Where a phone is with being woken by its notifications, which Notifications says on it (`push`). It moves only as
/// `moves_to` says, by `listen`. A computer isn't woken: it shows them while the app runs, and has none of this.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Push {
    /// Its person hasn't been asked yet, which they are once they see their home (`listen`).
    NotAsked,
    /// Its person, or its system, said it may not show notifications.
    Denied,
    /// Giving the network where its push service reaches it.
    Registering,
    /// The network wakes it.
    Registered,
    /// It couldn't be: an app built without push, or no server answered or took it. It's tried again once a server
    /// answers again or the device is signed in again, and at once when its person tries again (`retry`).
    Failed { why: String },
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl Push {
    /// Whether a phone in this state moves to `next`: the only moves there are.
    fn moves_to(&self, next: &Push) -> bool {
        use Push::*;
        matches!(
            (self, next),
            (NotAsked, Denied | Registering | Failed { .. }) | (Registering, Registered | Failed { .. }) | (Failed { .. }, Registering)
        )
    }
}

/// Where this phone is with its push (`Push`), and its person asking to try again.
static PUSH: LazyLock<watch::Sender<Push>> = LazyLock::new(|| watch::Sender::new(Push::NotAsked));
static AGAIN: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Moves this phone's push on to `next`, if that's a move there is from where it is.
#[cfg(any(target_os = "android", target_os = "ios"))]
fn moved(next: Push) {
    PUSH.send_if_modified(|now| {
        let moves = now.moves_to(&next);
        if moves {
            eprintln!("this phone's push: {next:?}");
            *now = next;
        }
        moves
    });
}

/// Where this phone is with being woken by its notifications, for its own row in Notifications: nothing on a computer.
pub fn push() -> Option<Push> {
    cfg!(any(target_os = "android", target_os = "ios")).then(|| PUSH.borrow().clone())
}

/// Has a phone whose push failed try again now, as its person does with Try Again, and gives where it is once that
/// try has ended, or a minute on.
pub async fn retry() -> Option<Push> {
    let mut push = PUSH.subscribe();
    if matches!(*push.borrow_and_update(), Push::Failed { .. }) {
        AGAIN.notify_one();
        let tried = async {
            while push.changed().await.is_ok() {
                if *push.borrow_and_update() != Push::Registering {
                    return;
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(60), tried).await;
    }
    self::push()
}

/// What the inbox answers: its notifications, newest first.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn inbox(client: &Client, after: i64) -> anyhow::Result<Vec<Value>> {
    let response =
        client.request("tools/call", json!({"name": "notifications_inbox", "arguments": {"after": after}})).await?;
    let text = response["content"][0]["text"].as_str().unwrap_or_default();
    if response["isError"] == true {
        anyhow::bail!("{text}");
    }
    let inbox: Value = serde_json::from_str(text)?;
    Ok(inbox["notifications"].as_array().cloned().unwrap_or_default())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_notification::init()
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_notifications::init()
}

/// Shows each notification that reaches the inbox after the app started, as a server says something changed. One
/// already read, on another device, isn't shown. A tap on one brings the window up on the app it's from.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn listen(app: AppHandle, client: Arc<Client>) {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use tauri::{Emitter, Manager};
    use tauri_plugin_notification::NotificationExt;

    // What each notification shown opens, by its id.
    let opens = Arc::new(Mutex::new(HashMap::<i32, String>::new()));
    let (tapped, handle) = (opens.clone(), app.clone());
    let listener = app.notification().on_action(move |action| {
        let Some(id) = action.notification().map(|shown| shown.id()) else { return };
        if action.action_id() != "tap" {
            return;
        }
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        if let Some(plugin) = tapped.lock().expect("no tap panics").get(&id) {
            drop(handle.emit("open", plugin));
        }
    });
    if let Err(error) = listener {
        eprintln!("a tap on a notification opens nothing here: {error}");
    }

    let mut notifications = client.notifications();
    // The newest notification there was, once the inbox has answered: what's after it is new.
    let mut latest = None;
    loop {
        match (latest, inbox(&client, latest.unwrap_or(0)).await) {
            (_, Err(error)) => eprintln!("couldn't ask the inbox: {error:#}"),
            (None, Ok(there)) => latest = Some(there.first().and_then(|one| one["id"].as_i64()).unwrap_or(0)),
            (Some(_), Ok(new)) => {
                for notification in new.iter().rev() {
                    let id = notification["id"].as_i64().unwrap_or_default();
                    latest = latest.max(Some(id));
                    if notification["read"] == true {
                        continue;
                    }
                    let title = notification["title"].as_str().unwrap_or_default();
                    let mut shown = app.notification().builder().id(id as i32).title(title);
                    // What it's from goes first, as the system shows no app of ours but the one.
                    let from = notification["from"].as_str();
                    let text = notification["text"].as_str();
                    let body = [from, text].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                    if !body.is_empty() {
                        shown = shown.body(body);
                    }
                    if let Some(plugin) = notification["app"].as_str() {
                        opens.lock().expect("no tap panics").insert(id as i32, plugin.to_owned());
                    }
                    match shown.show() {
                        Ok(()) => eprintln!("showed a notification: {title}"),
                        Err(error) => eprintln!("couldn't show a notification: {error}"),
                    }
                }
            }
        }
        // Until a server says something changed.
        loop {
            match notifications.recv().await {
                Ok(message) if message["method"] == "notifications/one/changed" => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }
}

/// The app a notification tapped on a phone opens, by its plugin's name, as its push plugin says of the tap
/// (`notificationClicked`): its `data` is what the notification carried, which the app's notification extension or
/// messaging service put there as it opened it, `{"app": "claims"}`.
pub fn opens(tapped: &Value) -> Option<&str> {
    tapped["data"]["app"].as_str().filter(|app| !app.is_empty())
}

/// Asks its person whether this phone may show notifications, and gives the plugin the token its push service
/// reaches it by: Apple's on an iPhone and Google's on Android (`Push`). What fails, as on a first open with no
/// server answering yet, is tried again once a server answers again or the device is signed in again, or its person
/// tries again. An app built without push (no Firebase config, or no Push Notifications capability) says why it
/// isn't woken, and still shows the inbox.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn listen(app: AppHandle, client: Arc<Client>) {
    use tauri_plugin_notifications::{NotificationsExt, PermissionState};

    let notifications = app.notifications();
    match notifications.request_permission().await {
        Ok(PermissionState::Granted) => {}
        Ok(_) => return moved(Push::Denied),
        Err(error) => return moved(Push::Failed { why: format!("This phone wasn't asked to show notifications: {error}") }),
    }
    let (mut connection, mut session) = (client.connection(), client.session());
    loop {
        moved(Push::Registering);
        match register(&notifications, &client).await {
            Ok(()) => return moved(Push::Registered),
            Err(why) => moved(Push::Failed { why }),
        }
        let answering = async {
            while connection.changed().await.is_ok() {
                if *connection.borrow_and_update() == client::Connection::Online {
                    return;
                }
            }
            std::future::pending().await
        };
        // Out of its network, if it isn't already, and then in again.
        let signed_in_again = async {
            let _ = session.wait_for(|session| *session != client::Session::SignedIn).await;
            let _ = session.wait_for(|session| *session == client::Session::SignedIn).await;
        };
        tokio::select! {
            _ = answering => {}
            _ = signed_in_again => {}
            _ = AGAIN.notified() => {}
        }
    }
}

/// Gives the plugin the token this phone's push service reaches it by, with the key what it's sent is encrypted to.
#[cfg(any(target_os = "android", target_os = "ios"))]
async fn register<R: Runtime>(notifications: &tauri_plugin_notifications::Notifications<R>, client: &Client) -> Result<(), String> {
    let (via, name) = if cfg!(target_os = "ios") { ("apns", "iPhone") } else { ("fcm", "Android phone") };
    let token = notifications.register_for_push_notifications().await.map_err(|error| error.to_string())?;
    let register =
        json!({"name": "notifications_register", "arguments": {"via": via, "token": token, "name": name, "key": key(client)}});
    let response = client.request("tools/call", register).await.map_err(|error| format!("{error:#}"))?;
    if response["isError"] == true {
        return Err(response["content"][0]["text"].as_str().unwrap_or("The network didn't take it.").to_owned());
    }
    Ok(())
}

/// The key this device's notifications are encrypted to, which it registers with its push token.
pub fn key(client: &Client) -> String {
    cipher::notifications(client.device_key()).public().to_string()
}

/// A notification sent to the device whose home is `home`, opened with its key there: `{title, text, app}`. It's
/// for what shows a notification on a phone, which may run while the app doesn't.
pub fn notification(home: &Path, sealed: &[u8]) -> anyhow::Result<Value> {
    let device = client::key(&home.join("key"))?;
    Ok(serde_json::from_slice(&cipher::open_notification(&device, sealed)?)?)
}

/// Opens a notification for Android's messaging service (`clients/app/gen/android/.../Notifications.kt`), which may run while the
/// app doesn't: `sealed` is what FCM carried, base64, and `home` the app's folder, whose key opens it. Gives its JSON,
/// or null if it doesn't open.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_client_Notifications_open<'local>(
    mut env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
    home: jni::objects::JString<'local>,
    sealed: jni::objects::JString<'local>,
) -> jni::sys::jstring {
    let mut opened = || -> anyhow::Result<String> {
        let home: String = env.get_string(&home)?.into();
        let sealed: String = env.get_string(&sealed)?.into();
        let sealed = data_encoding::BASE64.decode(sealed.as_bytes())?;
        Ok(notification(Path::new(&home), &sealed)?.to_string())
    };
    let opened = opened();
    match opened {
        Ok(string) => env.new_string(string).map(|string| string.into_raw()).unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            eprintln!("a notification didn't open: {error:#}");
            std::ptr::null_mut()
        }
    }
}
