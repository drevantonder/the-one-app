//! A Mac's windows: AppKit's, made and changed on the main thread, which the launcher's event loop runs, while the
//! native thread lays them out and draws them, with wgpu on Metal.

mod events;
mod window;

pub use window::{Display, Handle, NativeWindow};

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

/// Shows the system's own picker of files to open, as rfd shows it, of `accept`'s kinds where they're extensions
/// (`.png`), and tells `chosen` the files chosen, none where it was cancelled.
pub fn open_files(accept: &[String], multiple: bool, chosen: Box<dyn FnOnce(Vec<std::path::PathBuf>) + Send>) {
    let extensions: Vec<String> = accept.iter().filter_map(|kind| kind.strip_prefix('.')).map(str::to_owned).collect();
    let mut dialog = rfd::AsyncFileDialog::new();
    if !extensions.is_empty() {
        dialog = dialog.add_filter("", &extensions);
    }
    std::thread::spawn(move || {
        let files = futures_lite::future::block_on(async move {
            if multiple {
                dialog.pick_files().await.unwrap_or_default()
            } else {
                dialog.pick_file().await.into_iter().collect()
            }
        });
        chosen(files.into_iter().map(|file| file.path().to_owned()).collect());
    });
}
