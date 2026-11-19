//! Notifications' native code in the app: notifications on this device. The app keeps its person's inbox, and a server tells this client
//! when something changed, which may be a new one there.
//!
//! - A computer shows what's new in the inbox as the system's own notification while the app runs, and a click on one
//!   opens the app it's from. One that isn't running shows nothing: its person sees the inbox when they open it.
//! - A phone is woken by its push service instead, even when the app isn't running, so it gives the app where
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
#[cfg(any(target_os = "android", target_os = "ios"))]
use tauri::{AppHandle, Runtime};
use tokio::sync::{Notify, watch};

/// Where a phone is with being woken by its notifications, which Notifications says on it (`push`). It's the child of
/// its client's session: it registers each time the device is signed in, each time it connects to its network and
/// is back in front, and again whenever what it registered changes, its push token or whether its person allows
/// notifications. It moves only as `moves_to` says, by
/// `listen`. A computer isn't woken: it shows them while the app runs, and has none of this.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Push {
    /// Its person hasn't been asked yet, which they are once they see their home (`listen`).
    NotAsked,
    /// Its person, or its system, said it may not show notifications. It registers once they allow them, as in the
    /// phone's Settings, which it looks at again each time the app is back in front.
    Denied,
    /// Giving the network where its push service reaches it, for the first time or after it failed: a phone that's
    /// registered gives it again as it connects without leaving `Registered`.
    Registering,
    /// The network wakes it, until the device is signed in again, its push token changes or its person turns
    /// notifications off.
    Registered,
    /// It couldn't be: an app built without push, or no server answered or took it. It's tried again once a server
    /// answers again, the device is signed in again or the app is back in front, and at once when its person tries
    /// again (`retry`).
    Failed { why: String },
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl Push {
    /// Whether a phone in this state moves to `next`: the only moves there are.
    fn moves_to(&self, next: &Push) -> bool {
        use Push::*;
        matches!(
            (self, next),
            // Its person was asked, or it couldn't ask.
            (NotAsked, Denied | Registering | Failed { .. })
            // The network took it, or didn't.
            | (Registering, Registered | Failed { .. })
            // Signed in again, a new token, or allowed now: it registers again.
            | (Registered | Failed { .. } | Denied, Registering)
            // Its person turned notifications off.
            | (Registered | Failed { .. }, Denied)
            // Its system couldn't say whether it may.
            | (Registered | Denied, Failed { .. })
        )
    }
}

/// Where this phone is with its push (`Push`), and what tells it to look again: its person asking to try again, the
/// app back in front, and the push token its push service gave it last (`heard`).
static PUSH: LazyLock<watch::Sender<Push>> = LazyLock::new(|| watch::Sender::new(Push::NotAsked));
static AGAIN: LazyLock<Notify> = LazyLock::new(Notify::new);
static TOKEN: LazyLock<watch::Sender<Option<String>>> = LazyLock::new(|| watch::Sender::new(None));

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

/// Where this phone's push is, as it moves.
pub fn pushes() -> watch::Receiver<Push> {
    PUSH.subscribe()
}

/// Has a phone look again at whether it may show notifications, as once the app is back in front.
pub fn look_again() {
    AGAIN.notify_one();
}

/// The push token a phone's push service just gave it, which it registers if it isn't the one it registered.
pub fn token(token: String) {
    TOKEN.send_replace(Some(token));
}

/// Has a phone whose push failed try again now, as its person does with Try Again, or one that hasn't asked its person
/// yet ask them now, as Turn On does, and gives where it is once that has ended, or a minute on.
pub async fn retry() -> Option<Push> {
    let mut push = PUSH.subscribe();
    if matches!(*push.borrow_and_update(), Push::NotAsked | Push::Failed { .. }) {
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

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_notifications::init()
}

/// Shows each notification that reaches the inbox after the app started, as a server says something changed. One
/// already read, on another device, isn't shown. A click on one opens the app it's from (`tapped`, with its app's
/// name). `identifier` is the app's, which a Mac and Windows show its notifications as.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn listen(client: Arc<Client>, identifier: &str, tapped: impl Fn(String) + Clone + Send + 'static) {
    #[cfg(target_os = "macos")]
    if let Err(error) = notify_rust::set_application(identifier) {
        eprintln!("this Mac shows notifications as another app's: {error}");
    }
    let mut notifications = client.notifications();
    // The newest notification there was, once the inbox has answered: what's after it is new.
    let mut latest = None;
    loop {
        // Behind its servers, it's refused for asking, so it stops asking until they talk with it again
        // (`Connection::Behind`), as once it's updated or they are.
        let talks = |state: &client::State| !matches!(state.connection(), client::Connection::Behind(_));
        if client.state().wait_for(talks).await.is_err() {
            return;
        }
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
                    let title = notification["title"].as_str().unwrap_or_default().to_owned();
                    // What it's from goes first, as the system shows no app of ours but the one.
                    let from = notification["from"].as_str();
                    let text = notification["text"].as_str();
                    let body = [from, text].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                    let opens = match (notification["app"].as_str(), notification["page"].as_str()) {
                        (Some(app), Some(page)) => Some(format!("{app}/{page}")),
                        (app, _) => app.map(str::to_owned),
                    };
                    let (tapped, identifier) = (tapped.clone(), identifier.to_owned());
                    // The system's call waits on its notification service, and on Linux for the tap.
                    tokio::task::spawn_blocking(move || show(&identifier, &title, &body, opens, tapped));
                }
            }
        }
        // Until a server says something of Notifications' changed, or something it doesn't name: another app's
        // change is never a new notification, as sending one says so itself.
        loop {
            match notifications.recv().await {
                Ok(message)
                    if message["method"] == "notifications/one/changed"
                        && matches!(message["params"]["app"].as_str(), None | Some("notifications")) =>
                {
                    break;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }
}

/// Shows one notification, as the system's own, and hears it clicked: a Linux desktop's default action, a Mac's
/// click and a Windows toast's activation.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn show(identifier: &str, title: &str, body: &str, opens: Option<String>, tapped: impl Fn(String)) {
    let mut shown = notify_rust::Notification::new();
    shown.summary(title).body(body);
    // Without one, Windows shows it as PowerShell, the ID its toast library defaults to.
    #[cfg(windows)]
    shown.app_id(identifier);
    #[cfg(not(windows))]
    let _ = identifier;
    #[cfg(target_os = "linux")]
    shown.action("default", "Open");
    let handle = match shown.show() {
        Ok(handle) => handle,
        Err(error) => return eprintln!("couldn't show a notification: {error}"),
    };
    eprintln!("showed a notification: {title}");
    let clicked = |response: &notify_rust::NotificationResponse| {
        if let (notify_rust::NotificationResponse::Default, Some(app)) = (response, opens) {
            tapped(app)
        }
    };
    if let Err(error) = handle.wait_for_response(clicked) {
        eprintln!("a notification's click isn't heard: {error}");
    }
}

/// The app a notification tapped on a phone opens, by its app's name, as its push plugin says of the tap
/// (`notificationClicked`): its `data` is what the notification carried, which the app's notification extension or
/// messaging service put there as it opened it, `{"app": "claims"}`.
pub fn opens(tapped: &Value) -> Option<&str> {
    tapped["data"]["app"].as_str().filter(|app| !app.is_empty())
}

/// Asks its person whether this phone may show notifications, and gives the network the token its push service
/// reaches it by: Apple's on an iPhone and Google's on Android (`Push`). It follows the device's session for as long
/// as the app runs: each time the device is signed in, each time it connects to its network and each time the app is
/// back in front, it registers again, and as its token changes, and once its person allows notifications after all.
/// Push services rotate tokens, and a network that lost what it held of this phone gets it back the next time the
/// phone reaches it: registering again is an upsert. What fails, as on a first open with no server answering yet, is tried again
/// once a server answers again, or its person tries again. An app built without push (no Firebase config, or no Push
/// Notifications capability) says why it isn't woken, and still shows the inbox.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn listen(app: AppHandle, client: Arc<Client>) {
    use client::State;
    use tauri_plugin_notifications::NotificationsExt;

    let notifications = app.notifications();
    let (mut state, mut heard) = (client.state(), TOKEN.subscribe());
    // The token the network was last given, and whether it's to be given again whatever it's at.
    let (mut given, mut again) = (None, true);
    loop {
        let mut now = state.borrow_and_update().clone();
        if matches!(now, State::SignedIn { .. }) {
            match allowed(&notifications).await {
                Err(why) => moved(Push::Failed { why }),
                Ok(false) => moved(Push::Denied),
                Ok(true) if again || *PUSH.borrow() != Push::Registered => {
                    if *PUSH.borrow() != Push::Registered {
                        moved(Push::Registering);
                    }
                    match register(&notifications, &client).await {
                        Ok(token) => {
                            given = Some(token);
                            moved(Push::Registered)
                        }
                        Err(why) => moved(Push::Failed { why }),
                    }
                }
                Ok(true) => {}
            }
        }
        // Until something it registered may have changed.
        again = loop {
            tokio::select! {
                // Signed in again, after it was out of its network, it registers again, and so it does once a server
                // answers, as it does again after the network was lost, which may take what failed, or may hold
                // nothing of this phone.
                changed = state.changed() => {
                    let Ok(()) = changed else { return };
                    let was = std::mem::replace(&mut now, state.borrow_and_update().clone());
                    if !now.same_session(&was) {
                        break matches!(now, State::SignedIn { .. });
                    }
                    if now.connection().answers() && !was.connection().answers() {
                        break true;
                    }
                }
                changed = heard.changed() => {
                    let Ok(()) = changed else { return };
                    break heard.borrow_and_update().as_ref() != given.as_ref();
                }
                _ = AGAIN.notified() => break true,
            }
        };
    }
}

/// Whether this phone may show notifications, asking its person if they haven't said.
#[cfg(any(target_os = "android", target_os = "ios"))]
async fn allowed<R: Runtime>(notifications: &tauri_plugin_notifications::Notifications<R>) -> Result<bool, String> {
    use tauri_plugin_notifications::PermissionState::*;
    let asked = |error| format!("This phone wasn't asked to show notifications: {error}");
    match notifications.permission_state().await.map_err(asked)? {
        Granted => Ok(true),
        Denied => Ok(false),
        Prompt | PromptWithRationale => Ok(notifications.request_permission().await.map_err(asked)? == Granted),
    }
}

/// Gives the network the token this phone's push service reaches it by, with the key what it's sent is encrypted to,
/// and says which token that was.
#[cfg(any(target_os = "android", target_os = "ios"))]
async fn register<R: Runtime>(notifications: &tauri_plugin_notifications::Notifications<R>, client: &Client) -> Result<String, String> {
    let via = if cfg!(target_os = "ios") { "apns" } else { "fcm" };
    let token = notifications.register_for_push_notifications().await.map_err(|error| error.to_string())?;
    let register =
        json!({"name": "notifications_register", "arguments": {"via": via, "token": token, "key": key(client)}});
    let response = client.request("tools/call", register).await.map_err(|error| format!("{error:#}"))?;
    if response["isError"] == true {
        return Err(response["content"][0]["text"].as_str().unwrap_or("The network didn't take it.").to_owned());
    }
    Ok(token)
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
