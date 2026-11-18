//! A Linux desktop's windows: X11's, which every desktop has (Wayland's run X11 apps through XWayland).

mod keyboard;
mod x11;

pub use x11::{Display, NativeWindow};

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

/// Whether the person asked for less motion: `reduced-motion`'s 1, 0 being no preference.
pub fn reduced_motion() -> bool {
    appearance_setting("reduced-motion") == Some(1)
}
