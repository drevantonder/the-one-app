//! Where this device is with its network: opening the client, and following its session and connection.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "android")]
use std::time::Instant;

use ::client::{Client, Connection, State};
use serde_json::{Value, json};
use url::Url;

use crate::*;

/// Once the device is signed in to its network, which it's told only once its views are open (`follows`).
#[cfg(not(mobile))]
/// Waits until this device's session is settled: in its network, or signing in, which it is once its client has opened
/// and asked.
#[cfg(not(mobile))]
pub(crate) async fn settled() {
    let mut session = app().start.subscribe();
    let _ = session.wait_for(|now| matches!(now, Some((_, State::SignedIn { .. } | State::SignIn { .. })))).await;
}

pub(crate) async fn signed_in() {
    let mut session = app().start.subscribe();
    let _ = session.wait_for(|now| matches!(now, Some((_, State::SignedIn { .. })))).await;
}

/// Where this device is now, as the window's page hears it (`state`): its session, the session's place in the order
/// the page hears them in, whether its network answers or its servers say it's behind them, and while the device is
/// signing in, where the window goes: the network's sign-in page with its code, told it's in the app (`&app`), which
/// has no passkeys, and why it shows a new code, if it says.
pub(crate) fn state(seq: u64, session: &State) -> Value {
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

/// Opens the client, gives the window its views, and has the window follow the client's session and connection from
/// then on (`follows`). A client that couldn't open at all, as one whose home can't be read, is said as one still
/// opening that reaches nothing (`start`), which trying again opens again.
pub(crate) async fn opens() {
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
    #[cfg(not(mobile))]
    computer::opened();
    // A move into or out of being a server that the machine went off during is settled as the app opens, whether or
    // not the system still has the server to start (`cli::serving::settle`).
    #[cfg(not(mobile))]
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
        deploys: Mutex::default(),
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
            for (app, began) in message["params"]["deployed"].as_object().into_iter().flatten() {
                if let Some(began) = began.as_u64() {
                    views.deploys.lock().expect("no view panics").insert(app.clone(), began);
                }
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
        Client::open(home, "app", cli::resolvers(), tools).await
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
