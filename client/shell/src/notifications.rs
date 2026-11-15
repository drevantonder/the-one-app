//! Notifications on this device. The plugin `notifications` keeps its person's inbox, and a server tells this client
//! when something changed, which may be a new one there.
//!
//! - A computer shows what's new in the inbox as the system's own notification while the app runs, and a tap on one
//!   opens the app it's from. One that isn't running shows nothing: its person sees the inbox when they open it.
//! - A phone is woken by its push service instead, even when the app isn't running, so it gives the plugin where
//!   that reaches it, and shows nothing itself.

use std::sync::Arc;

use client::Client;
use serde_json::{Value, json};
use tauri::{AppHandle, Runtime};

/// What the inbox answers: its notifications, newest first.
async fn inbox(client: &Client, after: i64) -> anyhow::Result<Vec<Value>> {
    let answer =
        client.request("tools/call", json!({"name": "notifications_inbox", "arguments": {"after": after}})).await?;
    let text = answer["content"][0]["text"].as_str().unwrap_or_default();
    if answer["isError"] == true {
        anyhow::bail!("{text}");
    }
    let inbox: Value = serde_json::from_str(text)?;
    Ok(inbox["notifications"].as_array().cloned().unwrap_or_default())
}

#[cfg(desktop)]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_notification::init()
}

#[cfg(mobile)]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_notifications::init()
}

/// Shows each notification that reaches the inbox after the app started, as a server says something changed. One
/// already read, on another device, isn't shown. A tap on one brings the window up on the app it's from.
#[cfg(desktop)]
pub async fn hear(app: AppHandle, client: Arc<Client>) {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use tauri::{Emitter, Manager};
    use tauri_plugin_notification::NotificationExt;

    // What each notification shown opens, by its id.
    let opens = Arc::new(Mutex::new(HashMap::<i32, String>::new()));
    let (tapped, handle) = (opens.clone(), app.clone());
    let heard = app.notification().on_action(move |action| {
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
    if let Err(error) = heard {
        eprintln!("a tap on a notification opens nothing here: {error}");
    }

    let mut told = client.told();
    // The newest notification there was, once the inbox has answered: what's after it is new.
    let mut newest = None;
    loop {
        match (newest, inbox(&client, newest.unwrap_or(0)).await) {
            (_, Err(error)) => eprintln!("couldn't ask the inbox: {error:#}"),
            (None, Ok(there)) => newest = Some(there.first().and_then(|one| one["id"].as_i64()).unwrap_or(0)),
            (Some(_), Ok(new)) => {
                for notification in new.iter().rev() {
                    let id = notification["id"].as_i64().unwrap_or_default();
                    newest = newest.max(Some(id));
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
            match told.recv().await {
                Ok(message) if message["method"] == "notifications/one/changed" => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }
}

/// Asks its person whether this phone may show notifications, and gives the plugin the token its push service
/// reaches it by: Apple's on an iPhone and Google's on Android. An app built without push (no Firebase config, or no
/// Push Notifications capability) says why it isn't woken, and still shows the inbox.
#[cfg(mobile)]
pub async fn hear(app: AppHandle, client: Arc<Client>) {
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
    let register = json!({"name": "notifications_register", "arguments": {"via": via, "token": token, "name": name}});
    match client.request("tools/call", register).await {
        Ok(answer) if answer["isError"] != true => eprintln!("this phone is woken by notifications"),
        Ok(answer) => eprintln!("this phone isn't woken by notifications: {}", answer["content"][0]["text"]),
        Err(error) => eprintln!("this phone isn't woken by notifications: {error:#}"),
    }
}
