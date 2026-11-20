//! An iPhone's "On Charger" App Intent (`OnCharger` in Shortcuts.swift), which a Shortcuts automation runs as the phone
//! is plugged in or unplugged, without opening the app: it tells Device Activity whether this device is on its
//! charger (`server-load_power`), through the client this process opens as it starts, and says how that went to the
//! intent (`OneChargerReported` in Shortcuts.swift). Swift reaches it by its name, `OneCharger`, as Rust reaches
//! Swift's `OneShortcuts`.

use std::ffi::{CStr, c_char};

use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use serde_json::{Value, json};

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "OneCharger"]
    struct Charger;

    impl Charger {
        /// What the intent says, an NSString of JSON: its `id`, which the answer names, and whether it's `charging`.
        #[unsafe(method(report:))]
        fn report(said: &AnyObject) {
            let text: *const c_char = unsafe { msg_send![said, UTF8String] };
            let said: Value = (!text.is_null())
                .then(|| unsafe { CStr::from_ptr(text) }.to_str().ok().and_then(|text| serde_json::from_str(text).ok()))
                .flatten()
                .unwrap_or_default();
            let id = said["id"].as_i64().unwrap_or(0) as isize;
            let charging = said["charging"].as_bool().unwrap_or(false);
            crate::spawn(async move {
                let reported = told(json!({"charging": charging, "name": ::client::name()})).await;
                if let Err(error) = &reported {
                    eprintln!("Device Activity wasn't told this device's charger: {error}");
                }
                let Some(swift) = AnyClass::get(c"OneChargerReported") else { return };
                let () = unsafe { msg_send![swift, reported: id, ok: reported.is_ok()] };
            });
        }
    }
);

/// Makes `OneCharger` one Swift can find by its name, as the app starts.
pub fn register() {
    let _ = Charger::class();
}

/// Tells Device Activity, once the client has opened: as long as iOS lets an App Intent run (about 30 s), since a
/// Shortcut that started the app runs as its client is still opening.
async fn told(arguments: Value) -> Result<(), String> {
    let opened = async {
        let mut start = crate::app().start.subscribe();
        loop {
            if let Ok(views) = crate::views() {
                return Ok::<_, String>(views);
            }
            start.changed().await.map_err(|error| error.to_string())?;
        }
    };
    let views = tokio::time::timeout(std::time::Duration::from_secs(25), opened).await.map_err(|_| "the app's client didn't open in time".to_owned())??;
    let answer = views.client.request("tools/call", json!({"name": "server-load_power", "arguments": arguments})).await.map_err(|error| format!("{error:#}"))?;
    if answer["isError"] == true {
        return Err(answer["content"][0]["text"].as_str().unwrap_or("Device Activity refused it").to_owned());
    }
    Ok(())
}
