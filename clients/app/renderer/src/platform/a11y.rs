//! A native screen's semantic tree (`Tree::semantics`, which the inspector reads too) as AccessKit's, which each
//! phone's AccessKit adapter gives its screen reader (`android::a11y`, `ios::a11y`): headings by their level, what's
//! clickable as a button or by its role, text by what it says, fields by their name and value, each where it was
//! drawn in the window's device pixels. What the screen reader asks comes back as AccessKit's action request (`act`).

use accesskit::{Action, ActionRequest, Live, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use serde_json::Value;

use crate::Screen;
use crate::geometry::point;
use crate::tree::Id;

/// The node every other is under: the window's.
const WINDOW: NodeId = NodeId(u64::MAX);

/// Does what the screen reader asked of `screen`.
pub fn act(screen: &mut Screen, request: &ActionRequest) {
    let Some(node) = target(request) else { return };
    match request.action {
        Action::Click => drop(screen.press(node)),
        Action::Focus => screen.set_focused(Some(node)),
        // Swiping to a node shows it, scrolling what it's in, as Chrome does for a screen reader.
        Action::ScrollIntoView => drop(screen.reveal(node)),
        Action::ShowContextMenu => {
            if let Some(rect) = screen.rect(node) {
                let at = point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
                screen.pointer_down(at, 2, false);
                screen.pointer_up(at, 2);
            }
        }
        Action::ScrollDown | Action::ScrollUp => {
            if let Some(rect) = screen.rect(node) {
                let by = rect.height * 0.8 * if request.action == Action::ScrollDown { 1.0 } else { -1.0 };
                screen.pan(point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0), point(0.0, by));
            }
        }
        _ => {}
    }
}

fn target(request: &ActionRequest) -> Option<Id> {
    (request.target_node != WINDOW).then_some(request.target_node.0)
}

pub fn tree(screen: &Screen, density: f64) -> TreeUpdate {
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
