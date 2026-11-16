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
use std::sync::Arc;

use client::Client;
use encryption_client::cipher;
use serde_json::{Value, json};
use tauri::{AppHandle, Runtime};

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
/// reaches it by: Apple's on an iPhone and Google's on Android. An app built without push (no Firebase config, or no
/// Push Notifications capability) says why it isn't woken, and still shows the inbox.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn listen(app: AppHandle, client: Arc<Client>) {
    use tauri_plugin_notifications::NotificationsExt;

    let (via, name) = if cfg!(target_os = "ios") { ("apns", "iPhone") } else { ("fcm", "Android phone") };
    let notifications = app.notifications();
    if let Err(error) = notifications.request_permission().await {
        eprintln!("this phone wasn't asked to show notifications: {error}");
    }
    let token = match notifications.register_for_push_notifications().await {
        Ok(token) => token,
        Err(error) => return eprintln!("this phone isn't woken by notifications: {error}"),
    };
    let key = key(&client);
    let register =
        json!({"name": "notifications_register", "arguments": {"via": via, "token": token, "name": name, "key": key}});
    match client.request("tools/call", register).await {
        Ok(response) if response["isError"] != true => eprintln!("this phone is woken by notifications"),
        Ok(response) => eprintln!("this phone isn't woken by notifications: {}", response["content"][0]["text"]),
        Err(error) => eprintln!("this phone isn't woken by notifications: {error:#}"),
    }
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
