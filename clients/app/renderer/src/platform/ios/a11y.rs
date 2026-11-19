//! A native screen's semantic tree (`Tree::semantics`, which the inspector and Android's TalkBack read too) as
//! Flutter's semantics nodes, given to VoiceOver by Flutter's iOS semantics objects (`accessibility.mm`): headings by
//! their level, what's clickable as a button, text by what it says, fields by their name and value, each where it
//! was drawn. What VoiceOver asks of a node comes back as Flutter's semantics action, queued and done once the windows
//! aren't in use.

use std::cell::RefCell;
use std::ffi::{CString, c_void};

use objc2_ui_kit::UIView;
use serde_json::{Value, json};

use super::{WindowId, wake};
use crate::Screen;
use crate::tree::Id;

/// Flutter's semantics actions (`SemanticsAction`), those a screen here takes.
const TAP: i32 = 1 << 0;
const SCROLL_UP: i32 = 1 << 4;
const SCROLL_DOWN: i32 = 1 << 5;
const SHOW_ON_SCREEN: i32 = 1 << 8;
const DID_GAIN_ACCESSIBILITY_FOCUS: i32 = 1 << 15;
/// VoiceOver's two-finger scrub (`accessibilityPerformEscape`), which goes back.
pub const DISMISS: i32 = 1 << 18;
const FOCUS: i32 = 1 << 22;

/// Flutter's tristate and check state (`SemanticsTristate`, `SemanticsCheckState`).
const TRUE: i32 = 1;
const FALSE: i32 = 2;
const MIXED: i32 = 3;

thread_local! {
    /// What VoiceOver asked, of which node of which window, since the windows were last used.
    static ASKED: RefCell<Vec<(WindowId, i32, i32)>> = const { RefCell::new(Vec::new()) };
}

/// A window's bridge to VoiceOver (`accessibility.mm`), and the tree it was last given, as it was given.
pub struct A11y {
    bridge: *mut c_void,
    given: RefCell<String>,
}

impl A11y {
    pub fn new(view: &UIView, text_input: *mut c_void, window: WindowId) -> A11y {
        let bridge = unsafe { accessibility_new(view, text_input, window as *mut c_void, asked) };
        A11y { bridge, given: RefCell::new(String::new()) }
    }

    /// Gives VoiceOver the screen as it is, if it's listening and the screen changed. The window is Flutter's root
    /// (0), `size` big, in device pixels, as every node's rect is; each node is its own id one up.
    pub fn update(&self, screen: &Screen, size: (f32, f32), scale: f32) {
        if !listening() {
            self.given.borrow_mut().clear();
            return;
        }
        let mut nodes = Vec::new();
        let top: Vec<i32> = screen.tree().semantics(0).iter().filter_map(|semantic| add(semantic, screen, scale, &mut nodes)).collect();
        nodes.insert(0, json!({"id": 0, "rect": [0.0, 0.0, size.0, size.1], "children": top}));
        let tree = Value::Array(nodes).to_string();
        if *self.given.borrow() == tree {
            return;
        }
        let json = CString::new(tree.replace('\0', "")).expect("no NUL in it");
        unsafe { accessibility_update(self.bridge, json.as_ptr()) };
        *self.given.borrow_mut() = tree;
    }
}

impl Drop for A11y {
    fn drop(&mut self) {
        unsafe { accessibility_free(self.bridge) };
    }
}

/// Whether a screen reader is listening: VoiceOver or Switch Control, as Flutter's view controller asks, and always in
/// the simulator, where whether the Accessibility Inspector is can't be known, as Flutter's has it.
fn listening() -> bool {
    cfg!(target_abi = "sim") || unsafe { accessibility_running() }
}

/// What VoiceOver asked of window `window`, as the node (its tree's id) and the semantics action.
pub fn take(window: WindowId) -> Vec<(Id, i32)> {
    ASKED.with(|asked| {
        let mut asked = asked.borrow_mut();
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *asked).into_iter().partition(|(at, _, _)| *at == window);
        *asked = others;
        mine.into_iter().filter(|(_, node, _)| *node > 0).map(|(_, node, action)| ((node - 1) as Id, action)).collect()
    })
}

/// Does what VoiceOver asked of node `id` on `screen`.
pub fn act(screen: &mut Screen, id: Id, action: i32) {
    match action {
        TAP => drop(screen.press(id)),
        FOCUS => screen.set_focused(Some(id)),
        // Swiping to a node shows it, scrolling what it's in, as Chrome does for VoiceOver.
        SHOW_ON_SCREEN | DID_GAIN_ACCESSIBILITY_FOCUS => drop(screen.reveal(id)),
        SCROLL_UP | SCROLL_DOWN => {
            if let Some(rect) = screen.rect(id) {
                let by = rect.height * 0.8 * if action == SCROLL_DOWN { 1.0 } else { -1.0 };
                screen.pan(crate::geometry::point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0), crate::geometry::point(0.0, by));
            }
        }
        _ => {}
    }
}

/// Node `semantic` and what's in it, as Flutter's semantics nodes, into `out`: → its id there.
fn add(semantic: &Value, screen: &Screen, scale: f32, out: &mut Vec<Value>) -> Option<i32> {
    let id = semantic.get("id")?.as_u64()?;
    let node = screen.tree().nodes.get(&id);
    let secure = node.is_some_and(|node| node.str("type") == Some("password"));
    let clickable = semantic["actions"].as_array().is_some_and(|actions| actions.iter().any(|action| action == "click"));
    let role = semantic["role"].as_str().unwrap_or_default();
    let field = matches!(role, "textbox" | "searchbox");
    let states = &semantic["states"];
    // What's clickable and has no role of its own is a button, as Chromium names a div with a click handler; a tab
    // and a menu's item are read as buttons too, as UIKit's are.
    let button = matches!(role, "button" | "tab" | "menuitem") || (role.is_empty() && clickable);
    // A control is one element, read whole by the words in it, as UIKit's controls are; so is a live region.
    let whole = button || matches!(role, "link" | "option" | "checkbox" | "switch" | "radio");
    let live = semantic["live"].as_str().is_some();
    let label = semantic["name"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| (whole || live).then(|| words(semantic)))
        .unwrap_or_default();
    let value = screen
        .field(id)
        .and_then(|field| field["text"].as_str().map(str::to_owned))
        .map(|text| if secure { "•".repeat(text.chars().count()) } else { text })
        .or_else(|| semantic["value"].as_str().map(str::to_owned).or_else(|| semantic["value"].as_number().map(ToString::to_string)))
        .unwrap_or_default();
    let hint = semantic["description"].as_str().or_else(|| semantic["placeholder"].as_str()).unwrap_or_default();
    let level = semantic["level"].as_u64().or_else(|| semantic["level"].as_str().and_then(|level| level.parse().ok())).unwrap_or(0);
    let checked = match states["checked"].clone() {
        Value::Bool(true) => TRUE,
        Value::Bool(false) => FALSE,
        Value::String(mixed) if mixed == "mixed" => MIXED,
        _ => 0,
    };
    let tristate = |state: &str| match states[state] {
        Value::Bool(true) => TRUE,
        Value::Bool(false) => FALSE,
        _ => 0,
    };
    let mut actions = SHOW_ON_SCREEN | DID_GAIN_ACCESSIBILITY_FOCUS;
    if clickable || field {
        actions |= TAP;
    }
    if field {
        actions |= FOCUS;
    }
    if screen.goes_back() {
        actions |= DISMISS;
    }
    let rect = screen.rect(id).map(|rect| [rect.x * scale, rect.y * scale, rect.width * scale, rect.height * scale]);
    // A control is read by its label, so what's in it isn't read again.
    let children: Vec<i32> = if whole {
        Vec::new()
    } else {
        semantic["children"].as_array().into_iter().flatten().filter_map(|child| add(child, screen, scale, out)).collect()
    };
    let flutter_id = id as i32 + 1;
    out.push(json!({
        "id": flutter_id,
        "label": label,
        "value": value,
        "hint": hint,
        "heading": if role == "heading" { level.max(1) } else { 0 },
        "actions": actions,
        "rect": rect,
        "children": children,
        "flags": {
            "button": button,
            "header": role == "heading",
            "link": role == "link",
            "image": role == "img",
            "textField": field,
            "obscured": secure,
            "multiline": node.is_some_and(|node| node.is("multiline")),
            "liveRegion": live,
            "mutuallyExclusive": role == "radio",
            "checked": if matches!(role, "checkbox" | "radio") { checked } else { 0 },
            "toggled": if role == "switch" { checked } else { 0 },
            "selected": tristate("selected"),
            "enabled": if states["disabled"] == json!(true) { FALSE } else { 0 },
            "focused": if field { if screen.focused() == Some(id) { TRUE } else { FALSE } } else { 0 },
        },
    }));
    Some(flutter_id)
}

/// What a node and what's in it say, in order.
fn words(semantic: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    if let Some(name) = semantic["name"].as_str() {
        out.push(name.to_owned());
    }
    for child in semantic["children"].as_array().into_iter().flatten() {
        let child = words(child);
        if !child.is_empty() {
            out.push(child);
        }
    }
    out.join(", ")
}

/// What VoiceOver asks of window `context`'s node `node`, for its screen once the windows aren't in use.
extern "C" fn asked(context: *mut c_void, node: i32, action: i32) {
    ASKED.with(|asked| asked.borrow_mut().push((context as WindowId, node, action)));
    wake();
}

unsafe extern "C" {
    fn accessibility_new(view: &UIView, text_input: *mut c_void, context: *mut c_void, send: extern "C" fn(*mut c_void, i32, i32)) -> *mut c_void;
    fn accessibility_free(bridge: *mut c_void);
    fn accessibility_update(bridge: *mut c_void, json: *const std::ffi::c_char);
    fn accessibility_running() -> bool;
}
