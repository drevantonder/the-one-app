//! Each app with a screen of its own, where the device lists its own apps, so it's found and opened as they are: an
//! iPhone's Spotlight and the app's one App Shortcut, Open [app] in the One App, which Siri, Shortcuts and the Action
//! button have too (`gen/apple/Sources/app/Shortcuts.swift`), Windows' Start menu, a Mac's Applications folder, which
//! Spotlight and Launchpad find, and a Linux desktop's applications. Each opens its app as the device's own: on a phone
//! here by its `one:` link (`links`), and on a computer in a window of its own (`host`). They follow what's listed for
//! this device's person (`apps`), so an app installed is added, one uninstalled goes, and a device that's signed out
//! has none. A Mac's and Linux's are kept only by an installed build, as only it takes the device's `one:` links: a
//! developer's would leave behind ones to a build that's gone.
//!
//! An app's icon outside the One App has the One App's ring small in its corner (`badged`), so a person who finds it in
//! Spotlight, Raycast or the Start menu sees whose it is. Android badges its pinned shortcuts with this app's icon
//! itself, and its dynamic ones are in this app's own icon's menu, so they're plain (`MainActivity`).

use std::sync::Mutex;

use serde_json::Value;

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
/// first, so it's done off the thread that asks. A client kept somewhere of its own (`ONE_HOME`), as a network built or
/// tested with sets it, lists nothing: the device's list is its own client's, which it would otherwise replace.
pub fn keep(shortcuts: Vec<Shortcut>) {
    if std::env::var_os("ONE_HOME").is_some() {
        return;
    }
    // One keeps at a time, and what's kept is held only to read or change it: drawing each icon and writing the
    // device's list takes seconds, which an app's window naming its app (`name`) waited for as it started (4 s on
    // the owner's Mac, 20 Nov).
    static KEEPING: Mutex<()> = Mutex::new(());
    std::thread::spawn(move || {
        let _one = KEEPING.lock().expect("no keep panics");
        if KEPT.lock().expect("no keep panics").as_ref() == Some(&shortcuts) {
            return;
        }
        match device::keep(&shortcuts) {
            Ok(()) => *KEPT.lock().expect("no keep panics") = Some(shortcuts),
            Err(error) => eprintln!("this device's shortcuts to its apps aren't up to date: {error:#}"),
        }
    });
}

/// An icon as a square PNG `side` pixels across, from its `data:` address: an SVG drawn at that size, or a PNG
/// scaled to it.
#[cfg(not(target_os = "android"))]
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

/// An app's icon where it leaves the One App (Spotlight, Raycast, the Dock, the Start menu, a launcher), as a square PNG
/// `side` pixels across: on macOS's icon grid, its body 824 of 1024 across in the middle, rounded by 185, with a shadow
/// below it, and the One App's ring small in the bottom right corner, so it reads as that app's, of the One App. It's
/// drawn from the same numbers as `tooling/helper-icon`'s helpers. Inside the One App an icon stays plain.
#[cfg(any(windows, target_os = "linux", target_os = "ios"))]
fn badged(icon: &str, side: u32) -> anyhow::Result<Vec<u8>> {
    Ok(scaled(&badged_pixmap(icon)?, side)?.encode_png()?)
}

/// A 1024 picture at `side`.
#[cfg(not(target_os = "android"))]
fn scaled(whole: &resvg::tiny_skia::Pixmap, side: u32) -> anyhow::Result<resvg::tiny_skia::Pixmap> {
    use resvg::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
    let mut sized = Pixmap::new(side, side).ok_or_else(|| anyhow::anyhow!("an icon has a size"))?;
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    let scale = Transform::from_scale(side as f32 / 1024.0, side as f32 / 1024.0);
    sized.draw_pixmap(0, 0, whole.as_ref(), &paint, scale, None);
    Ok(sized)
}

/// `badged` at 1024.
#[cfg(not(target_os = "android"))]
fn badged_pixmap(icon: &str) -> anyhow::Result<resvg::tiny_skia::Pixmap> {
    use resvg::tiny_skia::{FillRule, FilterQuality, Mask, PathBuilder, Pixmap, PixmapPaint, Transform};
    let ring = Pixmap::decode_png(include_bytes!("../../../icons/ring.png"))?;
    let body = Pixmap::decode_png(&png(icon, 824)?)?;
    let canvas = || Pixmap::new(1024, 1024).ok_or_else(|| anyhow::anyhow!("a pixmap"));
    // The body, in a rounded square whose corners are quarter circles as near as cubic curves draw them.
    let mut app = canvas()?;
    app.draw_pixmap(100, 100, body.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    let (near, far, radius) = (100.0, 924.0, 185.0);
    let k = radius * 0.552_284_8;
    let mut path = PathBuilder::new();
    path.move_to(near + radius, near);
    path.line_to(far - radius, near);
    path.cubic_to(far - radius + k, near, far, near + radius - k, far, near + radius);
    path.line_to(far, far - radius);
    path.cubic_to(far, far - radius + k, far - radius + k, far, far - radius, far);
    path.line_to(near + radius, far);
    path.cubic_to(near + radius - k, far, near, far - radius + k, near, far - radius);
    path.line_to(near, near + radius);
    path.cubic_to(near, near + radius - k, near + radius - k, near, near + radius, near);
    path.close();
    let mut mask = Mask::new(1024, 1024).ok_or_else(|| anyhow::anyhow!("a mask"))?;
    mask.fill_path(&path.finish().ok_or_else(|| anyhow::anyhow!("a rounded square"))?, FillRule::Winding, true, Transform::identity());
    app.apply_mask(&mask);
    // The ring, 340 tall (about two fifths of the body), its right edge 66 and its bottom 66 in from the canvas's.
    let mut corner = canvas()?;
    let scale = 340.0 / ring.height() as f32;
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    let at = Transform::from_scale(scale, scale).post_translate(958.0 - ring.width() as f32 * scale, 618.0);
    corner.draw_pixmap(0, 0, ring.as_ref(), &paint, at, None);
    // Each with a shadow below it, so it reads on any colour.
    let mut whole = canvas()?;
    for (layer, sigma, down, opacity) in [(&app, 10.0, 10, 0.30), (&corner, 14.0, 8, 0.45)] {
        whole.draw_pixmap(0, down, shadow(layer, sigma, opacity).as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        whole.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    Ok(whole)
}

/// What a picture's shape casts: black as opaque as the shape is, to `opacity`, blurred by a Gaussian of `sigma`, which
/// three passes of a box blur make.
#[cfg(not(target_os = "android"))]
fn shadow(layer: &resvg::tiny_skia::Pixmap, sigma: f32, opacity: f32) -> resvg::tiny_skia::Pixmap {
    // Moves `from` to `to` along `lines` lines of `length`, each point the mean of the `radius` either side of it.
    fn blur(from: &[f32], to: &mut [f32], length: usize, lines: usize, along: usize, across: usize, radius: usize) {
        for line in 0..lines {
            let at = |point: usize| line * across + point * along;
            let mut sum: f32 = (0..=radius.min(length - 1)).map(|point| from[at(point)]).sum();
            for point in 0..length {
                to[at(point)] = sum / (2 * radius + 1) as f32;
                if point + radius + 1 < length {
                    sum += from[at(point + radius + 1)];
                }
                if point >= radius {
                    sum -= from[at(point - radius)];
                }
            }
        }
    }
    let (width, height) = (layer.width() as usize, layer.height() as usize);
    let mut alpha: Vec<f32> = layer.pixels().iter().map(|pixel| pixel.alpha() as f32 / 255.0).collect();
    let mut between = vec![0.0; alpha.len()];
    // Three boxes of width w have the variance of a Gaussian of 4σ² + 1 = w².
    let radius = ((4.0 * sigma * sigma + 1.0).sqrt() as usize) / 2;
    for _ in 0..3 {
        blur(&alpha, &mut between, width, height, 1, width, radius);
        blur(&between, &mut alpha, height, width, width, 1, radius);
    }
    let mut cast = resvg::tiny_skia::Pixmap::new(layer.width(), layer.height()).expect("the layer's own size");
    for (pixel, alpha) in cast.data_mut().chunks_exact_mut(4).zip(alpha) {
        pixel[3] = (alpha * opacity * 255.0).round() as u8;
    }
    cast
}

/// An iPhone's are what Spotlight indexes and its App Shortcut offers, which its Swift reads from `shortcuts.json` in
/// the app's Application Support folder, as each one's entrypoint, name and icon (`InstalledApp`), and is told to read
/// again (`OneShortcuts.changed`).
#[cfg(target_os = "ios")]
mod device {
    use super::Shortcut;

    pub fn keep(shortcuts: &[Shortcut]) -> anyhow::Result<()> {
        use data_encoding::BASE64;
        use objc2::runtime::AnyClass;
        let listed: Vec<_> = shortcuts
            .iter()
            .filter_map(|shortcut| match super::badged(&shortcut.icon, 180) {
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

/// Moves a folder to the name it has now if an earlier build named it with other capitals ("The One App" before it was
/// "the One App"), once, so there aren't two: macOS and Windows take both names for one folder, so renaming is how its
/// capitals change. Says whether it moved one.
#[cfg(any(target_os = "macos", windows))]
fn renamed(folder: &std::path::Path) -> anyhow::Result<bool> {
    let (Some(parent), Some(name)) = (folder.parent(), folder.file_name()) else { return Ok(false) };
    let Ok(entries) = std::fs::read_dir(parent) else { return Ok(false) };
    for entry in entries.flatten() {
        let was = entry.file_name();
        if was != name && was.to_string_lossy().eq_ignore_ascii_case(&name.to_string_lossy()) {
            std::fs::rename(entry.path(), folder)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Windows' are shortcuts in the Start menu, in a folder named for this app, each to this program with its `one:` link,
/// and with its icon and an AppUserModelID of its own (`app_id`), which the app's own window says too (`host`), so it
/// groups under its own icon in the taskbar, apart from this app's.
#[cfg(windows)]
mod device {
    use std::path::Path;

    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, SHStrDupW, ShellLink};
    use windows::core::{HSTRING, Interface};

    use super::Shortcut;

    pub fn keep(shortcuts: &[Shortcut]) -> anyhow::Result<()> {
        let start = super::data_dir()?.join("Microsoft/Windows/Start Menu/Programs").join(crate::NAME);
        let icons = super::local_data_dir()?.join("shortcuts");
        super::renamed(&start)?;
        std::fs::create_dir_all(&start)?;
        std::fs::create_dir_all(&icons)?;
        let program = cli::serving::program()?;
        // The shell's COM objects are its thread's, which is this one, made for this.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        let mut made = Vec::new();
        let mut changed = false;
        for shortcut in shortcuts {
            // A file's name has none of what Windows keeps out of one.
            let name: String =
                shortcut.name.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c) && !c.is_control()).collect();
            let file = format!("{}.lnk", name.trim().trim_end_matches('.'));
            if name.trim().is_empty() || made.contains(&file) {
                continue;
            }
            let icon = icons.join(format!("{}.ico", shortcut.entrypoint.replace(['/', '.'], "-")));
            let drawn = match super::badged(&shortcut.icon, 256) {
                Ok(drawn) => drawn,
                Err(error) => {
                    eprintln!("{}'s icon isn't one a device can show: {error:#}", shortcut.entrypoint);
                    continue;
                }
            };
            // The shell keeps an icon by its file's path, so it's told when one is drawn new (an app's own, or the badge
            // an update adds), as it isn't by a file changing.
            let drawn = ico(&drawn)?;
            if std::fs::read(&icon).ok().as_deref() != Some(&drawn) {
                std::fs::write(&icon, drawn)?;
                changed = true;
            }
            let id = super::app_id(&shortcut.entrypoint, None);
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
        if changed {
            unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
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

/// A Mac's are app bundles in `~/Applications/<this app's name>/`, each with the app's name and icon, whose program is
/// a clone of this one signed on its own (`unbundled`), which takes no room on APFS, told which app it is
/// (`LSEnvironment`'s `ONE_APP`), as the program its window runs (`program`): macOS knows a process by its bundle, so
/// it's the copy that gives an app its own
/// Dock icon, Cmd-Tab entry and menu bar. A bundle already as it would be made is left as it is, and one whose program
/// isn't this build's is made again, as this one is updated.
#[cfg(target_os = "macos")]
mod device {
    use std::path::{Path, PathBuf};

    use super::Shortcut;

    pub fn keep(shortcuts: &[Shortcut]) -> anyhow::Result<()> {
        if cfg!(debug_assertions) {
            return Ok(());
        }
        let folder = folder()?;
        // The bundles in a folder that moved are told to LaunchServices again, as it knows them by their path.
        if super::renamed(&folder)? {
            for kept in std::fs::read_dir(&folder)?.flatten() {
                register(&kept.path(), "-f");
            }
        }
        std::fs::create_dir_all(&folder)?;
        let program = cli::serving::program()?;
        let copy = unbundled(&program)?;
        let mut made = Vec::new();
        for shortcut in shortcuts {
            let bundle = bundle(&folder, &shortcut.name);
            if made.contains(&bundle) {
                continue;
            }
            match make(&bundle, shortcut, &program, &copy) {
                Ok(()) => made.push(bundle),
                Err(error) => eprintln!("{} isn't one of this Mac's apps: {error:#}", shortcut.entrypoint),
            }
        }
        for kept in std::fs::read_dir(&folder)?.flatten() {
            if !made.contains(&kept.path()) {
                register(&kept.path(), "-u");
                std::fs::remove_dir_all(kept.path())?;
            }
        }
        if made.is_empty() {
            std::fs::remove_dir(&folder)?;
        }
        Ok(())
    }

    /// Where an app's bundle is.
    pub fn bundle(folder: &Path, name: &str) -> PathBuf {
        let name: String = name.chars().map(|c| if matches!(c, '/' | ':') { '-' } else { c }).collect();
        folder.join(format!("{}.app", name.trim_start_matches('.')))
    }

    /// The folder this app's apps are in.
    pub fn folder() -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("this user has no home folder"))?;
        Ok(home.join("Applications").join(crate::NAME))
    }

    /// This program signed on its own, ad hoc, which each app's bundle has a copy of: it's signed in the One App's
    /// bundle (`cli::serving::bundled`), and macOS kills it in any other, but signed on its own it runs in any. It's
    /// one copy for every bundle's clone of it, where signing each for its own bundle took a whole program's room.
    fn unbundled(program: &Path) -> anyhow::Result<PathBuf> {
        let folder = ::client::home()?.join("shortcuts");
        let copy = folder.join(format!("one-{}", ::client::BUILD));
        if !copy.exists() {
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder)?;
            let signing = folder.join("one.signing");
            std::fs::copy(program, &signing)?;
            let signed = std::process::Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&signing).output()?;
            anyhow::ensure!(signed.status.success(), "codesign didn't sign it: {}", String::from_utf8_lossy(&signed.stderr).trim());
            std::fs::rename(&signing, &copy)?;
        }
        Ok(copy)
    }

    fn make(bundle: &Path, shortcut: &Shortcut, program: &Path, copy: &Path) -> anyhow::Result<()> {
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>one</string>
  <key>CFBundleIdentifier</key><string>{id}</string>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>{build}</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSEnvironment</key>
  <dict>
    <key>ONE_APP</key><string>{entrypoint}</string>
    <key>ONE_PROGRAM</key><string>{program}</string>
  </dict>
</dict>
</plist>
"#,
            id = text(&super::app_id(&shortcut.entrypoint, None)),
            name = text(&shortcut.name),
            build = text(::client::BUILD),
            entrypoint = text(&shortcut.entrypoint),
            program = text(&program.to_string_lossy()),
        );
        let icon = icns(&shortcut.icon)?;
        let made = std::fs::read_to_string(bundle.join("Contents/Info.plist")).is_ok_and(|was| was == plist)
            && std::fs::read(bundle.join("Contents/Resources/AppIcon.icns")).is_ok_and(|was| was == icon);
        if made {
            return Ok(());
        }
        let making = bundle.with_extension("making");
        let _ = std::fs::remove_dir_all(&making);
        std::fs::create_dir_all(making.join("Contents/MacOS"))?;
        std::fs::create_dir_all(making.join("Contents/Resources"))?;
        // A clone: APFS shares its blocks until either changes, and this program is replaced as it's updated, never
        // changed, so its bundles are made again then (their `CFBundleVersion`).
        std::fs::copy(copy, making.join("Contents/MacOS/one"))?;
        std::fs::write(making.join("Contents/Resources/AppIcon.icns"), icon)?;
        std::fs::write(making.join("Contents/Info.plist"), plist)?;
        let _ = std::fs::remove_dir_all(bundle);
        std::fs::rename(&making, bundle)?;
        // LaunchServices reads a bundle's LSEnvironment as it registers it, so it's told of the new one at once.
        register(bundle, "-f");
        Ok(())
    }

    /// Registers a bundle with LaunchServices (`-f`), or has it forgotten (`-u`).
    fn register(bundle: &Path, how: &str) {
        let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
        let _ = std::process::Command::new(lsregister).arg(how).arg(bundle).status();
    }

    /// An app's icon as a Mac app's (`badged`), as an .icns of PNGs at the sizes macOS asks for.
    fn icns(icon: &str) -> anyhow::Result<Vec<u8>> {
        let whole = super::badged_pixmap(icon)?;
        let mut family = icns::IconFamily::new();
        for (kind, side) in [(*b"ic10", 1024), (*b"ic09", 512), (*b"ic08", 256), (*b"ic07", 128), (*b"ic12", 64), (*b"ic11", 32)] {
            family.elements.push(icns::IconElement::new(icns::OSType(kind), super::scaled(&whole, side)?.encode_png()?));
        }
        let mut icns = Vec::new();
        family.write(&mut icns)?;
        Ok(icns)
    }
}

/// Linux's are desktop entries in the user's applications (`~/.local/share/applications`), named after each app's id
/// (`app_id`), which its window's class is too (`StartupWMClass`), each starting this program as that app's window, with
/// its icon beside this app's data.
#[cfg(target_os = "linux")]
mod device {
    use super::Shortcut;

    pub fn keep(shortcuts: &[Shortcut]) -> anyhow::Result<()> {
        if cfg!(debug_assertions) {
            return Ok(());
        }
        let entries = super::data_dir()?.join("applications");
        let icons = super::local_data_dir()?.join("shortcuts");
        std::fs::create_dir_all(&entries)?;
        std::fs::create_dir_all(&icons)?;
        let program = cli::serving::program()?;
        // A desktop entry's Exec quotes each argument, and escapes what's special inside the quotes.
        let quoted = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"));
        let mut made = Vec::new();
        for shortcut in shortcuts {
            let id = super::app_id(&shortcut.entrypoint, None);
            let icon = icons.join(format!("{id}.png"));
            match super::badged(&shortcut.icon, 256) {
                Ok(drawn) => std::fs::write(&icon, drawn)?,
                Err(error) => {
                    eprintln!("{}'s icon isn't one a device can show: {error:#}", shortcut.entrypoint);
                    continue;
                }
            }
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName={}\nExec=env {} {}\nIcon={}\nStartupWMClass={id}\nTerminal=false\n",
                shortcut.name.replace('\n', " "),
                quoted(&format!("ONE_APP={}", shortcut.entrypoint)),
                quoted(&program.to_string_lossy()),
                icon.display(),
            );
            let file = format!("{id}.desktop");
            std::fs::write(entries.join(&file), entry)?;
            made.push(id);
        }
        // What's no longer listed goes, its icon with it.
        let ours = format!("{}.", ::client::NAME);
        for kept in std::fs::read_dir(&entries)?.flatten() {
            let name = kept.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".desktop").filter(|id| id.starts_with(&ours)) {
                if !made.iter().any(|made| made == id) {
                    std::fs::remove_file(kept.path())?;
                    let _ = std::fs::remove_file(icons.join(format!("{id}.png")));
                }
            }
        }
        Ok(())
    }
}

/// Android lists its apps itself (`shortcuts` in page/shortcuts.js).
#[cfg(target_os = "android")]
mod device {
    pub fn keep(_shortcuts: &[super::Shortcut]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// An app's screen's id on a computer, as each system names an app: Windows' AppUserModelID, which it groups the app's
/// windows by in the taskbar, a Mac bundle's identifier, and a Linux window's class. It's this app's identifier, then
/// the app's name, its page's if it isn't the app's first, and who it's for, if not its user, last part first:
/// `app.theone.client.notes`, `app.theone.client.sign-in.users`, `app.theone.client.notes.family.groups`.
#[cfg(desktop)]
pub fn app_id(entrypoint: &str, subject: Option<&str>) -> String {
    let mut parts = entrypoint.split('/');
    let mut id = parts.next().unwrap_or_default().to_owned();
    let page: Vec<&str> = parts.collect();
    if page != ["client", "index.html"] {
        let page = page.iter().filter(|part| **part != "client").copied().collect::<Vec<_>>().join("-");
        id = format!("{id}.{}", page.trim_end_matches(".html"));
    }
    for part in subject.into_iter().flat_map(|subject| subject.split('/').rev()) {
        id = format!("{id}.{part}");
    }
    let id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' }).collect();
    format!("{}.{id}", ::client::NAME)
}

/// An app's screen's name, as its shortcut has it, once this device has listed them.
#[cfg(desktop)]
pub fn name(entrypoint: &str) -> Option<String> {
    let kept = KEPT.lock().expect("no keep panics");
    kept.iter().flatten().find(|shortcut| shortcut.entrypoint == entrypoint).map(|shortcut| shortcut.name.clone())
}

/// Where this user's data is (Linux's `~/.local/share`, Windows' roaming AppData), as Tauri said it.
#[cfg(any(windows, target_os = "linux"))]
fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    dirs::data_dir().ok_or_else(|| anyhow::anyhow!("this user has no data folder"))
}

/// This app's own data on this machine, as Tauri said it: its identifier's folder in the user's local data.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn local_data_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(dirs::data_local_dir().ok_or_else(|| anyhow::anyhow!("this user has no data folder"))?.join(::client::NAME))
}
