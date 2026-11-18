//! An app's screen as a GPUI view: its tree drawn as GPUI's elements with the design tokens, and what the person does
//! sent back to its JS as events.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, FocusHandle, ImageFormat, KeyDownEvent, Subscription, Task, Window, WindowAppearance,
    actions, div, img, px, relative,
};
use serde_json::{Value, json};

use crate::js::{Posted, Sandbox};
use crate::style::{colour, layout, text_style};
use crate::tree::{Id, Kind, Node, Style, Tree};

actions!(strict, [FocusNext, FocusPrevious]);

/// The typeface every screen is drawn in, as the UI library's are (core/ui/src/tokens.css), by the family name its
/// file has, which a stylesheet's `@font-face` calls "Allo".
pub const FONT: &str = "Allo Variable";

/// What the app's JS posts its client (`host.post`), for the host to answer: → the answer, where the host has it at
/// once. One it answers later, or a notification, goes to `Screen::message`.
pub type Asked = Box<dyn FnMut(Value, &mut Window, &mut Context<Screen>) -> Option<Value>>;

pub struct Screen {
    tree: Tree,
    sandbox: Sandbox,
    /// Light or dark as Colour Mode says, or as the window is (`None`).
    pub theme: Option<WindowAppearance>,
    asked: Option<Asked>,
    /// What can have the keyboard: inputs and what can be pressed, each with what tells its JS it has it or lost it.
    focus: HashMap<Id, (FocusHandle, [Subscription; 2])>,
    /// An input's text as it's typed, ahead of the app setting it back as its `value`.
    typed: HashMap<Id, String>,
    timer: Option<Task<()>>,
    /// Why it can't draw, if it can't: the app's JS failed, or sent what this host doesn't speak.
    pub failed: Option<String>,
}

impl Screen {
    /// The screen of an app whose JS has started, drawn from what it's posted so far.
    pub fn new(sandbox: Sandbox, asked: Option<Asked>, window: &mut Window, cx: &mut Context<Screen>) -> Screen {
        let mut screen = Screen {
            tree: Tree::default(),
            sandbox,
            theme: None,
            asked,
            focus: HashMap::new(),
            typed: HashMap::new(),
            timer: None,
            failed: None,
        };
        screen.pump(window, cx);
        screen
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    /// The semantic tree, as an agent and a screen reader read it.
    pub fn semantics(&self) -> Value {
        json!(self.tree.semantics(0))
    }

    /// Tells the app what the person did to node `id`, and draws what it changed.
    pub fn event(&mut self, id: Id, name: &str, value: Option<String>, window: &mut Window, cx: &mut Context<Screen>) {
        let event = json!({"v": crate::VERSION, "op": "event", "id": id, "name": name, "value": value});
        if let Err(error) = self.sandbox.send("event", &event) {
            self.failed = Some(format!("{error:#}"));
        }
        self.pump(window, cx);
    }

    /// Tells the app a message from its client: an answer to what it asked (`Asked`), or a notification.
    pub fn message(&mut self, message: &Value, window: &mut Window, cx: &mut Context<Screen>) {
        if let Err(error) = self.sandbox.send("message", message) {
            self.failed = Some(format!("{error:#}"));
        }
        self.pump(window, cx);
    }

    /// Applies what the app has posted, and waits for its next timer.
    fn pump(&mut self, window: &mut Window, cx: &mut Context<Screen>) {
        let mut answers = Vec::new();
        for posted in self.sandbox.take() {
            match posted {
                Posted::Update(update) => {
                    let applied = serde_json::from_str(&update).map_err(anyhow::Error::from).and_then(|update| self.tree.apply(update));
                    if let Err(error) = applied {
                        self.failed = Some(format!("{error:#}"));
                    }
                }
                Posted::Message(message) => {
                    let (Ok(message), Some(asked)) = (serde_json::from_str::<Value>(&message), self.asked.as_mut()) else { continue };
                    answers.extend(asked(message, window, cx));
                }
            }
        }
        if !answers.is_empty() {
            for answer in answers {
                if let Err(error) = self.sandbox.send("message", &answer) {
                    self.failed = Some(format!("{error:#}"));
                }
            }
            return self.pump(window, cx);
        }
        self.timer = self.sandbox.due().map(|due| {
            let due = Duration::from_secs_f64(due.max(0.0) / 1000.0);
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(due).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.sandbox.clock().advance(due.as_secs_f64() * 1000.0);
                    if let Err(error) = this.sandbox.fire() {
                        this.failed = Some(format!("{error:#}"));
                    }
                    this.pump(window, cx);
                });
            })
        });
        cx.notify();
    }

    fn dark(&self, window: &Window) -> bool {
        matches!(self.theme.unwrap_or(window.appearance()), WindowAppearance::Dark | WindowAppearance::VibrantDark)
    }

    /// The focus handle of `id`, made the first time, which tells its JS as it gets the keyboard and loses it.
    fn focus_handle(&mut self, id: Id, window: &mut Window, cx: &mut Context<Screen>) -> FocusHandle {
        if let Some((handle, _)) = self.focus.get(&id) {
            return handle.clone();
        }
        let handle = cx.focus_handle().tab_stop(true);
        let focused = cx.on_focus(&handle, window, move |this, window, cx| {
            if this.tree.nodes.get(&id).is_some_and(|node| node.listens("focus")) {
                this.event(id, "focus", None, window, cx)
            }
        });
        let blurred = cx.on_blur(&handle, window, move |this, window, cx| {
            if this.tree.nodes.get(&id).is_some_and(|node| node.listens("blur")) {
                this.event(id, "blur", None, window, cx)
            }
        });
        self.focus.insert(id, (handle.clone(), [focused, blurred]));
        handle
    }

    fn element(&mut self, id: Id, dark: bool, window: &mut Window, cx: &mut Context<Screen>) -> Option<AnyElement> {
        let node = self.tree.nodes.get(&id)?;
        if node.style.display.as_deref() == Some("none") {
            return None;
        }
        let style = node.style.clone();
        let kind = node.kind;
        let pressable = node.listens("click") && !node.is("disabled") && !node.is("aria-disabled");
        let semantics = Semantics::of(node);
        let element = match kind {
            Kind::RawText => {
                let text = node.value.clone().unwrap_or_default();
                return Some(gpui::Text::new(ElementId::Integer(id), text.into()).into_any_element());
            }
            Kind::Text => {
                let text = self.tree.text(id);
                let element = div().id(ElementId::Integer(id));
                let element = semantics.apply(element);
                text_style(layout(element, &style), &style, dark, false).child(text).into_any_element()
            }
            Kind::Image => {
                let source = node.str("src").and_then(data_url);
                let element = div().id(ElementId::Integer(id));
                let element = semantics.apply(layout(element, &style));
                match source {
                    Some(image) => element.child(img(image).size_full()).into_any_element(),
                    None => element.into_any_element(),
                }
            }
            Kind::Input => self.input(id, dark, semantics, window, cx),
            Kind::Box => {
                let children: Vec<Id> = node.children.clone();
                let mut element = div().id(ElementId::Integer(id));
                if pressable {
                    let handle = self.focus_handle(id, window, cx);
                    element = element
                        .track_focus(&handle)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| this.event(id, "click", None, window, cx)))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                this.event(id, "click", None, window, cx)
                            }
                        }));
                }
                let element = text_style(semantics.apply(layout(element, &style)), &style, dark, false);
                let children: Vec<AnyElement> =
                    children.into_iter().filter_map(|child| self.element(child, dark, window, cx)).collect();
                element.children(children).into_any_element()
            }
        };
        Some(element)
    }

    /// An `input`: its text, or its placeholder while it's empty, which takes what's typed while it has the keyboard.
    fn input(&mut self, id: Id, dark: bool, semantics: Semantics, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
        let handle = self.focus_handle(id, window, cx);
        let node = &self.tree.nodes[&id];
        let style = node.style.clone();
        let multiline = node.is("multiline");
        let value = self.typed.get(&id).cloned().unwrap_or_else(|| match node.prop("value") {
            Some(Value::String(value)) => value.clone(),
            Some(value) => value.to_string(),
            None => String::new(),
        });
        let shown = if node.str("type") == Some("password") { "•".repeat(value.chars().count()) } else { value.clone() };
        let placeholder = node.str("placeholder").unwrap_or_default().to_owned();
        let focused = handle.is_focused(window);
        let muted = colour("muted-foreground", dark);
        let element = div()
            .id(ElementId::Integer(id))
            .track_focus(&handle)
            .cursor_text()
            .flex()
            .flex_row()
            .items_center()
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some((handle, _)) = this.focus.get(&id) {
                    window.focus(&handle.clone(), cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let Some(node) = this.tree.nodes.get(&id) else { return };
                let mut value = this.typed.get(&id).cloned().unwrap_or_else(|| node.str("value").unwrap_or_default().to_owned());
                let keystroke = &event.keystroke;
                let name = match keystroke.key.as_str() {
                    "enter" if !multiline => return,
                    "enter" => {
                        value.push('\n');
                        "input"
                    }
                    "backspace" => {
                        value.pop();
                        "input"
                    }
                    _ if keystroke.modifiers.platform || keystroke.modifiers.control => return,
                    _ => match &keystroke.key_char {
                        Some(typed) => {
                            value.push_str(typed);
                            "input"
                        }
                        None => return,
                    },
                };
                cx.stop_propagation();
                this.typed.insert(id, value.clone());
                if this.tree.nodes.get(&id).is_some_and(|node| node.listens(name)) {
                    this.event(id, name, Some(value), window, cx);
                } else {
                    cx.notify();
                }
            }));
        let element = text_style(semantics.apply(layout(element, &style)), &style, dark, false);
        let caret = focused.then(|| div().w(px(1.0)).h(relative(1.0)).bg(colour("foreground", dark)));
        if shown.is_empty() {
            element.children(caret).child(div().text_color(muted).child(placeholder)).into_any_element()
        } else {
            element.child(shown).children(caret).into_any_element()
        }
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark(window);
        let root = div()
            .id("strict")
            .key_context("strict")
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .size_full()
            .overflow_y_scroll()
            .bg(colour("background", dark))
            .text_color(colour("foreground", dark));
        // Body, as a page's is (`body` in ui.css), which what's in it inherits.
        let root = text_style(root, &Style::default(), dark, true);
        if let Some(failed) = &self.failed {
            return root.p(px(16.0)).child(failed.clone()).into_any_element();
        }
        let style = self.tree.root().style.clone();
        let children: Vec<Id> = self.tree.root().children.clone();
        let children: Vec<AnyElement> = children.into_iter().filter_map(|child| self.element(child, dark, window, cx)).collect();
        layout(root, &style).children(children).into_any_element()
    }
}

/// Tab and Shift-Tab move the keyboard along what takes it, as a page's do.
pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([gpui::KeyBinding::new("tab", FocusNext, None), gpui::KeyBinding::new("shift-tab", FocusPrevious, None)]);
}

/// The typeface, from the UI library's WOFF2, for GPUI's text system, which reads TrueType. It's Allo's default
/// weight whatever the type style asks: GPUI's text systems match a face by its weight, and don't set a variable
/// font's weight axis as a browser does.
pub fn add_fonts(text_system: &gpui::TextSystem) -> anyhow::Result<()> {
    let ttf = wuff::decompress_woff2(include_bytes!("../../../../core/ui/src/Allo-Variable.woff2"))
        .map_err(|error| anyhow::anyhow!("Allo didn't decode: {error:?}"))?;
    text_system.add_fonts(vec![ttf.into()])
}


/// A node's semantics as AccessKit has them, on its element.
struct Semantics {
    role: Option<gpui::accesskit::Role>,
    label: Option<String>,
    description: Option<String>,
    selected: Option<bool>,
    expanded: Option<bool>,
    checked: Option<gpui::accesskit::Toggled>,
}

impl Semantics {
    fn of(node: &Node) -> Semantics {
        let role = node.str("role").and_then(role).or(match node.kind {
            Kind::Image => Some(gpui::accesskit::Role::Image),
            Kind::Input if node.str("type") == Some("search") => Some(gpui::accesskit::Role::SearchInput),
            Kind::Input if node.is("multiline") => Some(gpui::accesskit::Role::MultilineTextInput),
            Kind::Input => Some(gpui::accesskit::Role::TextInput),
            _ => None,
        });
        let checked = node.str("aria-checked").map(|checked| match checked {
            "true" => gpui::accesskit::Toggled::True,
            "mixed" => gpui::accesskit::Toggled::Mixed,
            _ => gpui::accesskit::Toggled::False,
        });
        let state = |name: &str| node.str(name).map(|value| value == "true");
        Semantics {
            role,
            label: node.str("aria-label").or_else(|| node.str("alt")).map(str::to_owned),
            description: node.str("aria-description").map(str::to_owned),
            selected: state("aria-selected"),
            expanded: state("aria-expanded"),
            checked,
        }
    }

    fn apply(self, mut element: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
        if let Some(role) = self.role {
            element = element.role(role);
        }
        if let Some(label) = self.label {
            element = element.aria_label(label);
        }
        if let Some(description) = self.description {
            element = element.aria_description(description);
        }
        if let Some(selected) = self.selected {
            element = element.aria_selected(selected);
        }
        if let Some(expanded) = self.expanded {
            element = element.aria_expanded(expanded);
        }
        if let Some(checked) = self.checked {
            element = element.aria_toggled(checked);
        }
        element
    }
}

/// ARIA's role as AccessKit's.
fn role(role: &str) -> Option<gpui::accesskit::Role> {
    use gpui::accesskit::Role;
    Some(match role {
        "alert" => Role::Alert,
        "button" => Role::Button,
        "checkbox" => Role::CheckBox,
        "dialog" => Role::Dialog,
        "group" => Role::Group,
        "heading" => Role::Heading,
        "img" => Role::Image,
        "link" => Role::Link,
        "list" => Role::List,
        "listitem" => Role::ListItem,
        "main" => Role::Main,
        "menu" => Role::Menu,
        "menuitem" => Role::MenuItem,
        "navigation" => Role::Navigation,
        "progressbar" => Role::ProgressIndicator,
        "radio" => Role::RadioButton,
        "radiogroup" => Role::RadioGroup,
        "region" => Role::Region,
        "searchbox" => Role::SearchInput,
        "separator" => Role::Splitter,
        "slider" => Role::Slider,
        "status" => Role::Status,
        "switch" => Role::Switch,
        "tab" => Role::Tab,
        "tablist" => Role::TabList,
        "tabpanel" => Role::TabPanel,
        "textbox" => Role::TextInput,
        "toolbar" => Role::Toolbar,
        "banner" => Role::Banner,
        _ => return None,
    })
}

/// A `data:` address's image.
fn data_url(src: &str) -> Option<Arc<gpui::Image>> {
    let (head, body) = src.strip_prefix("data:")?.split_once(',')?;
    let format = match head.split(';').next()? {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        "image/svg+xml" => ImageFormat::Svg,
        _ => return None,
    };
    let bytes = if head.ends_with(";base64") {
        data_encoding::BASE64.decode(body.as_bytes()).ok()?
    } else {
        body.as_bytes().to_vec()
    };
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}
