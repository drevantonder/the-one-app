//! A Linux desktop's windows: X11's, which every desktop has (Wayland's run X11 apps through XWayland).

mod keyboard;
mod x11;

pub use x11::{Display, NativeWindow};

/// Light or dark, as the desktop says through its portal (`org.freedesktop.appearance` `color-scheme`: 1 is dark,
/// 2 light, 0 no preference), as GPUI's Linux platform reads it.
pub fn appearance() -> crate::Appearance {
    use dbus::arg::{RefArg, Variant};
    let read = || -> Result<u64, dbus::Error> {
        let connection = dbus::blocking::Connection::new_session()?;
        let portal = connection.with_proxy("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", std::time::Duration::from_millis(500));
        let (scheme,): (Variant<Box<dyn RefArg>>,) =
            portal.method_call("org.freedesktop.portal.Settings", "ReadOne", ("org.freedesktop.appearance", "color-scheme"))?;
        Ok(scheme.0.as_u64().unwrap_or(0))
    };
    match read() {
        Ok(1) => crate::Appearance::Dark,
        _ => crate::Appearance::Light,
    }
}
