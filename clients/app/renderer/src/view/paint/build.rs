//! A screen's tree as Taffy's nodes, styled again only where it or what it inherits changed (`Painter::build`, `restyle`).

use super::*;

impl Painter<'_> {
    /// `id` and what's in it as Taffy's nodes, styled again only where it or what it inherits changed, and gone into
    /// only where something in it did. The root fills the window and scrolls, as a page's body.
    pub(in crate::view) fn build(&mut self, id: Id, inherited: Text, root: bool) -> NodeId {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let state = State { exiting: tree.leaving_in(id), ..self.states.of(id, node.kind == Kind::Input) };
        let restyle = match self.layout.kept.get_mut(&id) {
            None => true,
            Some(kept) => {
                kept.frame = self.frame;
                let restyle = self.everything
                    || kept.inherited != inherited
                    || kept.state != state
                    || node.kind == Kind::Input
                    || self.changed.contains(&id)
                    || self.moving.contains(&id);
                if !restyle && !self.below.contains(&id) {
                    return kept.node;
                }
                restyle
            }
        };
        let text = match restyle {
            true => self.restyle(id, inherited, state, root),
            false => self.layout.leaf(self.layout.kept[&id].node).text,
        };
        if matches!(node.kind, Kind::Box | Kind::Scroll | Kind::List) {
            let mut children: Vec<Id> = node.children.iter().copied().filter(|child| tree.nodes.contains_key(child)).collect();
            // A list's rows are laid out in their order, whatever order they came in.
            if node.kind == Kind::List {
                children.sort_by_key(|child| super::super::list::index(&tree.nodes[child]));
            }
            let built: Vec<NodeId> = children.iter().map(|child| self.build(*child, text, false)).collect();
            // What's fixed is laid out in the window, as the root's (`Retained::place_fixed`).
            let inside: Vec<NodeId> = built.into_iter().filter(|child| !self.layout.leaf(*child).fixed).collect();
            let kept = self.layout.kept.get_mut(&id).expect("it's built");
            if kept.children != children {
                let now: HashSet<&Id> = children.iter().collect();
                self.gone.extend(kept.children.iter().filter(|child| !now.contains(child)));
                kept.children = children;
            }
            if kept.inside != inside {
                if !root {
                    self.layout.taffy.set_children(kept.node, &inside).expect("its children are Taffy's");
                }
                kept.inside = inside;
            }
            if node.kind == Kind::List {
                self.layout.hold(id, node, tree);
            }
        }
        self.layout.kept[&id].node
    }

    /// Where each node that moves as it's laid out again is laid out now, in the window, as it would be unscrolled, so
    /// what's scrolled doesn't move it: each that's moved goes there from where it was (`Transitions::laid_out`).
    pub(in crate::view) fn laid_out(&mut self) {
        let moves: Vec<(Id, Option<String>)> = self.tree.moves_layout().map(|(id, shared)| (id, shared.map(str::to_owned))).collect();
        for (id, shared) in moves {
            let Some(laid) = self.layout.kept.get(&id).and_then(|kept| self.layout.taffy.layout(kept.node).ok()).copied() else { continue };
            let mut origin = point(laid.location.x, laid.location.y);
            let mut at = id;
            while let Some(parent) = self.tree.parent(at).filter(|_| !self.layout.kept.get(&at).is_some_and(|kept| self.layout.leaf(kept.node).fixed)) {
                let Some(above) = self.layout.kept.get(&parent).and_then(|kept| self.layout.taffy.layout(kept.node).ok()) else { break };
                origin = point(origin.x + above.location.x, origin.y + above.location.y);
                at = parent;
            }
            let bounds = Bounds { origin, size: Size { width: laid.size.width, height: laid.size.height } };
            self.transitions.laid_out(id, shared.as_deref(), bounds, self.time, self.frame, self.conditions.reduced_motion || self.unseen);
        }
    }

    /// Styles `id` again, in `state`, inheriting `inherited`, telling Taffy only what's different. → the text it
    /// passes on.
    pub(in crate::view) fn restyle(&mut self, id: Id, inherited: Text, state: State, root: bool) -> Text {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let mut computed = style::of(node, state, self.conditions);
        // What's just been added starts from its `entering` state, to move from.
        let entering = (computed.transition.is_some() && self.transitions.entering(id)).then(|| style::of(node, State { entering: true, ..state }, self.conditions));
        let size = self.layout.kept.get(&id).and_then(|kept| self.layout.taffy.layout(kept.node).ok()).map_or(Size::default(), |laid| Size { width: laid.size.width, height: laid.size.height });
        self.transitions.apply(id, &mut computed, entering.as_ref(), self.time, self.since, self.conditions.reduced_motion, self.unseen, size);
        self.transitions.animate(id, &mut computed, self.time, self.conditions);
        let mut layout = computed.layout.clone();
        let mut look = computed.look.clone();
        // A `Text` inherits what it's in, as CSS does.
        let text = inherited.with(&computed);
        let level = node.prop("aria-level").and_then(|level| level.as_u64().or_else(|| level.as_str()?.parse().ok())).unwrap_or(2);
        let text = match node.kind == Kind::Text && node.styles.is_none() && node.str("role") == Some("heading") && level <= 3 {
            true => text.heading(self.conditions),
            false => text,
        };
        if root {
            // The window, which places what's fixed, and what's absolute with nothing positioned around it.
            layout.position = taffy::style::Position::Relative;
            layout.size = taffy::geometry::Size { width: taffy::style::Dimension::percent(1.0), height: taffy::style::Dimension::percent(1.0) };
            layout.overflow = taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll };
            look.scrolls = true;
        }
        if node.kind == Kind::Scroll {
            let along = matches!(layout.flex_direction, taffy::style::FlexDirection::Row | taffy::style::FlexDirection::RowReverse);
            layout.overflow = if along {
                taffy::geometry::Point { x: taffy::style::Overflow::Scroll, y: taffy::style::Overflow::Hidden }
            } else {
                taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll }
            };
            look.clips = true;
            look.scrolls = true;
        }
        layout.scrollbar_width = 0.0;
        let lines = computed.line_clamp;
        let mut spans = Vec::new();
        let measure = match node.kind {
            Kind::Text => {
                let (mut words, mut runs) = (String::new(), Vec::new());
                self.runs(id, text, None, &mut words, &mut runs, &mut spans);
                // One run is the text's own style, which needs none.
                if runs.len() == 1 && runs[0].style == text.style && runs[0].color == text.color && runs[0].background_color.is_none() && runs[0].underline == text.underline {
                    runs.clear();
                }
                Some(Measure::Text { words, runs, text, lines })
            }
            Kind::RawText => Some(Measure::Text { words: node.value.clone().unwrap_or_default(), runs: Vec::new(), text, lines: None }),
            Kind::Input => {
                let words = self.fields.get(&id).map(Field::display).unwrap_or_default();
                let words = if words.is_empty() { node.str("placeholder").unwrap_or_default().to_owned() } else { words };
                Some(Measure::Input { words: if words.is_empty() { " ".into() } else { words }, text, multiline: node.is("multiline") })
            }
            Kind::Frame => Some(Measure::Frame { height: self.frames.get(&id).and_then(|child| child.screen.content_height) }),
            Kind::Image | Kind::Path | Kind::Picker | Kind::Box | Kind::Scroll | Kind::List | Kind::Video => None,
        };
        let positioned = layout.position != taffy::style::Position::Static;
        if look.will_change {
            self.layout.will_change.insert(id);
        } else {
            self.layout.will_change.remove(&id);
        }
        let leaf = Leaf { id, text, look, line_clamp: lines, positioned, fixed: computed.fixed, measure, paragraph: None, spans };
        let taffy = &mut self.layout.taffy;
        match self.layout.kept.get_mut(&id) {
            Some(kept) => {
                if taffy.style(kept.node).is_ok_and(|was| *was != layout) {
                    taffy.set_style(kept.node, layout).expect("taffy takes any style");
                }
                let was = taffy.get_node_context_mut(kept.node).expect("every node has its leaf");
                let measured = was.measure != leaf.measure;
                *was = leaf;
                if measured {
                    taffy.mark_dirty(kept.node).expect("it's Taffy's");
                }
                (kept.inherited, kept.state) = (inherited, state);
            }
            None => {
                let node = taffy.new_leaf_with_context(layout, leaf).expect("taffy takes any style");
                let kept = Kept { node, inherited, state, children: Vec::new(), inside: Vec::new(), frame: self.frame };
                self.layout.kept.insert(id, kept);
            }
        }
        if computed.fixed {
            self.layout.fixed.insert(id);
        } else {
            self.layout.fixed.remove(&id);
        }
        text
    }

    /// What's in text `id`, set as each text in it says, as CSS inherits text properties into an inline: its words, its
    /// runs, and its texts' and raw texts' spans (outer before inner), with `behind` what's behind its words.
    pub(in crate::view) fn runs(&self, id: Id, text: Text, behind: Option<Hsla>, words: &mut String, runs: &mut Vec<TextRun>, spans: &mut Vec<Span>) {
        let tree = self.tree;
        let Some(node) = tree.nodes.get(&id) else { return };
        for child in &node.children {
            let Some(inner) = tree.nodes.get(child).filter(|inner| !inner.hidden()) else { continue };
            let start = words.len();
            match inner.kind {
                Kind::RawText => {
                    words.push_str(inner.value.as_deref().unwrap_or_default());
                    let run = TextRun { len: words.len() - start, style: text.style, color: text.color, background_color: behind, underline: text.underline };
                    match runs.last_mut() {
                        Some(last) if TextRun { len: last.len, ..run } == *last => last.len += run.len,
                        _ if run.len > 0 => runs.push(run),
                        _ => {}
                    }
                    spans.push(Span { id: *child, start, end: words.len(), cursor: None, pointer: None });
                }
                Kind::Text => {
                    let computed = style::of(inner, self.states.of(*child, false), self.conditions);
                    let set = text.with(&computed);
                    let at = spans.len();
                    spans.push(Span { id: *child, start, end: start, cursor: computed.look.cursor, pointer: computed.look.pointer_events });
                    self.runs(*child, set, computed.look.background.or(behind), words, runs, spans);
                    spans[at].end = words.len();
                }
                _ => self.runs(*child, text, behind, words, runs, spans),
            }
        }
    }
}
