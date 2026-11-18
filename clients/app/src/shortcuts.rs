//! Each app with a screen of its own, where the device lists its own apps, so it's found and opened as they are: an
//! iPhone's Spotlight and the app's one App Shortcut, Open [app] in The One App, which Siri, Shortcuts and the Action
//! button have too (`gen/apple/Sources/app/Shortcuts.swift`). Each opens its app here by its
//! `one:` link (`links`). They follow what's listed for this device's person (`apps`), so an app installed is added,
//! one uninstalled goes, and a device that's signed out has none.

use std::sync::Mutex;

use serde_json::Value;
use tauri::AppHandle;

/// An app's screen: its entrypoint (`<app>/<page>`), which `one:<entrypoint>` opens, its name, and its icon as a
/// `data:` address.
#[derive(Clone, PartialEq)]
pub struct Shortcut {
    pub entrypoint: String,
    pub name: String,
    pub icon: String,
}

/// The screens listed (`apps`): each page a server lists as an entrypoint, with its name and icon, but the launcher's,
/// which is this app's own.
pub fn listed(listed: &Value) -> Vec<Shortcut> {
    let resources = listed["resources"].as_array().into_iter().flatten();
    let mut shortcuts: Vec<Shortcut> = resources
        .filter(|resource| {
            resource["mimeType"] == "text/html;profile=mcp-app" && resource["_meta"]["category"] != "launcher"
        })
        .filter_map(|resource| {
            let (app, _, page) = super::parts(resource["uri"].as_str()?)?;
            Some(Shortcut {
                entrypoint: format!("{app}/{page}"),
                name: resource["name"].as_str()?.to_owned(),
                icon: resource["icons"][0]["src"].as_str()?.to_owned(),
            })
        })
        .collect();
    shortcuts.sort_by(|one, other| (&one.name, &one.entrypoint).cmp(&(&other.name, &other.entrypoint)));
    shortcuts.dedup_by(|one, other| one.entrypoint == other.entrypoint);
    shortcuts
}

/// What the device has, once it's had any: the last `keep` it took.
static KEPT: Mutex<Option<Vec<Shortcut>>> = Mutex::new(None);

/// Has the device list `shortcuts`, and none else of this app's, unless they're what it has. Each icon is drawn
/// first, so it's done off the thread that asks.
pub fn keep(app: &AppHandle, shortcuts: Vec<Shortcut>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut kept = KEPT.lock().expect("no keep panics");
        if kept.as_ref() == Some(&shortcuts) {
            return;
        }
        match device::keep(&app, &shortcuts) {
            Ok(()) => *kept = Some(shortcuts),
            Err(error) => eprintln!("this device's shortcuts to its apps aren't up to date: {error:#}"),
        }
    });
}

/// An icon as a square PNG `side` pixels across, from its `data:` address: an SVG drawn at that size, or a PNG
/// scaled to it.
#[cfg(target_os = "ios")]
fn png(icon: &str, side: u32) -> anyhow::Result<Vec<u8>> {
    use resvg::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
    use resvg::usvg;
    let (kind, data) = icon
        .strip_prefix("data:")
        .and_then(|icon| icon.split_once(";base64,"))
        .ok_or_else(|| anyhow::anyhow!("an icon is a base64 data: address"))?;
    let data = data_encoding::BASE64.decode(data.as_bytes())?;
    let mut drawn = Pixmap::new(side, side).ok_or_else(|| anyhow::anyhow!("an icon has a size"))?;
    let side = side as f32;
    if kind == "image/svg+xml" {
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default())?;
        let size = tree.size();
        resvg::render(&tree, Transform::from_scale(side / size.width(), side / size.height()), &mut drawn.as_mut());
    } else {
        let picture = Pixmap::decode_png(&data)?;
        let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
        let scale = Transform::from_scale(side / picture.width() as f32, side / picture.height() as f32);
        drawn.draw_pixmap(0, 0, picture.as_ref(), &paint, scale, None);
    }
    Ok(drawn.encode_png()?)
}

/// An iPhone's are what Spotlight indexes and its App Shortcut offers, which its Swift reads from `shortcuts.json` in
/// the app's Application Support folder, as each one's entrypoint, name and icon (`InstalledApp`), and is told to read
/// again (`OneShortcuts.changed`).
#[cfg(target_os = "ios")]
mod device {
    use super::Shortcut;

    pub fn keep(_app: &tauri::AppHandle, shortcuts: &[Shortcut]) -> anyhow::Result<()> {
        use data_encoding::BASE64;
        use objc2::runtime::AnyClass;
        let listed: Vec<_> = shortcuts
            .iter()
            .filter_map(|shortcut| match super::png(&shortcut.icon, 180) {
                Ok(icon) => Some(
                    serde_json::json!({"id": shortcut.entrypoint, "name": shortcut.name, "icon": BASE64.encode(&icon)}),
                ),
                Err(error) => {
                    eprintln!("{}'s icon isn't one a device can show: {error:#}", shortcut.entrypoint);
                    None
                }
            })
            .collect();
        // An iPhone app's home is its own container, and its Application Support folder the one Swift names.
        let folder =
            std::path::Path::new(&std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support");
        std::fs::create_dir_all(&folder)?;
        let file = folder.join("shortcuts.json");
        std::fs::write(file.with_extension("new"), serde_json::to_vec(&listed)?)?;
        std::fs::rename(file.with_extension("new"), &file)?;
        let swift = AnyClass::get(c"OneShortcuts").ok_or_else(|| anyhow::anyhow!("this build has no OneShortcuts"))?;
        let () = unsafe { objc2::msg_send![swift, changed] };
        Ok(())
    }
}

/// Any other device lists nothing of the apps in this one.
#[cfg(not(target_os = "ios"))]
mod device {
    pub fn keep(_app: &tauri::AppHandle, _shortcuts: &[super::Shortcut]) -> anyhow::Result<()> {
        Ok(())
    }
}
