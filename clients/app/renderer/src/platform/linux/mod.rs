//! A Linux desktop's windows: X11's, which every desktop has (Wayland's run X11 apps through XWayland).

mod keyboard;
mod x11;
mod xim_handler;

pub use x11::{Display, Handle, NativeWindow};

/// The desktop's clipboard and primary selection, as arboard serves them, one for every window on this thread.
struct Clipboard(std::rc::Rc<std::cell::RefCell<arboard::Clipboard>>);

impl crate::editing::Clipboard for Clipboard {
    fn read(&mut self, primary: bool) -> Option<String> {
        use arboard::GetExtLinux;
        self.0.borrow_mut().get().clipboard(kind(primary)).text().ok()
    }

    fn write(&mut self, primary: bool, text: &str) {
        use arboard::SetExtLinux;
        if let Err(error) = self.0.borrow_mut().set().clipboard(kind(primary)).text(text.to_owned()) {
            log::warn!("nothing went on the clipboard: {error}");
        }
    }
}

fn kind(primary: bool) -> arboard::LinuxClipboardKind {
    if primary { arboard::LinuxClipboardKind::Primary } else { arboard::LinuxClipboardKind::Clipboard }
}

/// The desktop's clipboard, for a window's screen, where it has one.
pub fn clipboard() -> Option<Box<dyn crate::editing::Clipboard>> {
    thread_local! {
        static SHARED: Option<std::rc::Rc<std::cell::RefCell<arboard::Clipboard>>> = arboard::Clipboard::new()
            .map_err(|error| log::warn!("there's no clipboard: {error}"))
            .ok()
            .map(|clipboard| std::rc::Rc::new(std::cell::RefCell::new(clipboard)));
    }
    SHARED.with(|shared| shared.clone()).map(|shared| Box::new(Clipboard(shared)) as Box<dyn crate::editing::Clipboard>)
}

/// A setting of the desktop's appearance, as its portal says it (`org.freedesktop.appearance`), as GPUI's Linux
/// platform reads `color-scheme`.
fn appearance_setting(key: &str) -> Option<u64> {
    use dbus::arg::{RefArg, Variant};
    let connection = dbus::blocking::Connection::new_session().ok()?;
    let portal = connection.with_proxy("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", std::time::Duration::from_millis(500));
    let (value,): (Variant<Box<dyn RefArg>>,) = portal.method_call("org.freedesktop.portal.Settings", "ReadOne", ("org.freedesktop.appearance", key)).ok()?;
    value.0.as_u64()
}

/// Light or dark: `color-scheme`'s 1 is dark, 2 light, 0 no preference.
pub fn appearance() -> crate::Appearance {
    match appearance_setting("color-scheme") {
        Some(1) => crate::Appearance::Dark,
        _ => crate::Appearance::Light,
    }
}

/// Says each time the desktop goes light or dark, as its portal signals (`SettingChanged`), for as long as the app
/// runs.
pub(super) fn appearance_changes(to: calloop::channel::Sender<crate::Appearance>) {
    use dbus::arg::{RefArg, Variant};
    let Ok(connection) = dbus::blocking::Connection::new_session() else { return };
    let rule = dbus::message::MatchRule::new_signal("org.freedesktop.portal.Settings", "SettingChanged");
    let changed = move |(namespace, key, value): (String, String, Variant<Box<dyn RefArg>>), _: &dbus::blocking::Connection, _: &dbus::Message| {
        if namespace != "org.freedesktop.appearance" || key != "color-scheme" {
            return true;
        }
        to.send(if value.0.as_u64() == Some(1) { crate::Appearance::Dark } else { crate::Appearance::Light }).is_ok()
    };
    if let Err(error) = connection.add_match(rule, changed) {
        return log::warn!("the desktop going light or dark isn't heard: {error}");
    }
    while connection.process(std::time::Duration::from_secs(3600)).is_ok() {}
}

/// Whether the person asked for less motion: `reduced-motion`'s 1, 0 being no preference.
pub fn reduced_motion() -> bool {
    appearance_setting("reduced-motion") == Some(1)
}
