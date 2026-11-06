//! What the Mac and the iPhone share: their bundle's Info.plist, and the defaults the old Swift apps kept, which this
//! one takes over once (their address and sign-in), so a signed-in Mac or iPhone stays signed in.

use std::path::Path;

use objc2::rc::Retained;
use objc2_foundation::{NSString, NSUserDefaults};

/// A string in the bundle's Info.plist (`AppGroup`, `KeychainGroup`), if it has one.
pub fn info(key: &str) -> Option<String> {
    use core_foundation::string::CFString;
    let info = core_foundation::bundle::CFBundle::main_bundle().info_dictionary();
    let value = info.find(CFString::new(key))?;
    value.downcast::<CFString>().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// Whether the bundle's Info.plist says `key` is true (`Push`, in a build with pushes).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn info_true(key: &str) -> bool {
    use core_foundation::string::CFString;
    let info = core_foundation::bundle::CFBundle::main_bundle().info_dictionary();
    let value = info.find(CFString::new(key));
    value.and_then(|v| v.downcast::<core_foundation::boolean::CFBoolean>()).is_some_and(bool::from)
}

pub fn defaults() -> Retained<NSUserDefaults> {
    NSUserDefaults::standardUserDefaults()
}

fn string(key: &str) -> Option<String> {
    defaults().stringForKey(&NSString::from_str(key)).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// What the old app kept in its defaults, taken over the first time (`data` has no address yet): the address, an
/// invite's secret, its sign-in's client (if it came back where this one does), and the client's own folder; on a Mac,
/// its Open at Login and the widgets hidden from the menu bar.
pub fn take_over(data: &Path) {
    if data.join("address").exists() {
        return;
    }
    let Some(address) = string("address") else { return };
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(data.join("address"), &address);
    if let Some(invite) = string("invite") {
        let _ = std::fs::write(data.join("invite"), invite);
    }
    // The Mac's came back to the loopback, the iPhone's to its own link.
    let back = if cfg!(target_os = "ios") { format!("{}://signed-in/", one::config::CLI) } else { "http://127.0.0.1/callback".into() };
    if let Some(client) = string("client").filter(|_| string("clientRedirect").as_deref() == Some(back.as_str())) {
        let _ = std::fs::write(data.join("sign-in"), format!("{address}\n{client}\n{back}"));
    }
    #[cfg(target_os = "macos")]
    {
        // Open at login was set the first time it opened; the person's choice since stays (lib.rs `tray`).
        if defaults().boolForKey(&NSString::from_str("loginItemSet")) {
            let _ = std::fs::write(data.join("opened-at-login"), "");
        }
        // The widgets they hid from the menu bar (menubar.rs).
        if let Some(hidden) = defaults().stringArrayForKey(&NSString::from_str("hiddenWidgets")) {
            let kinds: Vec<String> = hidden.iter().map(|k| k.to_string()).collect();
            let _ = std::fs::write(data.join("menubar-hidden"), kinds.join("\n"));
        }
    }
    // Its client's folder: Application Support/client on an iPhone, the Mac app's own folder's on a Mac.
    let support = data.parent().map(Path::to_path_buf);
    #[cfg(target_os = "macos")]
    let old = support.map(|s| s.join(one::config::NAME).join(one::config::CLI).join("client"));
    #[cfg(target_os = "ios")]
    let old = support.map(|s| s.join("client"));
    if let Some(old) = old.filter(|o| o.is_dir())
        && !data.join("client").exists()
    {
        let _ = std::fs::rename(old, data.join("client"));
    }
}
