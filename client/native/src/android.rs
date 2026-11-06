//! What's Android's own, beside the window, in Kotlin (android/app/src/main/kotlin/app/client): signing in in a Chrome
//! Custom Tab (where a passkey's PRF works), the Keystore this device's keys are sealed by, pushes (FCM) and what a tapped
//! one opens or answers, the document scanner, the share sheet, haptics, the system bars, and the home-screen widgets.
//!
//! Two ways across, both JNI into this library, which the app's process loads whether or not its window is open:
//! - Kotlin calls in (`Native.call`, `Java_app_client_Native_call`): what the system starts with no window (a widget's
//!   button, the refresh WorkManager makes, a push) is answered here, by the same code the window uses (headless.rs).
//! - Rust calls out, to `Host` (what needs no window: the Keystore, the phone's name, the widgets drawn again) or to the
//!   Tauri plugin `OnePlugin` (what needs the window's activity: the Custom Tab, the scanner, the share sheet).

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JString, JValue};
use jni::sys::{JNI_VERSION_1_6, jint, jstring};
use jni::{JNIEnv, JavaVM};
use serde_json::{Value, json};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};

static VM: OnceLock<JavaVM> = OnceLock::new();
static HOST: OnceLock<GlobalRef> = OnceLock::new();
static PLUGIN: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

/// Found as the library loads, on a thread that sees the app's classes: a thread of Rust's own can't find them.
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _: *mut c_void) -> jint {
    if let Ok(mut env) = vm.get_env()
        && let Ok(class) = env.find_class("app/client/Host")
        && let Ok(global) = env.new_global_ref(class)
    {
        let _ = HOST.set(global);
    }
    let _ = VM.set(vm);
    JNI_VERSION_1_6
}

pub fn plugin() -> TauriPlugin<tauri::Wry> {
    Builder::new("one")
        .setup(|_, api| {
            let _ = PLUGIN.set(api.register_android_plugin("app.client", "OnePlugin")?);
            Ok(())
        })
        .build()
}

/// Asks the plugin (the window's activity) and waits for its answer: never from the main thread.
pub fn ask(method: &str, args: Value) -> Result<Value, String> {
    PLUGIN.get().ok_or("The app hasn't started yet.")?.run_mobile_plugin(method, args).map_err(|e| e.to_string())
}

pub async fn ask_off(method: &'static str, args: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || ask(method, args)).await.map_err(|e| e.to_string())?
}

/// Tells the plugin, not waiting.
pub fn tell(method: &'static str, args: Value) {
    std::thread::spawn(move || {
        if let Err(e) = ask(method, args) {
            eprintln!("{method}: {e}");
        }
    });
}

/// Asks `Host`, which needs no window: `{ "ok": … }` or `{ "error": "…" }`, as one string.
pub fn host(method: &str, args: Value) -> Result<Value, String> {
    let vm = VM.get().ok_or("The library isn't loaded by an app.")?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let class = HOST.get().ok_or("Host isn't found.")?;
    // SAFETY: a global reference to the class `Host`, found as the library loaded.
    let class = unsafe { JClass::from_raw(class.as_obj().as_raw()) };
    let (method, args) = (env.new_string(method).map_err(|e| e.to_string())?, env.new_string(args.to_string()).map_err(|e| e.to_string())?);
    let out = env
        .call_static_method(&class, "call", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&method), JValue::Object(&args)])
        .and_then(|v| v.l())
        .map_err(|e| {
            let _ = env.exception_clear();
            e.to_string()
        })?;
    let text: String = env.get_string(&JString::from(out)).map_err(|e| e.to_string())?.into();
    let said: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    match said["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(said["ok"].clone()),
    }
}

/// What only the page's own frame is given (Kotlin's `Host.token`, handed it by a web message listener that answers the
/// main frame at the app's own origin alone): every one of the app's commands carries it, as `x-one`, or is refused. Android's
/// web view gives its Tauri script and `ipc` to every frame, a screen's sandboxed one too, which could otherwise call them.
pub fn ours(invoke: &tauri::ipc::Invoke<tauri::Wry>) -> bool {
    static TOKEN: OnceLock<String> = OnceLock::new();
    let token = TOKEN.get_or_init(|| host("token", json!({})).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default());
    !token.is_empty() && invoke.message.headers().get("x-one").and_then(|v| v.to_str().ok()) == Some(token.as_str())
}

/// The app's own folder, where it keeps what isn't secret (Kotlin's `filesDir`): the same for the window and for what
/// runs with none.
pub fn files() -> Option<std::path::PathBuf> {
    host("files", json!({})).ok()?.as_str().map(Into::into)
}

/// What the Kotlin app kept, taken over once (Kotlin's `Host.takeOver`, which reads its SharedPreferences and Keystore
/// key): its address, an invite's secret, its sign-in's client and who's signed in, into this folder, and its refresh
/// token into the secret store as this app keeps it. Its device keys are kept where they were.
pub fn take_over() {
    if let Err(e) = host("takeOver", json!({ "scheme": one::config::CLI })) {
        eprintln!("taking over the old app's sign-in: {e}");
    }
}

/// What the person calls this phone: its name in Settings, else its model.
pub fn name() -> Option<String> {
    host("name", json!({})).ok()?.as_str().map(str::to_string).filter(|n| !n.is_empty())
}

/// The phone's own id for this app, hashed (its Android ID, which outlives reinstalling), which signing in again replaces
/// its sign-in under, as the Kotlin app's was.
pub fn this_device() -> Option<String> {
    host("device", json!({})).ok()?.as_str().map(str::to_string)
}

/// Whether Google's document scanner is here to use (Play services, and memory enough).
pub fn can_scan() -> bool {
    host("canScan", json!({})).ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

/// How much bigger than usual the person has made text in Settings (1 for usual), for screens to be zoomed by.
pub fn text_scale() -> f64 {
    host("textScale", json!({})).ok().and_then(|v| v.as_f64()).unwrap_or(1.0)
}

/// Signs in in a Chrome Custom Tab at `url` (or, in a test, waiting only for the link): comes back to the app's own link,
/// `<cli>://signed-in/…`, which Android opens the app on → where it came back to.
pub fn sign_in(url: String, open: bool) -> Result<String, String> {
    let out = ask("signIn", json!({ "url": url, "scheme": one::config::CLI, "open": open }))?;
    out["url"].as_str().map(str::to_string).ok_or_else(|| "Sign in again.".into())
}

/// Once let in, in a build with pushes (a Firebase project in its config.toml): gives the push Worker this phone's FCM token,
/// as it starts and whenever it changes (`push`). Notes are opened with the device's key by Kotlin's `PushService`.
pub fn push(device: std::sync::Arc<one_client::Device>) {
    tauri::async_runtime::spawn(async move {
        let token = match ask_off("push", json!({})).await {
            Ok(out) => out["token"].as_str().map(str::to_string),
            Err(e) => return eprintln!("push: no token: {e}"),
        };
        let Some(token) = token else { return };
        let address = one::push::Address { via: "fcm".into(), address: token, ..Default::default() };
        if let Err(e) = device.register_push(&address).await {
            eprintln!("push: can't register: {e}");
        }
    });
}

/// # Safety
/// Called by the JVM, with its own environment and strings.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_client_Native_call<'a>(mut env: JNIEnv<'a>, _: JClass<'a>, kind: JString<'a>, json: JString<'a>) -> jstring {
    let read = |env: &mut JNIEnv, s: &JString| -> String { env.get_string(s).map(Into::into).unwrap_or_default() };
    let (kind, json) = (read(&mut env, &kind), read(&mut env, &json));
    // One of the things the app answers with no window (`headless`): a widget's button (`call`), WorkManager's refresh
    // (`widgets`), a push (`open-note`), its Approve or Not Me (`join`), a new FCM token (`register-push`).
    let out = tauri::async_runtime::block_on(crate::headless::run_here(&kind, &serde_json::from_str(&json).unwrap_or_default()));
    env.new_string(out.to_string()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}
