//! Another app's page drawn in a box of this one's (`tree::Kind::Frame`), as an iframe is: its own screen, its JS on
//! its own thread with its own app's permissions, which its host makes (`Screen::make_frame`), drawn in this screen's layout
//! and scene, with the pointer, the keyboard and Tab passing into it and out of it as for any box.

use serde_json::Value;

use super::Screen;
use crate::editing::{Clipboard, Ime};
use crate::geometry::{Bounds, Point, point};
use crate::inspect::Key;
use crate::render::scene::Scene;
use crate::tree::{Id, Kind};

/// What makes the screen of the page a frame shows, for whoever it's for: its host, which hands it over once it's
/// started (`Screen::framed`), or never, where it can't.
pub type Make = Box<dyn FnMut(Id, &str, Option<&str>)>;

/// A frame's page: its screen, the scene it last drew, where it's drawn in its parent's window, and what it shows.
pub(crate) struct Child {
    pub screen: Screen,
    pub scene: Scene,
    pub rect: Bounds<f32>,
    pub src: String,
    /// How tall its page said its content is, as its frame was last laid out.
    pub height: Option<f32>,
}

/// The clipboard a screen shares with the pages in its frames.
#[derive(Clone)]
pub(crate) struct Shared(std::rc::Rc<std::cell::RefCell<Box<dyn Clipboard>>>);

impl Clipboard for Shared {
    fn read(&mut self, primary: bool) -> Option<String> {
        self.0.borrow_mut().read(primary)
    }

    fn write(&mut self, primary: bool, text: &str) {
        self.0.borrow_mut().write(primary, text)
    }
}

impl Screen {
    /// The page frame `id` shows, which its host made, as `src` (`<app>/<page>[#<fragment>]`) said: it's drawn in that
    /// box from now on, unless the frame went or shows another page since it was asked for.
    pub fn framed(&mut self, id: Id, src: &str, mut screen: Screen) {
        if !self.asking.remove(&(id, src.to_owned())) {
            return;
        }
        screen.nested = true;
        screen.insets = Default::default();
        // A page in a frame is on its parent's device: a finger or not, and the text size its person has, which its type's sizes follow.
        (screen.touch, screen.text_size) = (self.touch, self.text_size);
        let shared = Shared(std::rc::Rc::new(std::cell::RefCell::new(std::mem::replace(&mut self.clipboard, Box::new(crate::editing::Memory::default())))));
        self.clipboard = Box::new(shared.clone());
        screen.clipboard = Box::new(shared);
        if let Some(wake) = &self.wake {
            screen.on_post(wake.clone());
        }
        self.frames.insert(id, Child { screen, scene: Scene::default(), rect: Bounds::default(), src: src.to_owned(), height: None });
        self.again = true;
    }

    /// Frame `id`'s page starts again, as its app's page was updated.
    pub fn reload_frame(&mut self, id: Id) {
        self.frames.remove(&id);
        self.sync_frames();
    }

    /// The screen of frame `id`'s page, for what its host tells it.
    pub fn frame_mut(&mut self, id: Id) -> Option<&mut Screen> {
        self.frames.get_mut(&id).map(|child| &mut child.screen)
    }

    /// Asks for each frame's page that isn't here yet, and lets go of each whose frame went or shows another.
    pub(super) fn sync_frames(&mut self) {
        let wanted: Vec<(Id, String, Option<String>)> = self
            .tree
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == Kind::Frame)
            .filter_map(|(id, node)| Some((*id, node.str("src")?.to_owned(), node.str("subject").map(str::to_owned))))
            .collect();
        let shows = |id: &Id, src: &str| wanted.iter().any(|(wanted, wants, _)| wanted == id && wants == src);
        self.frames.retain(|id, child| shows(id, &child.src));
        self.asking.retain(|(id, src)| shows(id, src));
        for (id, src, subject) in wanted {
            if self.frames.contains_key(&id) || !self.asking.insert((id, src.clone())) {
                continue;
            }
            if let Some(make) = self.make_frame.as_mut() {
                make(id, &src, subject.as_deref());
            }
        }
    }

    /// The frame that takes the pointer at `at`, and where `at` is in its page.
    pub(super) fn frame_at(&self, at: Point<f32>) -> Option<(Id, Point<f32>)> {
        let top = *self.under(at).first()?;
        let child = self.frames.get(&top)?;
        Some((top, point(at.x - child.rect.origin.x, at.y - child.rect.origin.y)))
    }

    /// Where `at` is in frame `id`'s page.
    pub(super) fn local(&self, id: Id, at: Point<f32>) -> Point<f32> {
        let origin = self.frames.get(&id).map_or(point(0.0, 0.0), |child| child.rect.origin);
        point(at.x - origin.x, at.y - origin.y)
    }

    /// The pointer moved to `at`: the page of the frame it's over, or holding a press in, is told, and the one it left.
    /// → whether a frame's page took it.
    pub(super) fn frame_moved(&mut self, at: Point<f32>) -> bool {
        let inside = self.captured_frame.map(|id| (id, self.local(id, at))).or_else(|| self.frame_at(at));
        let was = std::mem::replace(&mut self.pointing, inside.map(|(id, _)| id));
        if let Some(left) = was.filter(|was| Some(*was) != self.pointing)
            && let Some(child) = self.frames.get_mut(&left)
        {
            child.screen.pointer_left();
        }
        let Some((id, local)) = inside else { return false };
        if let Some(child) = self.frames.get_mut(&id) {
            child.screen.pointer_moved(local);
        }
        true
    }

    /// The pointer left the window: the page of the frame it was over too.
    pub(super) fn frame_left(&mut self) {
        if let Some(child) = self.pointing.take().and_then(|id| self.frames.get_mut(&id)) {
            child.screen.pointer_left();
        }
    }

    /// A button went down at `at`: in a frame, its page has it, and the frame the keyboard. → whether it was in one.
    pub(super) fn frame_down(&mut self, at: Point<f32>, button: u8, shift: bool) -> bool {
        let Some((id, local)) = self.frame_at(at) else { return false };
        self.states.focus_visible = false;
        self.focus(Some(id));
        self.captured_frame = Some(id);
        if let Some(child) = self.frames.get_mut(&id) {
            child.screen.pointer_down(local, button, shift);
        }
        true
    }

    /// A button came up at `at`: the page of the frame it went down in has it. → whether it went down in one.
    pub(super) fn frame_up(&mut self, at: Point<f32>, button: u8) -> bool {
        let Some(id) = self.captured_frame.take() else { return false };
        let local = self.local(id, at);
        if let Some(child) = self.frames.get_mut(&id) {
            child.screen.pointer_up(local, button);
        }
        true
    }

    /// Scrolled at `at`, by a wheel or a trackpad (`wheel`, with its phase where the system says) or a finger: the
    /// page of the frame there scrolls, if it still can that way. → what scrolled, if it did.
    pub(super) fn frame_scroll(&mut self, at: Point<f32>, delta: Point<f32>, wheel: Option<(Option<super::Phase>, Option<Point<f32>>)>) -> Option<Id> {
        let (id, local) = self.frame_at(at)?;
        let child = self.frames.get_mut(&id)?;
        match wheel {
            Some((phase, notches)) => child.screen.scroll(local, delta, phase, notches),
            None => child.screen.pan(local, delta),
        }
        .map(|_| id)
    }

    /// The pointer's shape at `at`, where it's over a frame: its page's.
    pub(super) fn frame_cursor(&self, at: Point<f32>) -> Option<super::Cursor> {
        let (id, local) = self.frame_at(at)?;
        Some(self.frames.get(&id)?.screen.cursor(local))
    }

    /// The page of the frame with the keyboard, if one has it.
    pub(super) fn focused_frame(&mut self) -> Option<&mut Screen> {
        let id = self.focus.focused?;
        self.frames.get_mut(&id).map(|child| &mut child.screen)
    }

    /// A key, where a frame has the keyboard: its page has it. → whether it did, and if it was Tab that took the
    /// keyboard out of its page, which way (`true` back), for this screen to move it on from the frame.
    pub(super) fn frame_key(&mut self, key: &Key) -> Option<Option<bool>> {
        let child = self.focused_frame()?;
        child.key(key);
        Some(child.left.take())
    }

    /// Tab came to frame `id`: the keyboard goes into its page, to its first stop, or with Shift its last. → whether
    /// its page has one.
    pub(super) fn enter_frame(&mut self, id: Id, back: bool) -> bool {
        let Some(child) = self.frames.get_mut(&id) else { return false };
        child.screen.enter(back)
    }

    /// The keyboard comes into this page by Tab, as into a frame: to its first stop, or its last. → whether it has one.
    pub(super) fn enter(&mut self, back: bool) -> bool {
        let next = if back { self.focus.tab_order.last() } else { self.focus.tab_order.first() }.copied();
        let Some(next) = next else { return false };
        self.states.focus_visible = true;
        self.focus(Some(next));
        self.reveal(next);
        if self.editing.fields.contains_key(&next) {
            self.edit(next, crate::editing::Field::select_all);
        }
        true
    }

    /// What the input method says, where a frame has the keyboard: its page's. → whether one had it.
    pub(super) fn frame_ime(&mut self, ime: Ime) -> bool {
        match self.focused_frame() {
            Some(child) => {
                child.ime(ime);
                true
            }
            None => false,
        }
    }

    /// Where the frame with the keyboard has its caret, in the window.
    pub(super) fn frame_caret(&self) -> Option<Option<Bounds<f32>>> {
        let child = self.frames.get(&self.focus.focused?)?;
        Some(child.screen.caret_rect().map(|caret| Bounds { origin: point(caret.origin.x + child.rect.origin.x, caret.origin.y + child.rect.origin.y), size: caret.size }))
    }

    /// The semantic tree with each frame's page's in its place.
    pub(super) fn grafted(&self, read: Vec<Value>) -> Vec<Value> {
        read.into_iter()
            .flat_map(|mut node| {
                if let Some(id) = node.get("frame").and_then(Value::as_u64) {
                    return self.frames.get(&id).map(|child| child.screen.grafted(child.screen.tree.semantics(0))).unwrap_or_default();
                }
                if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
                    *children = self.grafted(std::mem::take(children));
                }
                vec![node]
            })
            .collect()
    }

    /// The inspector's tree (`inspect::tree`), with what each frame's page has in the frame's place, where it's drawn
    /// in this window.
    pub(super) fn inspected_tree(&self) -> Value {
        fn moved(nodes: &mut [Value], by: Point<f32>) {
            for node in nodes {
                if let Some(rect) = node.get_mut("rect").and_then(Value::as_object_mut) {
                    for (edge, by) in [("x", by.x), ("y", by.y)] {
                        if let Some(at) = rect.get(edge).and_then(Value::as_f64) {
                            rect.insert(edge.into(), serde_json::json!(at + f64::from(by)));
                        }
                    }
                }
                if let Some(Value::Array(children)) = node.get_mut("children") {
                    moved(children, by);
                }
            }
        }
        fn graft(screen: &Screen, nodes: &mut [Value]) {
            for node in nodes {
                if let Some(child) = node.get("frame").and_then(Value::as_u64).and_then(|id| screen.frames.get(&id)) {
                    let mut inside = child.screen.inspected_tree()["tree"].take();
                    if let Value::Array(inside) = &mut inside {
                        moved(inside, child.rect.origin);
                    }
                    let rect = child.rect;
                    node["src"] = Value::String(child.src.clone());
                    node["rect"] = serde_json::json!({"x": rect.origin.x, "y": rect.origin.y, "width": rect.size.width, "height": rect.size.height});
                    node["children"] = inside;
                } else if let Some(Value::Array(children)) = node.get_mut("children") {
                    graft(screen, children);
                }
            }
        }
        let mut tree = crate::inspect::tree(&self.tree, &self.rects);
        if let Some(Value::Array(nodes)) = tree.get_mut("tree") {
            graft(self, nodes);
        }
        tree
    }
}
