//! A Mac's windows: AppKit's, made and changed on the main thread, which the launcher's event loop runs, while the
//! native thread lays them out and draws them, with wgpu on Metal.

mod events;
mod window;

pub use window::{Display, NativeWindow};

use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSPasteboard, NSPasteboardTypeString, NSWorkspace};
use objc2_foundation::{NSArray, NSString};

/// The Mac's clipboard (the general pasteboard), as GPUI's Mac platform reads and writes text on it. A Mac has no
/// primary selection.
struct Clipboard;

impl crate::editing::Clipboard for Clipboard {
    fn read(&mut self, primary: bool) -> Option<String> {
        if primary {
            return None;
        }
        NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }).map(|text| text.to_string())
    }

    fn write(&mut self, primary: bool, text: &str) {
        if primary {
            return;
        }
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    }
}

/// The Mac's clipboard, for a window's screen.
pub fn clipboard() -> Option<Box<dyn crate::editing::Clipboard>> {
    Some(Box::new(Clipboard))
}

/// Whether the person asked for less motion (System Settings, Accessibility, Display, Reduce motion).
pub fn reduced_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Light or dark, as the Mac is set (System Settings, Appearance), as GPUI's `window_appearance` reads it.
pub fn appearance() -> crate::Appearance {
    dispatch2::run_on_main(|main| of(&NSApplication::sharedApplication(main).effectiveAppearance()))
}

/// Light or dark, as `appearance` is drawn.
fn of(appearance: &NSAppearance) -> crate::Appearance {
    let names = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
    let names = NSArray::from_slice(&names);
    match appearance.bestMatchFromAppearancesWithNames(&names) {
        Some(name) if &*name == unsafe { NSAppearanceNameDarkAqua } => crate::Appearance::Dark,
        _ => crate::Appearance::Light,
    }
}
