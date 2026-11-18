//! A strict screen drawn by our own renderer on Android: its JS runs in QuickJS (rquickjs, the desktop host's
//! sandbox, `js`), its ops build the tree (`tree`, shared with the desktop host), taffy lays it out, GPUI's scene and
//! Blade renderer draw it with Vulkan, cosmic-text sets its text, and the Java view (StrictView) bridges the
//! keyboard, autofill and the screen reader to it. Everything runs on Android's UI thread.

#[path = "../../../../clients/app/strict/src/js.rs"]
pub mod js;
#[path = "../../../../clients/app/strict/src/tokens.rs"]
pub mod tokens;
#[path = "../../../../clients/app/strict/src/tree.rs"]
pub mod tree;

mod a11y;
mod atlas;
mod edit;
mod renderer;
mod scene;
mod text;
mod view;

use std::cell::RefCell;
use std::ffi::{CString, c_void};
use std::io::Read;
use std::ptr::NonNull;
use std::time::Instant;

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring};
use serde_json::{Value, json};

use edit::Edit;
use tree::{Id, Kind};
use view::View;

/// The version of the strict protocol this host speaks.
pub const VERSION: u32 = 1;

struct Window(ndk::native_window::NativeWindow);

impl raw_window_handle::HasWindowHandle for Window {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::AndroidNdkWindowHandle::new(NonNull::new(self.0.ptr().as_ptr() as *mut c_void).expect("a window"));
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw_window_handle::RawWindowHandle::AndroidNdk(handle)) })
    }
}

impl raw_window_handle::HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(raw_window_handle::RawDisplayHandle::Android(raw_window_handle::AndroidDisplayHandle::new())) })
    }
}

struct Touch {
    start: (f32, f32),
    scroller: Option<(Id, f32)>,
    dragging: bool,
}

pub struct App {
    sandbox: js::Sandbox,
    view: View,
    renderer: Option<(renderer::Renderer, Window)>,
    touch: Option<Touch>,
    a11y: Option<a11y::A11y>,
}

fn read_asset(assets: &ndk::asset::AssetManager, path: &str) -> anyhow::Result<Vec<u8>> {
    let mut asset = assets.open(&CString::new(path)?).ok_or_else(|| anyhow::anyhow!("no asset {path}"))?;
    let mut bytes = Vec::new();
    asset.read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl App {
    /// Applies what the JS posted since last asked, and lays the tree out again if it changed.
    fn pump(&mut self) {
        let mut changed = false;
        for posted in self.sandbox.take() {
            if let js::Posted::Update(json) = posted {
                match serde_json::from_str(&json).map_err(anyhow::Error::from).and_then(|update| self.view.tree.apply(update)) {
                    Ok(()) => changed = true,
                    Err(error) => log::error!("update refused: {error:#}"),
                }
            }
        }
        if changed {
            self.view.layout();
        }
    }

    fn event(&mut self, id: Id, name: &str, extra: Value) {
        // The shared tree's `listens` makes `keydown`'s prop `onKeydown`; JSX's (and the protocol's props') is `onKeyDown`.
        let listens = |node: &tree::Node| if name == "keydown" { node.is("onKeyDown") } else { node.listens(name) };
        if !self.view.tree.nodes.get(&id).is_some_and(listens) {
            return;
        }
        let mut event = json!({ "v": VERSION, "op": "event", "id": id, "name": name });
        if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
            event.extend(extra);
        }
        let started = Instant::now();
        if let Err(error) = self.sandbox.send("event", &event) {
            log::error!("event: {error:#}");
        }
        let js = started.elapsed();
        self.pump();
        log::info!("{name}: js {js:?}, ops and layout {:?}", started.elapsed() - js);
    }

    fn draw(&mut self) {
        let Some((renderer, _)) = self.renderer.as_mut() else { return };
        let started = Instant::now();
        self.view.paint(renderer);
        self.view.scene.dark = self.view.dark;
        let scene = started.elapsed();
        renderer.draw(&self.view.scene);
        log::info!(
            "drew: scene {scene:?} ({} quads, {} glyphs, {} images), gpu {:?}",
            self.view.scene.quads.len(),
            self.view.scene.monochrome_sprites.len(),
            self.view.scene.polychrome_sprites.len(),
            started.elapsed() - scene
        );
        if let Some(a11y) = self.a11y.as_mut() {
            a11y.update(&self.view);
        }
    }

    /// What the bridge needs to start editing `id`: how the keyboard is to behave, and the field's state.
    fn focus_command(&mut self, id: Id) -> Value {
        let node = &self.view.tree.nodes[&id];
        let edit = self.view.edits.entry(id).or_insert_with(|| Edit { secure: node.str("type") == Some("password"), ..Default::default() }).clone();
        let (base, extent, cs, ce) = edit.utf16();
        let bounds = self.view.bounds(id).unwrap_or_default();
        json!({
            "focus": {
                "id": id,
                "type": node.str("type").unwrap_or("text"),
                "autocomplete": node.str("autocomplete"),
                "enterkeyhint": node.str("enterkeyhint"),
                "autocapitalize": node.str("autocapitalize"),
                "spellcheck": node.prop("spellcheck").map(|v| v != &json!(false) && v != &json!("false")).unwrap_or(true),
                "label": node.str("aria-label").or(node.str("placeholder")),
                "bounds": [bounds.origin[0], bounds.origin[1], bounds.size[0], bounds.size[1]],
            },
            "state": { "text": edit.text, "base": base, "extent": extent, "composingStart": cs, "composingEnd": ce },
        })
    }

    fn state_command(&self, id: Id) -> Value {
        let edit = &self.view.edits[&id];
        let (base, extent, cs, ce) = edit.utf16();
        json!({ "state": { "id": id, "text": edit.text, "base": base, "extent": extent, "composingStart": cs, "composingEnd": ce } })
    }

    fn set_focus(&mut self, id: Option<Id>) -> Option<Value> {
        if self.view.focus == id {
            return None;
        }
        if let Some(old) = self.view.focus.take() {
            self.event(old, "blur", json!({}));
        }
        self.view.focus = id;
        match id {
            Some(id) => {
                self.event(id, "focus", json!({}));
                Some(self.focus_command(id))
            }
            None => Some(json!({ "blur": true })),
        }
    }

    fn tap(&mut self, x: f32, y: f32) -> Option<Value> {
        let target = self.view.target(x, y);
        match target {
            Some(id) if self.view.tree.nodes[&id].kind == Kind::Input => {
                let byte = self.view.input_byte(id, x);
                let focus = self.set_focus(Some(id));
                let edit = self.view.edits.get_mut(&id).expect("focused");
                edit.set_caret(byte);
                let state = self.state_command(id);
                Some(merge(focus, state))
            }
            Some(id) => {
                let blur = self.set_focus(None);
                self.event(id, "click", json!({}));
                blur
            }
            None => self.set_focus(None),
        }
    }

    fn action(&mut self, id: Id, action: &str) -> Option<Value> {
        match action {
            "next" => {
                let inputs = self.view.inputs();
                let next = inputs.iter().position(|i| *i == id).and_then(|at| inputs.get(at + 1)).copied();
                match next {
                    Some(next) => self.set_focus(Some(next)),
                    None => self.set_focus(None),
                }
            }
            _ => {
                self.event(id, "keydown", json!({ "key": "Enter" }));
                if action == "done" || action == "go" { self.set_focus(None) } else { None }
            }
        }
    }
}

fn merge(a: Option<Value>, b: Value) -> Value {
    match (a, b) {
        (Some(Value::Object(mut a)), Value::Object(b)) => {
            a.extend(b);
            Value::Object(a)
        }
        (_, b) => b,
    }
}

fn app<'a>(handle: jlong) -> &'a RefCell<App> {
    unsafe { &*(handle as *const RefCell<App>) }
}

fn string(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn answer(env: &mut JNIEnv, value: Option<Value>) -> jstring {
    match value {
        Some(value) => env.new_string(value.to_string()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeCreate(
    mut env: JNIEnv,
    _: JClass,
    assets: JObject,
    page: JString,
    cache: JString,
    density: jfloat,
    dark: jboolean,
) -> jlong {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("StrictSpike"));
    renderer::warm();
    let started = Instant::now();
    let page = string(&mut env, &page);
    let cache = string(&mut env, &cache);
    let manager = unsafe { ndk_sys::AAssetManager_fromJava(env.get_raw() as *mut _, assets.as_raw() as *mut _) };
    let assets = unsafe { ndk::asset::AssetManager::from_ptr(NonNull::new(manager).expect("an asset manager")) };
    let images = unsafe { ndk::asset::AssetManager::from_ptr(NonNull::new(manager).expect("an asset manager")) };
    let files: js::Files = Box::new(move |url: &url::Url| Ok(String::from_utf8(read_asset(&assets, url.path().trim_start_matches('/'))?)?));
    let result = (|| -> anyhow::Result<App> {
        let page = url::Url::parse(&page)?;
        let folder = page.path().rsplit_once('/').map(|(folder, _)| folder.trim_start_matches('/').to_owned()).unwrap_or_default();
        let sandbox = js::Sandbox::start(&page, files, Some(cache.into()), js::Clock::real())?;
        let images = Box::new(move |src: &str| read_asset(&images, &format!("{folder}/{src}")).ok());
        let view = View::new(tree::Tree::default(), density, dark != 0, images)?;
        let mut app = App { sandbox, view, renderer: None, touch: None, a11y: None };
        app.pump();
        Ok(app)
    })();
    match result {
        Ok(app) => {
            log::info!("started its JS in {:?}", started.elapsed());
            Box::into_raw(Box::new(RefCell::new(app))) as jlong
        }
        Err(error) => {
            log::error!("didn't start: {error:#}");
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeSurface(env: JNIEnv, _: JClass, handle: jlong, surface: JObject, width: jint, height: jint) {
    let mut app = app(handle).borrow_mut();
    let started = Instant::now();
    let window = unsafe { ndk::native_window::NativeWindow::from_surface(env.get_raw() as *mut _, surface.as_raw() as *mut _) };
    let Some(window) = window else { return };
    let same = app.renderer.as_ref().is_some_and(|(_, w)| w.0.ptr() == window.ptr());
    if same {
        app.renderer.as_mut().expect("there").0.resize(width as u32, height as u32);
    } else {
        if let Some((mut renderer, _)) = app.renderer.take() {
            renderer.destroy();
        }
        let window = Window(window);
        let made = renderer::context().and_then(|gpu| renderer::Renderer::new(gpu, &window, blade_graphics::Extent { width: width as u32, height: height as u32, depth: 1 }));
        match made {
            Ok(renderer) => app.renderer = Some((renderer, window)),
            Err(error) => {
                log::error!("no renderer: {error:#}");
                return;
            }
        }
        log::info!("renderer made in {:?}", started.elapsed());
    }
    app.view.size = (width as f32, height as f32);
    app.view.layout();
    app.draw();
    log::info!("first drawn {:?} after the surface", started.elapsed());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeSurfaceDestroyed(_: JNIEnv, _: JClass, handle: jlong) {
    if let Some((mut renderer, _)) = app(handle).borrow_mut().renderer.take() {
        renderer.destroy();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeDestroy(_: JNIEnv, _: JClass, handle: jlong) {
    let app = unsafe { Box::from_raw(handle as *mut RefCell<App>) };
    let mut app = app.into_inner();
    if let Some((mut renderer, _)) = app.renderer.take() {
        renderer.destroy();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeInsets(_: JNIEnv, _: JClass, handle: jlong, top: jint, bottom: jint) {
    let mut app = app(handle).borrow_mut();
    if app.view.insets != (top as f32, bottom as f32) {
        app.view.insets = (top as f32, bottom as f32);
        app.view.layout();
        app.draw();
    }
}

/// A finger: `action` is MotionEvent's (0 down, 1 up, 2 move, 3 cancel). → what the bridge is to do, as JSON.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeTouch(mut env: JNIEnv, _: JClass, handle: jlong, action: jint, x: jfloat, y: jfloat) -> jstring {
    let mut app = app(handle).borrow_mut();
    let slop = 8.0 * app.view.density;
    let command = match action {
        0 => {
            let scroller = app.view.scroller(x, y).map(|id| (id, app.view.scroll.get(&id).copied().unwrap_or(0.0)));
            app.touch = Some(Touch { start: (x, y), scroller, dragging: false });
            None
        }
        2 => {
            let Some(touch) = app.touch.as_mut() else { return std::ptr::null_mut() };
            let dy = y - touch.start.1;
            if !touch.dragging && (dy.abs() > slop || (x - touch.start.0).abs() > slop) {
                touch.dragging = true;
            }
            if let (true, Some((id, from))) = (touch.dragging, touch.scroller) {
                let max = app.view.content.get(&id).copied().unwrap_or(0.0);
                app.view.scroll.insert(id, (from - dy).clamp(0.0, max));
                app.draw();
            }
            None
        }
        1 => {
            let touch = app.touch.take();
            let command = if touch.is_some_and(|t| !t.dragging) { app.tap(x, y) } else { None };
            app.draw();
            command
        }
        _ => {
            app.touch = None;
            None
        }
    };
    answer(&mut env, command)
}

/// A long press: in a field, selects the word under it, for the bridge's Cut, Copy and Paste menu.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeLongPress(mut env: JNIEnv, _: JClass, handle: jlong, x: jfloat, y: jfloat) -> jstring {
    let mut app = app(handle).borrow_mut();
    app.touch = None;
    let Some(id) = app.view.target(x, y).filter(|id| app.view.tree.nodes[id].kind == Kind::Input) else { return std::ptr::null_mut() };
    let byte = app.view.input_byte(id, x);
    let focus = app.set_focus(Some(id));
    let edit = app.view.edits.get_mut(&id).expect("focused");
    if edit.secure { edit.set_caret(byte) } else { edit.select_word(byte) }
    let state = app.state_command(id);
    app.draw();
    answer(&mut env, Some(merge(Some(merge(focus, state)), json!({ "menu": true }))))
}

/// The keyboard changed a field (Flutter's `updateEditingState`), in UTF-16 offsets.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeEditingState(
    mut env: JNIEnv,
    _: JClass,
    handle: jlong,
    id: jint,
    text: JString,
    base: jint,
    extent: jint,
    composing_start: jint,
    composing_end: jint,
) {
    let text = string(&mut env, &text);
    let mut app = app(handle).borrow_mut();
    let id = id as Id;
    let secure = app.view.edits.get(&id).is_some_and(|e| e.secure);
    let edit = Edit::from_utf16(text, base, extent, composing_start, composing_end, secure);
    log::info!("editing state: selection {base}..{extent}, composing {composing_start}..{composing_end}, {} chars", edit.text.chars().count());
    let changed = app.view.edits.get(&id).is_none_or(|old| old.text != edit.text);
    let value = edit.text.clone();
    app.view.edits.insert(id, edit);
    if changed {
        app.event(id, "input", json!({ "value": value }));
    }
    app.draw();
}

/// The keyboard's action key (Flutter's `performEditorAction`): next, go, done, search, send.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeAction(mut env: JNIEnv, _: JClass, handle: jlong, id: jint, action: JString) -> jstring {
    let action = string(&mut env, &action);
    let mut app = app(handle).borrow_mut();
    let command = app.action(id as Id, &action);
    app.draw();
    answer(&mut env, command)
}

/// Every input, for Android's autofill (Flutter's `onProvideAutofillVirtualStructure`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeFields(mut env: JNIEnv, _: JClass, handle: jlong) -> jstring {
    let app = app(handle).borrow();
    let fields: Vec<Value> = app
        .view
        .inputs()
        .into_iter()
        .map(|id| {
            let node = &app.view.tree.nodes[&id];
            let bounds = app.view.bounds(id).unwrap_or_default();
            json!({
                "id": id,
                "type": node.str("type").unwrap_or("text"),
                "autocomplete": node.str("autocomplete"),
                "label": node.str("aria-label").or(node.str("placeholder")),
                "text": app.view.edits.get(&id).map(|e| e.text.clone()).unwrap_or_default(),
                "bounds": [bounds.origin[0], bounds.origin[1], bounds.size[0], bounds.size[1]],
            })
        })
        .collect();
    answer(&mut env, Some(Value::Array(fields)))
}

/// A password manager filled a field (Flutter's `autofill`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeAutofill(mut env: JNIEnv, _: JClass, handle: jlong, id: jint, value: JString) {
    let value = string(&mut env, &value);
    let mut app = app(handle).borrow_mut();
    let id = id as Id;
    let secure = app.view.edits.get(&id).map(|e| e.secure).unwrap_or_else(|| app.view.tree.nodes.get(&id).is_some_and(|n| n.str("type") == Some("password")));
    let edit = Edit { base: value.len(), extent: value.len(), text: value.clone(), composing: None, secure };
    app.view.edits.insert(id, edit);
    app.event(id, "input", json!({ "value": value }));
    app.draw();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeAccessibility(env: JNIEnv, _: JClass, handle: jlong, view: JObject) {
    let mut env = env;
    let adapter = a11y::A11y::new(&mut env, &view, handle);
    app(handle).borrow_mut().a11y = Some(adapter);
}

/// What the screen reader asked for, on the UI thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_strictspike_StrictView_nativeAccessibilityDrain(_: JNIEnv, _: JClass, handle: jlong) {
    let mut app = app(handle).borrow_mut();
    for request in a11y::take(handle) {
        if let Some(id) = a11y::target(&request) {
            match request.action {
                accesskit::Action::Click => {
                    let bounds = app.view.bounds(id).unwrap_or_default();
                    let _ = app.tap(bounds.origin[0] + 1.0, bounds.origin[1] + bounds.size[1] / 2.0);
                }
                accesskit::Action::Focus => {
                    let _ = app.set_focus(Some(id));
                }
                _ => {}
            }
        }
    }
    app.draw();
}
