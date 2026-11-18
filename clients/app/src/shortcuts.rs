//! Each app with a screen of its own, where the device lists its own apps, so it's found and opened as they are: an
//! iPhone's Spotlight and the app's one App Shortcut, Open [app] in The One App, which Siri, Shortcuts and the Action
//! button have too (`gen/apple/Sources/app/Shortcuts.swift`), and Windows' Start menu. Each opens its app here by its
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
#[cfg(any(target_os = "ios", windows))]
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

/// Windows' are shortcuts in the Start menu, in a folder named for this app, each to this program with its `one:` link,
/// and with its icon and an AppUserModelID of its own (`app_user_model_id`), so a window of its own groups under its own
/// icon in the taskbar, apart from this app's.
#[cfg(windows)]
mod device {
    use std::path::Path;

    use tauri::Manager;
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, SHStrDupW, ShellLink};
    use windows::core::{HSTRING, Interface};

    use super::Shortcut;

    pub fn keep(app: &tauri::AppHandle, shortcuts: &[Shortcut]) -> anyhow::Result<()> {
        let identifier = &app.config().identifier;
        let start = app.path().data_dir()?.join("Microsoft/Windows/Start Menu/Programs").join(&app.package_info().name);
        let icons = app.path().app_local_data_dir()?.join("shortcuts");
        std::fs::create_dir_all(&start)?;
        std::fs::create_dir_all(&icons)?;
        let program = std::env::current_exe()?;
        // The shell's COM objects are its thread's, which is this one, made for this.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        let mut made = Vec::new();
        for shortcut in shortcuts {
            // A file's name has none of what Windows keeps out of one.
            let name: String =
                shortcut.name.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c) && !c.is_control()).collect();
            let file = format!("{}.lnk", name.trim().trim_end_matches('.'));
            if name.trim().is_empty() || made.contains(&file) {
                continue;
            }
            let icon = icons.join(format!("{}.ico", shortcut.entrypoint.replace(['/', '.'], "-")));
            let drawn = match super::png(&shortcut.icon, 256) {
                Ok(drawn) => drawn,
                Err(error) => {
                    eprintln!("{}'s icon isn't one a device can show: {error:#}", shortcut.entrypoint);
                    continue;
                }
            };
            std::fs::write(&icon, ico(&drawn)?)?;
            let id = super::app_user_model_id(identifier, &shortcut.entrypoint);
            link(&start.join(&file), &program, &format!("one:{}", shortcut.entrypoint), &icon, &shortcut.name, &id)?;
            made.push(file);
        }
        // What's no longer listed goes, its icon with it.
        for kept in std::fs::read_dir(&start)?.flatten() {
            if !made.iter().any(|file| kept.file_name() == file.as_str()) {
                std::fs::remove_file(kept.path())?;
            }
        }
        for kept in std::fs::read_dir(&icons)?.flatten() {
            let named = kept.path().file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
            if !shortcuts.iter().any(|shortcut| shortcut.entrypoint.replace(['/', '.'], "-") == named) {
                std::fs::remove_file(kept.path())?;
            }
        }
        // With none, its folder goes too, so the Start menu has no empty one.
        if made.is_empty() {
            std::fs::remove_dir(&start)?;
            std::fs::remove_dir(&icons)?;
        }
        Ok(())
    }

    /// An icon file of one picture, a PNG, which Windows reads from Vista on.
    fn ico(png: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut icon = ico::IconDir::new(ico::ResourceType::Icon);
        icon.add_entry(ico::IconDirEntry::encode_as_png(&ico::IconImage::read_png(std::io::Cursor::new(png))?)?);
        let mut file = Vec::new();
        icon.write(&mut file)?;
        Ok(file)
    }

    /// Saves a shortcut at `file` to `program` with `arguments`, its `icon`, `description` and AppUserModelID `id`.
    fn link(
        file: &Path,
        program: &Path,
        arguments: &str,
        icon: &Path,
        description: &str,
        id: &str,
    ) -> anyhow::Result<()> {
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(program))?;
            link.SetArguments(&HSTRING::from(arguments))?;
            link.SetIconLocation(&HSTRING::from(icon), 0)?;
            link.SetDescription(&HSTRING::from(description))?;
            // As InitPropVariantFromString makes one, which the SDK has only inline: its string the COM allocator's,
            // which the shell frees as that, and cleared once the store has its copy.
            let mut value = PROPVARIANT {
                Anonymous: PROPVARIANT_0 {
                    Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                        vt: VT_LPWSTR,
                        Anonymous: PROPVARIANT_0_0_0 { pwszVal: SHStrDupW(&HSTRING::from(id))? },
                        ..Default::default()
                    }),
                },
            };
            let store: IPropertyStore = link.cast()?;
            let set = store.SetValue(&PKEY_AppUserModel_ID, &value);
            PropVariantClear(&mut value)?;
            set?;
            store.Commit()?;
            link.cast::<IPersistFile>()?.Save(&HSTRING::from(file), true)?;
        }
        Ok(())
    }
}

/// Any other device lists nothing of the apps in this one.
#[cfg(not(any(target_os = "ios", windows)))]
mod device {
    pub fn keep(_app: &tauri::AppHandle, _shortcuts: &[super::Shortcut]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// The AppUserModelID of an app here, by its entrypoint's app, which Windows groups its windows under, in the taskbar,
/// apart from this app's own (its identifier, `run`): `<identifier>.<app>`.
#[cfg(windows)]
pub fn app_user_model_id(identifier: &str, entrypoint: &str) -> String {
    format!("{identifier}.{}", entrypoint.split('/').next().unwrap_or(entrypoint))
}
