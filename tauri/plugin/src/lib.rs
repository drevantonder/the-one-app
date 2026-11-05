//! The shell's own natives on an iPhone, in Swift (ios/): a menu as an action sheet, a passkey assertion, push
//! registration, and the App Group the extensions share. Elsewhere each says it isn't there.

use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_one_natives);

pub struct Natives<R: Runtime>(Option<PluginHandle<R>>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("one-natives")
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            let handle = Some(api.register_ios_plugin(init_plugin_one_natives)?);
            #[cfg(not(target_os = "ios"))]
            let handle = {
                let _ = api;
                None
            };
            app.manage(Natives::<R>(handle));
            Ok(())
        })
        .build()
}

impl<R: Runtime> Natives<R> {
    /// Runs one of the Swift plugin's methods, waiting for its answer: call it off the main thread.
    pub fn run(&self, method: &str, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        #[cfg(mobile)]
        if let Some(h) = &self.0 {
            return h.run_mobile_plugin(method, payload).map_err(|e| e.to_string());
        }
        let _ = (method, payload, &self.0);
        Err("Not on this platform.".into())
    }
}
