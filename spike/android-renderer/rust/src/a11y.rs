//! The screen's semantic tree (`Tree::semantics`, shared with the desktop host) as AccessKit's, given to Android's
//! screen reader by AccessKit's Android adapter, which injects itself into the view as its accessibility delegate.
//! The adapter calls in on the UI thread while it holds its own lock, so what it asks is queued and done after
//! (the view's `requestAccessibilityDrain` posts that).

use std::collections::HashMap;
use std::sync::Mutex;

use accesskit::{Action, ActionRequest, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};
use accesskit_android::InjectingAdapter;
use jni::objects::{GlobalRef, JObject};
use jni::{JNIEnv, JavaVM};
use serde_json::Value;

use crate::tree::Id;
use crate::view::View;

static QUEUE: Mutex<Vec<(i64, ActionRequest)>> = Mutex::new(Vec::new());
static ACTIVE: Mutex<Vec<i64>> = Mutex::new(Vec::new());

/// The node every other is under: the window's.
const WINDOW: NodeId = NodeId(u64::MAX);

pub struct A11y {
    adapter: InjectingAdapter,
}

struct Drain {
    vm: JavaVM,
    view: GlobalRef,
    handle: i64,
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
        ACTIVE.lock().unwrap().push(self.handle);
        self.request();
        None
    }
}

impl accesskit::ActionHandler for Drain {
    fn do_action(&mut self, request: ActionRequest) {
        QUEUE.lock().unwrap().push((self.handle, request));
        self.request();
    }
}

impl A11y {
    pub fn new(env: &mut JNIEnv, view: &JObject, handle: i64) -> A11y {
        let drain = || Drain { vm: env.get_java_vm().unwrap(), view: env.new_global_ref(view).unwrap(), handle };
        let (activation, action) = (drain(), drain());
        A11y { adapter: InjectingAdapter::new(env, view, activation, action) }
    }

    pub fn update(&mut self, view: &View) {
        self.adapter.update_if_active(|| tree(view));
    }
}

pub fn take(handle: i64) -> Vec<ActionRequest> {
    let mut queue = QUEUE.lock().unwrap();
    let (mine, others) = std::mem::take(&mut *queue).into_iter().partition(|(h, _)| *h == handle);
    *queue = others;
    mine.into_iter().map(|(_, r)| r).collect()
}

pub fn target(request: &ActionRequest) -> Option<Id> {
    (request.target_node != WINDOW).then_some(request.target_node.0)
}

fn tree(view: &View) -> TreeUpdate {
    let mut nodes = Vec::new();
    let bounds: HashMap<Id, Rect> = view
        .hits
        .iter()
        .map(|hit| (hit.id, Rect { x0: hit.bounds.origin[0] as f64, y0: hit.bounds.origin[1] as f64, x1: (hit.bounds.origin[0] + hit.bounds.size[0]) as f64, y1: (hit.bounds.origin[1] + hit.bounds.size[1]) as f64 }))
        .collect();
    let top: Vec<NodeId> = view.tree.semantics(0).iter().filter_map(|n| add(n, view, &bounds, &mut nodes)).collect();
    let mut window = Node::new(Role::Window);
    window.set_children(top);
    nodes.push((WINDOW, window));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: view.focus.map(NodeId).unwrap_or(WINDOW),
    }
}

fn add(semantic: &Value, view: &View, bounds: &HashMap<Id, Rect>, out: &mut Vec<(NodeId, Node)>) -> Option<NodeId> {
    let id = semantic.get("id")?.as_u64()?;
    let secure = view.tree.nodes.get(&id).is_some_and(|n| n.str("type") == Some("password"));
    let role = match semantic.get("role").and_then(Value::as_str).unwrap_or("") {
        "heading" => Role::Heading,
        "list" => Role::List,
        "listitem" => Role::ListItem,
        "button" => Role::Button,
        "searchbox" => Role::SearchInput,
        "textbox" if secure => Role::PasswordInput,
        "textbox" => Role::TextInput,
        "img" => Role::Image,
        "text" => Role::Label,
        "status" => Role::Status,
        "main" => Role::Main,
        _ => Role::GenericContainer,
    };
    let mut node = Node::new(role);
    if let Some(name) = semantic.get("name").and_then(Value::as_str) {
        node.set_label(name);
    }
    if let Some(placeholder) = semantic.get("placeholder").and_then(Value::as_str) {
        node.set_placeholder(placeholder);
    }
    if let Some(edit) = view.edits.get(&id) {
        node.set_value(edit.display());
    }
    if let Some(rect) = bounds.get(&id) {
        node.set_bounds(*rect);
    }
    if let Some(actions) = semantic.get("actions").and_then(Value::as_array) {
        for action in actions.iter().filter_map(Value::as_str) {
            match action {
                "click" => node.add_action(Action::Click),
                "focus" => node.add_action(Action::Focus),
                _ => {}
            }
        }
    }
    if matches!(role, Role::TextInput | Role::PasswordInput | Role::SearchInput) {
        node.add_action(Action::Focus);
        node.add_action(Action::Click);
    }
    let children: Vec<NodeId> = semantic.get("children").and_then(Value::as_array).into_iter().flatten().filter_map(|c| add(c, view, bounds, out)).collect();
    node.set_children(children);
    out.push((NodeId(id), node));
    Some(NodeId(id))
}
