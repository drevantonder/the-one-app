//! A native screen's semantic tree (`Tree::semantics`, which the inspector reads too) as AccessKit's, given to
//! TalkBack by AccessKit's Android adapter, which makes itself the view's accessibility delegate: headings by their
//! level, what's clickable as a button or by its role, text by what it says, fields by their name and value. The
//! adapter calls in while it holds its own lock, so what TalkBack asks is queued and done after, on the UI thread
//! (`NativeView.requestAccessibilityDrain`).

use std::sync::Mutex;

use accesskit::{Action, ActionRequest, Live, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use accesskit_android::InjectingAdapter;
use jni::objects::{GlobalRef, JObject};
use jni::{JNIEnv, JavaVM};
use serde_json::Value;

use super::WindowId;
use crate::Screen;
use crate::tree::Id;

static QUEUE: Mutex<Vec<(WindowId, ActionRequest)>> = Mutex::new(Vec::new());

/// The node every other is under: the window's.
const WINDOW: NodeId = NodeId(u64::MAX);

pub struct A11y {
    adapter: InjectingAdapter,
    density: f64,
}

/// Asks the view to drain the queue on the UI thread.
struct Drain {
    vm: JavaVM,
    view: GlobalRef,
    window: WindowId,
}

impl Drain {
    fn request(&self) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            let _ = env.call_method(&self.view, "requestAccessibilityDrain", "()V", &[]);
        }
    }
}

impl accesskit::ActivationHandler for Drain {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // The tree is given as the screen next draws, which the drain makes it do.
        self.request();
        None
    }
}

impl accesskit::ActionHandler for Drain {
    fn do_action(&mut self, request: ActionRequest) {
        QUEUE.lock().expect("nothing panics holding it").push((self.window, request));
        self.request();
    }
}

impl A11y {
    pub fn new(env: &mut JNIEnv, view: &JObject, window: WindowId) -> A11y {
        let drain = |env: &mut JNIEnv| Drain { vm: env.get_java_vm().expect("a VM"), view: env.new_global_ref(view).expect("a view"), window };
        let (activation, action) = (drain(env), drain(env));
        let density = env
            .call_method(view, "getResources", "()Landroid/content/res/Resources;", &[])
            .and_then(|resources| env.call_method(resources.l()?, "getDisplayMetrics", "()Landroid/util/DisplayMetrics;", &[]))
            .and_then(|metrics| env.get_field(metrics.l()?, "density", "F"))
            .and_then(|density| density.f())
            .unwrap_or(1.0);
        A11y { adapter: InjectingAdapter::new(env, view, activation, action), density: f64::from(density) }
    }

    /// Gives TalkBack the screen as it is, if it's listening.
    pub fn update(&mut self, screen: &Screen) {
        let density = self.density;
        self.adapter.update_if_active(|| tree(screen, density));
    }
}

/// What TalkBack asked of window `window` since it was last asked.
pub fn take(window: WindowId) -> Vec<ActionRequest> {
    let mut queue = QUEUE.lock().expect("nothing panics holding it");
    let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *queue).into_iter().partition(|(at, _)| *at == window);
    *queue = others;
    mine.into_iter().map(|(_, request)| request).collect()
}

pub fn target(request: &ActionRequest) -> Option<Id> {
    (request.target_node != WINDOW).then_some(request.target_node.0)
}

fn tree(screen: &Screen, density: f64) -> TreeUpdate {
    let mut nodes = Vec::new();
    let top: Vec<NodeId> = screen.tree().semantics(0).iter().filter_map(|semantic| add(semantic, screen, density, &mut nodes)).collect();
    let mut window = Node::new(Role::Window);
    window.set_children(top);
    nodes.push((WINDOW, window));
    let focus = screen.focused().filter(|focused| nodes.iter().any(|(id, _)| id.0 == *focused)).map_or(WINDOW, NodeId);
    TreeUpdate { nodes, tree: Some(TreeInfo::new(WINDOW)), tree_id: TreeId::ROOT, focus }
}

/// ARIA's role as AccessKit's, as Chromium maps them.
fn role(role: &str, secure: bool) -> Role {
    match role {
        "heading" => Role::Heading,
        "button" => Role::Button,
        "link" => Role::Link,
        "list" => Role::List,
        "listitem" => Role::ListItem,
        "listbox" => Role::ListBox,
        "option" => Role::ListBoxOption,
        "searchbox" => Role::SearchInput,
        "textbox" if secure => Role::PasswordInput,
        "textbox" => Role::TextInput,
        "img" => Role::Image,
        "text" => Role::Label,
        "status" => Role::Status,
        "alert" => Role::Alert,
        "alertdialog" => Role::AlertDialog,
        "dialog" => Role::Dialog,
        "main" => Role::Main,
        "navigation" => Role::Navigation,
        "banner" => Role::Banner,
        "region" => Role::Region,
        "group" => Role::Group,
        "checkbox" => Role::CheckBox,
        "switch" => Role::Switch,
        "radio" => Role::RadioButton,
        "radiogroup" => Role::RadioGroup,
        "menu" => Role::Menu,
        "menuitem" => Role::MenuItem,
        "tab" => Role::Tab,
        "tablist" => Role::TabList,
        "tabpanel" => Role::TabPanel,
        "toolbar" => Role::Toolbar,
        "progressbar" => Role::ProgressIndicator,
        "slider" => Role::Slider,
        "separator" => Role::Splitter,
        "combobox" => Role::ComboBox,
        "form" => Role::Form,
        "tooltip" => Role::Tooltip,
        _ => Role::GenericContainer,
    }
}

fn add(semantic: &Value, screen: &Screen, density: f64, out: &mut Vec<(NodeId, Node)>) -> Option<NodeId> {
    let id = semantic.get("id")?.as_u64()?;
    let node = screen.tree().nodes.get(&id);
    let secure = node.is_some_and(|node| node.str("type") == Some("password"));
    let clickable = semantic["actions"].as_array().is_some_and(|actions| actions.iter().any(|action| action == "click"));
    let said = semantic["role"].as_str().unwrap_or_default();
    // What's clickable and has no role of its own is a button, as Chromium names a div with a click handler.
    let mut role = role(said, secure);
    if role == Role::GenericContainer && clickable {
        role = Role::Button;
    }
    let mut out_node = Node::new(role);
    // A live region (a toast, an announcement) is read as its words change, without the screen reader's focus
    // moving: AccessKit's adapter says so as its name changes, so it's named by all it says.
    let live = match semantic["live"].as_str() {
        Some("assertive") => Some(Live::Assertive),
        Some("polite") => Some(Live::Polite),
        _ => None,
    };
    if let Some(live) = live {
        out_node.set_live(live);
    }
    // A control is one element, read whole by the words in it, as Android's own controls are; so is a live region.
    let whole = matches!(role, Role::Button | Role::Link | Role::ListBoxOption | Role::CheckBox | Role::Switch | Role::RadioButton | Role::Tab | Role::MenuItem);
    let name = semantic["name"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| (whole || live.is_some()).then(|| words(semantic)).filter(|words| !words.is_empty()));
    // Words are a label's value, as AccessKit has them (`label_comes_from_value`); anything else is named by its label.
    match name {
        Some(name) if role == Role::Label => out_node.set_value(name),
        Some(name) => out_node.set_label(name),
        None => {}
    }
    if let Some(level) = semantic["level"].as_u64().or_else(|| semantic["level"].as_str().and_then(|level| level.parse().ok())) {
        out_node.set_level(level as usize);
    }
    if let Some(description) = semantic["description"].as_str() {
        out_node.set_description(description);
    }
    if let Some(placeholder) = node.and_then(|node| node.str("placeholder")) {
        out_node.set_placeholder(placeholder);
    }
    if let Some(text) = screen.field(id).and_then(|field| field["text"].as_str().map(str::to_owned)) {
        out_node.set_value(if secure { "•".repeat(text.chars().count()) } else { text });
    }
    if let Some(checked) = node.and_then(|node| node.str("aria-checked")) {
        out_node.set_toggled(match checked {
            "true" => Toggled::True,
            "mixed" => Toggled::Mixed,
            _ => Toggled::False,
        });
    }
    if node.is_some_and(|node| node.is("disabled") || node.str("aria-disabled") == Some("true")) {
        out_node.set_disabled();
    }
    if let Some(rect) = screen.rect(id) {
        let (x, y, width, height) = (f64::from(rect.x) * density, f64::from(rect.y) * density, f64::from(rect.width) * density, f64::from(rect.height) * density);
        out_node.set_bounds(Rect { x0: x, y0: y, x1: x + width, y1: y + height });
    }
    if clickable {
        out_node.add_action(Action::Click);
    }
    if matches!(role, Role::TextInput | Role::PasswordInput | Role::SearchInput) {
        out_node.add_action(Action::Focus);
        out_node.add_action(Action::Click);
    }
    // A control is read by its name, so what's in it isn't read again.
    let children: Vec<NodeId> = if whole {
        Vec::new()
    } else {
        semantic["children"].as_array().into_iter().flatten().filter_map(|child| add(child, screen, density, out)).collect()
    };
    out_node.set_children(children);
    out.push((NodeId(id), out_node));
    Some(NodeId(id))
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
