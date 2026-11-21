//! A frame, in its passes (`paint`): what's styled again (`plan`), the tree styled, laid out and painted with the window's own title and notice over it (`draw`), what's kept of it (`keep`), and what's told of where things landed.

use super::*;

impl Screen {
    /// The conditions its tokens' values vary by, as its window is now.
    pub(super) fn conditions(&self) -> Conditions {
        let insets = [self.insets.top, self.insets.right, self.insets.bottom, self.insets.left].map(|inset| inset.round().max(0.0) as u16);
        let title_bar = [self.chrome.title_start, if self.chrome.title_end > 0 { self.chrome.title_end } else if self.chrome.title_start > 0 { 16 } else { 0 }];
        let (phone, rem) = match self.text_size {
            Some(size) => (true, (size * 100.0).round() as u16),
            None => (false, Conditions::default().rem),
        };
        Conditions { dark: self.dark(), touch: self.touch, phone, reduced_motion: self.reduced_motion, insets, title_bar, page: self.page, rem }
    }

    /// Draws its canvas over what it painted, `opacity` as opaque: its page's background with nothing on it, under its
    /// title row where its window has one (`title_row`), as its window shows while its page starts (`platform`'s
    /// `Cover`).
    pub(crate) fn paint_canvas(&self, room: Size<Pixels>, scale: f32, opacity: f32, scene: &mut Scene) {
        let (_, background) = Text::root(self.conditions());
        let Some(background) = background else { return };
        let top = if self.title.is_some() { self.insets.top } else { 0.0 };
        let canvas = Bounds { origin: point(0.0, top), size: Size { width: room.width.0, height: (room.height.0 - top).max(0.0) } };
        scene.insert_primitive(Quad {
            order: 0,
            border_style: Default::default(),
            bounds: canvas.scale_to(scale),
            content_mask: ContentMask { bounds: canvas.scale_to(scale) },
            background: Background::from(Hsla { a: background.a * opacity, ..background }),
            border_color: Hsla::default(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
            transformation: TransformationMatrix::unit(),
        });
    }

    /// Its GPU was lost, and what it drew with is made again: what it kept in the old atlas (its pictures, and the
    /// layers drawn from them) goes, and it's all drawn afresh, its frames' pages too.
    pub fn gpu_lost(&mut self) {
        self.pictures.forget();
        self.layers.clear();
        self.seen = None;
        for child in self.frames.values_mut() {
            child.screen.gpu_lost();
        }
    }

    /// Where its page asked for the system's own material, as it was last drawn, which a panel's host puts under it.
    pub fn glass(&self) -> &[crate::platform::Glass] {
        &self.chrome.glass
    }

    /// Paints the screen into `scene`, a scene of its own, all of it, as an offscreen picture of it is: `paint` keeps the
    /// scene a frame that changes nothing drew, which a new scene doesn't have.
    pub fn paint_afresh(&mut self, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
        self.seen = None;
        self.paint(None, size, scale, text, atlas, scene);
    }

    /// Paints the screen into `scene`, `size` logical pixels at `scale` device pixels each, as a frame's passes, in
    /// order, as GPUI's and Flutter's frames run theirs: the page told its room; each frame's page painted; whether
    /// anything it shows changed, and what's styled again and what's drawn from a layer (`plan`); the tree styled, laid
    /// out and painted (`draw`); what that drew kept, for the pointer, the keyboard and the window (`keep`); and then the
    /// keyboard where a dialog came or went (`settle_focus`), and what's told of where things landed (`tell_placed`).
    /// What moves by itself (a scroll's coast, a spring, a transition) is drawn where it is at the time of the frame
    /// it's for, `at`, the display's (as `requestAnimationFrame` is told it, Core Animation's link and Flutter's frames
    /// have it), or with none (drawn as input came), now: drawn at the time it was painted, a frame the loop's own clock
    /// drew a quarter refresh late for a link that stopped moved a step and a quarter, and the next three quarters.
    /// → whether it painted anything, which a frame that changed nothing doesn't: the scene drawn last is it.
    pub fn paint(&mut self, at: Option<Instant>, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) -> bool {
        self.tell_room(size);
        self.paint_frames(at, scale, text, atlas);
        let started = crate::platform::on_cpu();
        self.cost.script += self.js.script();
        // A frame that changes nothing paints nothing: the scene drawn last is kept.
        let now = at.map_or_else(|| self.js.clock().now(), |at| crate::platform::time_at(self.js.clock(), at));
        let Some(plan) = self.plan(now, size, scale) else { return false };
        scene.clear();
        let drawn = self.draw(&plan, text, atlas, scene);
        let modal = drawn.modal;
        let intersections = self.keep(drawn, &plan);
        self.painted = (plan.window.size, scale);
        self.pictures.drawn();
        self.tell_settled();
        scene.finish();
        self.settle_focus(modal);
        self.tell_placed(intersections, plan.scrolled.then_some(plan.now));
        self.cost.paint += crate::platform::on_cpu().saturating_sub(started);
        self.cost.painted += 1;
        true
    }

    /// The page is told when its room, its motion, its insets or the device's pressure change, as a frame's host tells
    /// an MCP App.
    fn tell_room(&mut self, size: Size<Pixels>) {
        self.room = Size { width: size.width.0, height: size.height.0 };
        let now = (self.room, self.reduced_motion, self.insets, self.pressure);
        if self.told.is_some_and(|told| told != now) {
            self.told = Some(now);
            let context = json!({
                "containerDimensions": {"width": self.room.width, "height": self.room.height},
                "reducedMotion": self.reduced_motion,
                "pressure": self.pressure.name(),
                "safeAreaInsets": self.safe_area(),
            });
            self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": context}));
        }
    }

    /// Each frame's page first, in the room its frame had, as this one looks, which this frame splices in.
    fn paint_frames(&mut self, at: Option<Instant>, scale: f32, text: &mut TextSystem, atlas: &Atlas) {
        for (id, child) in &mut self.frames {
            let screen = &mut child.screen;
            (screen.theme, screen.appearance, screen.reduced_motion, screen.touch) = (self.theme, self.appearance, self.reduced_motion, self.touch);
            screen.active = self.active && self.focus.focused == Some(*id);
            // A page that says it's another height has its frame laid out again.
            if child.height != screen.content_height {
                child.height = screen.content_height;
                self.layout.conditions = None;
            }
            if child.rect.size.width > 0.0 && child.rect.size.height > 0.0 {
                screen.paint(at, Size { width: Pixels(child.rect.size.width), height: Pixels(child.rect.size.height) }, scale, text, atlas, &mut child.scene);
                self.again |= screen.take_again();
            }
        }
    }

    /// What this frame draws: nothing, where nothing it shows has changed, which keeps the scene drawn last and costs
    /// nothing; or what's styled again (what changed, what's moving, each input, as its words are its field's, and
    /// what's changed state, with what's around it, so the rest is kept as it was laid out), and what's drawn from a
    /// layer, moved, rather than painted again.
    fn plan(&mut self, now: f64, size: Size<Pixels>, scale: f32) -> Option<Plan> {
        let conditions = self.conditions();
        let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
        self.pictures.take();
        self.coast(now);
        self.states.focused = self.focus.focused;
        self.states.touch = self.touch;
        let caret = self.caret_shows();
        let hovering = |bounds: &Bounds<f32>| self.pointer.is_some_and(|at| bounds.contains(&at));
        let mut fields: Vec<_> = self.editing.fields.iter().map(|(id, field)| (*id, field.display(), field.value(), field.placed(), field.upstream())).collect();
        fields.sort_by_key(|(id, ..)| *id);
        let seen = Seen {
            size: (window.size, scale),
            conditions,
            states: self.states.clone(),
            scrolled: self.scrolled.clone(),
            stretched: self.stretched().into_iter().collect(),
            fields,
            caret,
            handles: self.handles_on(),
            hovering: self.chrome.action.as_ref().is_some_and(hovering),
            notice: self.notice.as_ref().map(|notice| (notice.text.clone(), notice.action.as_ref().map(|(label, _)| label.clone()))),
            title: self.title.as_ref().map(|title| (title.name.clone(), title.start)),
            panel: self.panel.map(|panel| (panel.corner, panel.fill)),
            line: self.line(),
            fetched: self.fetched.len(),
            decoded: self.pictures.decoded,
            frames: self.frames.iter().map(|(id, child)| (*id, child.screen.frame)).collect(),
        };
        // What changed since (`changed`), as a node held by a finger that's moved, is drawn, though nothing else did.
        if !self.animating && self.changed.is_none() && self.tree.unchanged() && self.seen.as_ref() == Some(&seen) && self.stale_lists().is_empty() {
            return None;
        }
        // What's kept of what only moves, grows or fades is drawn again where what it draws from changed: anything
        // fetched, decoded, scrolled or in another frame's page, or the window's conditions.
        let scrolled = self.seen.as_ref().is_some_and(|was| (&was.scrolled, &was.stretched) != (&seen.scrolled, &seen.stretched));
        if self.seen.as_ref().is_none_or(|was| (was.size, was.conditions, &was.scrolled, &was.stretched, was.fetched, was.decoded, &was.frames) != (seen.size, seen.conditions, &seen.scrolled, &seen.stretched, seen.fetched, seen.decoded, &seen.frames)) {
            self.layers.clear();
        }
        self.seen = Some(seen);
        // What's styled again: what changed, what's moving, each input (its words are its field's), and what's
        // changed state, with what's around it, so the rest is kept as it was laid out.
        let everything = self.layout.conditions != Some(conditions);
        self.layout.conditions = Some(conditions);
        let mut changed = self.tree.take_changed();
        let moving = self.transitions.moving();
        let mut below = HashSet::new();
        let states = self.states.changed(&self.styled);
        self.styled = self.states.clone();
        // A text inside a text is drawn as a run of the one it's in, which is styled again as its state changes.
        changed.extend(states.iter().filter_map(|id| self.tree.outer_text(*id)));
        // A list whose rows' heights changed holds their room again (`list`).
        let stale = self.stale_lists();
        for id in changed.iter().chain(&moving).chain(&states).copied().chain(self.tree.inputs()).chain(stale) {
            let mut at = Some(id);
            while let Some(id) = at.filter(|id| below.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        // What's moving, growing or fading and nothing else, with nothing in it changing, moving otherwise or holding
        // the keyboard, is drawn from its layer, moved; anything else in it that changes draws it all again.
        let whole = self.transitions.moving_whole(now);
        let mut touched = HashSet::new();
        let focused = self.states.focused.filter(|id| self.editing.fields.contains_key(id));
        for id in changed.iter().chain(&states).chain(moving.iter().filter(|id| !whole.contains(id))).chain(&focused).copied() {
            let mut at = Some(id);
            while let Some(id) = at.filter(|id| touched.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        // What's in a layer and moving as a layer of its own is drawn into it each frame, where it's moved to, as a
        // compositor draws a sublayer; anything else moving in it draws it again.
        let drawing: Vec<Id> = moving.iter().filter(|id| !whole.contains(id) || touched.contains(id)).copied().collect();
        for id in &drawing {
            let mut at = self.tree.parent(*id);
            while let Some(id) = at.filter(|id| touched.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        // What says it's about to move (`will-change`) is one as it holds still too: drawn ahead, it only moves then.
        let layered: HashSet<Id> = whole.iter().chain(&self.layout.will_change).filter(|id| !touched.contains(id)).copied().collect();
        self.layers.retain(|id, _| layered.contains(id));
        // It's drawn ahead in a frame where nothing moves, as a browser draws in idle time, not in one that's busy.
        let still = moving.is_empty();
        self.frame += 1;
        Some(Plan { conditions, window, scale, now, caret, everything, changed, moving, below, layered, still, scrolled })
    }

    /// Styles, lays out and paints the tree into `scene`, as `plan` says, and the window's own title and notice
    /// over it. → what the painter found as it drew.
    fn draw(&mut self, plan: &Plan, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) -> Drawing {
        let Plan { conditions, window, scale, now, caret, everything, ref changed, ref moving, ref below, ref layered, still, .. } = *plan;
        let line = self.line();
        let stretched = self.stretched();
        let handles = self.handles_on();
        let mut painter = Painter {
            tree: &self.tree,
            layout: &mut self.layout,
            changed,
            moving,
            below,
            everything,
            frame: self.frame,
            gone: Vec::new(),
            text,
            atlas,
            scene,
            scale,
            conditions,
            states: &self.states,
            placed: Vec::new(),
            tab_order: Vec::new(),
            scrolled: &mut self.scrolled,
            stretched,
            fields: &self.editing.fields,
            drawn: &mut self.editing.drawn,
            texts: &mut self.editing.texts,
            selected: self.editing.selected.map(|selected| (selected.id, selected.range())),
            caret,
            handles,
            pictures: &mut self.pictures,
            fetched: &self.fetched,
            wanted: Vec::new(),
            rects: Rects::new(),
            time: now,
            since: self.changed.take().unwrap_or(now).min(now),
            transitions: &mut self.transitions,
            animating: false,
            intersections: Vec::new(),
            near: Bounds { origin: point(-window.size.width / 2.0, -window.size.height / 2.0), size: Size { width: window.size.width * 2.0, height: window.size.height * 2.0 } },
            window,
            modal: None,
            turn: TransformationMatrix::unit(),
            glass: self.panel.map(|_| Vec::new()),
            fill: self.panel.filter(|panel| panel.fill).map(|_| colour_token("card", conditions)),
            drag_regions: Vec::new(),
            kept_regions: Vec::new(),
            frames: &self.frames,
            framed: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            videos: &mut self.videos,
            layers: &mut self.layers,
            layered,
            still,
            unseen: !self.visible,
            recording: None,
            parts: Vec::new(),
            unsettled: None,
        };
        // The window, as a page's body: its root's background and text, scrolling what's taller than it.
        let (body, background) = Text::root(conditions);
        // A panel has no background of its own: what its page draws is over the system's glass, or the host's fill.
        match self.panel {
            None => painter.quad(window, window, background.unwrap_or_default(), Edges::default(), Hsla::default(), Corners::default(), false, 1.0),
            Some(panel) if panel.whole => {
                if let Some(glass) = painter.glass.as_mut() {
                    glass.push(crate::platform::Glass { bounds: window, radius: panel.corner, opacity: 1.0 });
                }
                if let Some(fill) = painter.fill {
                    painter.quad(window, window, fill, Edges::default(), Hsla::default(), Corners::all(panel.corner), false, 1.0);
                }
            }
            Some(_) => {}
        }
        let root = painter.build(0, body, true);
        // What's no longer in the tree goes, unless it moved elsewhere in it.
        for id in std::mem::take(&mut painter.gone) {
            painter.layout.forget(id, painter.frame, painter.transitions);
        }
        painter.layout.place_fixed(painter.tree);
        let available = taffy::geometry::Size { width: AvailableSpace::Definite(window.size.width), height: AvailableSpace::Definite(window.size.height) };
        let laying_out = crate::platform::on_cpu();
        let mut measured = lay_out(painter.layout, painter.text, root, available);
        // A list's rows laid out as other than it placed them for (a row grown, or one it hadn't drawn) are placed again
        // as they are, and laid out again, in this frame, as rows in a column would be.
        if measured.is_ok() && painter.layout.measure_lists(painter.tree) {
            for id in painter.layout.lists.iter().filter(|(_, listed)| listed.stale).map(|(id, _)| *id).collect::<Vec<_>>() {
                if let Some(node) = painter.tree.nodes.get(&id) {
                    painter.layout.hold(id, node, painter.tree);
                }
            }
            measured = lay_out(painter.layout, painter.text, root, available);
            self.again |= painter.layout.measure_lists(painter.tree);
        }
        self.cost.layout += crate::platform::on_cpu().saturating_sub(laying_out);
        match measured {
            Err(error) => log::error!("its layout failed: {error}"),
            Ok(()) => {
                painter.laid_out();
                painter.paint(root, point(0.0, 0.0), Transform::NONE, window, 1.0, None, true)
            }
        }
        // The window's title, over its page.
        let hovering = |bounds: &Bounds<f32>| self.pointer.is_some_and(|at| bounds.contains(&at));
        let edge = self.full_screen.then_some(EDGE);
        // What's under the pointer, or has the keyboard by Tab, is lit.
        let keyed = |id: Id| self.states.focus_visible && self.focus.focused == Some(id);
        // Its own window buttons, as a header bar's: at the title row's start, before the title, and at its end,
        // which the page's top bar stops short of.
        const BUTTON: f32 = 24.0;
        const EDGE_GAP: f32 = 12.0;
        let (starts, ends) = &self.window_buttons;
        let mut buttons = Vec::new();
        for (at, kind) in starts.iter().enumerate() {
            buttons.push((*kind, Bounds { origin: point(EDGE_GAP + at as f32 * (BUTTON + 8.0), (self.insets.top - BUTTON) / 2.0), size: Size { width: BUTTON, height: BUTTON } }));
        }
        for (at, kind) in ends.iter().rev().enumerate() {
            // Windows' are Windows' own shape: flush with the window's edge and as tall as the row, in a row.
            buttons.push((
                *kind,
                if cfg!(windows) {
                    Bounds { origin: point(window.size.width - (at + 1) as f32 * CAPTION_BUTTON, 0.0), size: Size { width: CAPTION_BUTTON, height: self.insets.top } }
                } else {
                    Bounds { origin: point(window.size.width - EDGE_GAP - BUTTON - at as f32 * (BUTTON + 8.0), (self.insets.top - BUTTON) / 2.0), size: Size { width: BUTTON, height: BUTTON } }
                },
            ));
        }
        for (kind, bounds) in &buttons {
            painter.window_button(*kind, *bounds, hovering(bounds) || self.chrome.pressing_button == Some(*kind));
        }
        self.chrome.buttons = buttons;
        let after_buttons = (!starts.is_empty()).then(|| EDGE_GAP + starts.len() as f32 * (BUTTON + 8.0));
        let title = self.title.as_ref().map(|title| painter.title(title, edge.or(after_buttons).unwrap_or(title.start), self.insets.top));
        // The page's top bar starts after it, and ends before the buttons at the end, which it's laid out again for
        // once either moves.
        let start = title.map_or(0, |end| (end + 12.0).ceil() as u16);
        let end = match (ends.len(), cfg!(windows)) {
            (0, _) => 0,
            (count, true) => (count as f32 * CAPTION_BUTTON).ceil() as u16,
            (count, false) => (EDGE_GAP + count as f32 * (BUTTON + 8.0) + 4.0).ceil() as u16,
        };
        if (start, end) != (self.chrome.title_start, self.chrome.title_end) {
            (self.chrome.title_start, self.chrome.title_end) = (start, end);
            self.again = true;
        }
        self.chrome.action = self.notice.as_ref().and_then(|notice| painter.notice(notice, self.insets.top, self.chrome.action.as_ref().is_some_and(hovering) || keyed(NOTICE_ACTION)));
        // The rail's line, over everything: dark over a light page and light over a dark one, as iOS's home
        // indicator is, the page's own label colour.
        if let Some(line) = line {
            painter.quad(line, window, colour_token("label", conditions), Edges::default(), Hsla::default(), Corners::all(line.size.width / 2.0), false, 1.0);
        }
        self.chrome.glass = painter.glass.take().unwrap_or_default();
        self.chrome.drag_regions = std::mem::take(&mut painter.drag_regions);
        self.chrome.kept_regions = std::mem::take(&mut painter.kept_regions);
        Drawing {
            placed: painter.placed,
            tab_order: painter.tab_order,
            rects: painter.rects,
            animating: painter.animating,
            modal: painter.modal,
            wanted: painter.wanted,
            intersections: painter.intersections,
            framed: painter.framed,
        }
    }

    /// Keeps what the frame drew, for the pointer, the keyboard, the inspector and the window: where each frame's page
    /// is, the pictures to fetch, what's under the pointer and Tab's order. → where each node listening for
    /// `intersection` is near the window.
    fn keep(&mut self, drawn: Drawing, plan: &Plan) -> Vec<(Id, bool)> {
        let Drawing { placed, mut tab_order, rects, animating, modal, wanted, intersections, framed } = drawn;
        // Each frame's page is drawn in the room it had now from the next frame, if that's changed.
        for (id, rect) in framed {
            if let Some(child) = self.frames.get_mut(&id) {
                self.again |= child.rect.size != rect.size;
                child.rect = rect;
            }
        }
        // What it wanted to draw that's by an address is fetched by its window, once; with nothing to fetch it, it
        // isn't found.
        for src in wanted {
            if !self.fetched.contains_key(&src) && !self.fetching.contains(&src) {
                match self.fetch.as_mut() {
                    Some(fetch) => {
                        fetch(&src);
                        self.fetching.insert(src);
                    }
                    None => drop(self.fetched.insert(src, None)),
                }
            }
        }
        let moving = self.transitions.finish(plan.now);
        // What's going is gone once nothing in it moves, having been drawn going.
        let still = self.transitions.moving();
        let gone: Vec<Id> = self.tree.going().filter(|id| self.going.contains(id) && !still.iter().any(|moving| self.within(*moving, *id))).collect();
        for id in gone {
            self.tree.left(id);
            self.again = true;
        }
        self.going = self.tree.going().collect();
        self.placed = placed;
        // A modal dialog holds the keyboard: Tab goes only to what's in it, and it has the keyboard as it appears.
        // What's inert, or in what is, Tab passes over.
        if let Some(modal) = modal {
            tab_order.retain(|id| self.within(*id, modal));
        }
        tab_order.retain(|id| !self.tree.inert(*id));
        // The window's own notice action comes first, as it's first in it, unless a modal holds the keyboard.
        if modal.is_none() && self.chrome.action.is_some() {
            tab_order.insert(0, NOTICE_ACTION);
        }
        self.focus.tab_order = tab_order;
        self.rects = rects;
        self.animating = animating || moving || self.coasting();
        // What's about to move and wasn't drawn ahead, as something was moving, is in a frame of its own once it stops.
        self.again |= !plan.still && self.layout.will_change.iter().any(|id| plan.layered.contains(id) && !self.layers.get(id).is_some_and(|layer| layer.recorded));
        intersections
    }

    /// What's told of where things landed: what a still pointer is over now, as a browser finds again after layout;
    /// where each node listening for `layout` is; and whether each listening for `intersection` is near the window.
    fn tell_placed(&mut self, intersections: Vec<(Id, bool)>, scrolled: Option<f64>) {
        // What took the keyboard as its page said so (`autofocus`) is scrolled into view once it's drawn, as a browser's
        // `focus()` does it, which is how the arrow keys on a list's rows go on past what shows.
        if let Some(id) = self.focus.reveal.take() {
            if self.reveal(id) {
                self.changed();
                self.again = true;
            }
        }
        self.ask_lists(self.painted.0.height);
        // What moved under a pointer that's still is what it's over now, as a browser finds again after layout; what
        // scrolled under it, once it's stopped a moment (`hover_after`), so a wheel's notches move what's drawn rather
        // than lighting each row they pass under it, which drew each again.
        if let Some(now) = scrolled {
            self.hover_after = Some(now + HOVER_AFTER_SCROLL);
        } else if let Some(at) = self.pointer.filter(|_| !self.touch && self.hover_after.is_none()) {
            let before = self.states.hovered.clone();
            self.hover(at);
            self.again |= self.states.hovered != before;
        }
        // Where each node that listens for `layout` is, as its layout settles and whenever its size changes, and again
        // as it shows after it wasn't drawn, as a browser's ResizeObserver tells an element shown after `display: none`
        // (a menu's items, kept unseen while it's closed, measured as it opens), and as it starts listening again, as
        // a ResizeObserver tells what it's just been asked to observe.
        let (rects, tree) = (&self.rects, &self.tree);
        self.laid_out.retain(|id, _| rects.get(id).is_some() && tree.nodes.get(id).is_some_and(|node| node.listens("layout")));
        let listening: Vec<(Id, Rect)> = self
            .tree
            .laid_out()
            .filter_map(|id| Some((id, *self.rects.get(&id)?)))
            .filter(|(id, rect)| self.laid_out.get(id).is_none_or(|was| (was.width, was.height) != (rect.width, rect.height)))
            .collect();
        for (id, rect) in listening {
            self.laid_out.insert(id, rect);
            self.tell(id, "layout", json!({"rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}}));
            self.again = true;
        }
        // Whether each node that listens for `intersection` is near the window, as it's first laid out and whenever
        // that changes.
        self.intersecting.retain(|id, _| self.tree.nodes.contains_key(id));
        for (id, near) in intersections {
            if self.intersecting.insert(id, near) != Some(near) {
                self.tell(id, "intersection", json!({"isIntersecting": near}));
                self.again = true;
            }
        }
    }

    /// Whether what it did as it painted (a dialog taking the keyboard) changed it, so it's painted again.
    pub fn take_again(&mut self) -> bool {
        std::mem::take(&mut self.again) || self.animating
    }
}

/// Lays `layout` out from `root` in `available`, measuring a text's words and an input's lines with `text`, as CSS
/// sizes what has nothing in it: its style's size, or its padding and border around what it measures.
fn lay_out(layout: &mut Retained, text: &mut TextSystem, root: NodeId, available: taffy::geometry::Size<AvailableSpace>) -> Result<(), taffy::TaffyError> {
    layout.taffy.compute_layout_with_measure(root, available, |input, _, leaf, style| {
        // `space` is the room for its content, inside its padding and border.
        taffy::compute_leaf_layout(input, style, |_, _| 0.0, |_, space| {
            let Some(Leaf { measure: Some(measure), .. }) = leaf else {
                return taffy::geometry::Size::ZERO;
            };
            let width = match space.width {
                AvailableSpace::Definite(width) => Some(width),
                AvailableSpace::MinContent => Some(0.0),
                AvailableSpace::MaxContent => None,
            };
            let size = match measure {
                // A text with no words has no lines, so it's no taller than its padding, as CSS lays out a block
                // with nothing in it: a row's heading that isn't there leaves no line above its text.
                Measure::Text { words, .. } if words.is_empty() => Size { width: 0.0, height: 0.0 },
                Measure::Text { words, runs, text: style, lines } => text.paragraph(words, &style.style, runs, width.filter(|_| style.wrap), *lines).size(),
                Measure::Input { words, text: style, multiline } => {
                    let paragraph = text.paragraph(words, &style.style, &[], multiline.then_some(width).flatten(), None);
                    Size { width: if *multiline { paragraph.size().width } else { 0.0 }, height: paragraph.size().height }
                }
                Measure::Frame { height } => Size { width: width.unwrap_or(300.0), height: height.unwrap_or(150.0) },
            };
            taffy::geometry::Size { width: size.width.ceil(), height: size.height }
        })
    })
}

/// Paints a screen that says only `words`, in the middle, in the muted colour, as a page's empty state does.
pub(crate) fn paint_message(words: &str, appearance: Appearance, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
    scene.clear();
    message(words, appearance, size, scale, text, atlas, scene);
    scene.finish();
}

/// What `paint_message` paints, in a scene that's finished after.
fn message(words: &str, appearance: Appearance, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
    // A host's own message, in a computer's size: it has no window's conditions.
    let conditions = Conditions { dark: appearance == Appearance::Dark, ..Conditions::default() };
    let (body, background) = Text::root(conditions);
    let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
    let mask = ContentMask { bounds: window.scale_to(scale) };
    scene.insert_primitive(Quad {
        order: 0,
        border_style: Default::default(),
        bounds: window.scale_to(scale),
        content_mask: mask,
        background: Background::from(background.unwrap_or_default()),
        border_color: Hsla::default(),
        corner_radii: Corners::default(),
        border_widths: Edges::default(),
        transformation: TransformationMatrix::unit(),
    });
    let width = (window.size.width - 32.0).max(1.0);
    let paragraph = text.paragraph(words, &body.style, &[], Some(width), None);
    let top = (window.size.height - paragraph.size().height) / 2.0;
    let muted = colour_token("secondary-label", conditions);
    let _ = text.paint(&paragraph, point(px(16.0), px(top)), width, 0.5, &|_| muted, scale, mask, TransformationMatrix::unit(), atlas, scene);
}

/// Paints a window whose page isn't there yet, `opened` ago, as iOS's launch screen is: the app's background, and once
/// a quick page would have shown (`quiet`), the UI library's `Status`, a spinner over "Loading", fading in over `fades`,
/// so a page that's there by then shows none (Andre, 20 Nov). Its spinner turns once a second, as `spin` in
/// core/ui/src/tokens.ts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_loading(opened: Duration, quiet: Duration, fades: Duration, appearance: Appearance, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
    scene.clear();
    message("", appearance, size, scale, text, atlas, scene);
    let t = (opened.saturating_sub(quiet).as_secs_f32() / fades.as_secs_f32()).min(1.0);
    let opacity = t * t * (3.0 - 2.0 * t);
    if opacity <= 0.0 {
        return scene.finish();
    }
    let conditions = Conditions { dark: appearance == Appearance::Dark, ..Conditions::default() };
    let table = crate::tokens::table();
    let (icon, gap) = (table.length("icon", conditions).unwrap_or(16.0), table.length("grid-2", conditions).unwrap_or(8.0));
    let (_, label) = chrome_type(conditions);
    let paragraph = text.paragraph("Loading", &label, &[], None, None);
    let top = (size.height.0 - icon - gap - paragraph.size().height) / 2.0;
    let mask = ContentMask { bounds: Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } }.scale_to(scale) };
    let spinner = Bounds { origin: point((size.width.0 - icon) / 2.0, top), size: Size { width: icon, height: icon } };
    let device = Size { width: (icon * scale).round() as u32, height: (icon * scale).round() as u32 };
    let grey = colour_token("secondary-label", conditions);
    let (key, draw) = path("M12 3a9 9 0 1 0 9 9", "0 0 24 24", device, None, Some(grey), 2.0 * scale);
    if let Some(tile) = pictures::drawn(atlas, &key, draw) {
        let centre = [(spinner.origin.x + icon / 2.0) * scale, (spinner.origin.y + icon / 2.0) * scale];
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: 0,
            opacity,
            bounds: spinner.scale_to(scale),
            content_mask: mask.clone(),
            corner_radii: Corners::default(),
            tile,
            transformation: TransformationMatrix::rotation(opened.as_secs_f32().fract() * 360.0, centre),
        });
    }
    let colour = colour_token("label", conditions);
    let colour = Hsla { a: colour.a * opacity, ..colour };
    let left = (size.width.0 - paragraph.size().width) / 2.0;
    let _ = text.paint(&paragraph, point(px(left), px(top + icon + gap)), paragraph.size().width, 0.0, &|_| colour, scale, mask, TransformationMatrix::unit(), atlas, scene);
    scene.finish();
}

/// What a frame draws, as `Screen::plan` worked it out: the conditions, the window and its scale, the time on the
/// page's clock and whether the caret shows; whether everything is styled again, or what changed, is moving or is in
/// or around either; what's drawn from its layer; and whether nothing moves, so what's about to is drawn ahead.
struct Plan {
    conditions: Conditions,
    window: Bounds<f32>,
    scale: f32,
    now: f64,
    caret: bool,
    everything: bool,
    changed: HashSet<Id>,
    moving: HashSet<Id>,
    below: HashSet<Id>,
    layered: HashSet<Id>,
    still: bool,
    /// Something scrolled since the frame before.
    scrolled: bool,
}

/// What the painter found as it drew a frame (`Screen::draw`): what takes the pointer and the
/// keyboard, where each node is, whether anything moves by itself, the topmost modal, the pictures it wants fetched,
/// what's near the window and where each frame's page is.
struct Drawing {
    placed: Vec<(Id, Placed)>,
    tab_order: Vec<Id>,
    rects: Rects,
    animating: bool,
    modal: Option<Id>,
    wanted: Vec<String>,
    intersections: Vec<(Id, bool)>,
    framed: Vec<(Id, Bounds<f32>)>,
}

#[derive(PartialEq)]
pub(super) struct Seen {
    size: (Size<f32>, f32),
    conditions: Conditions,
    states: States,
    pub(super) scrolled: HashMap<Id, Point<f32>>,
    pub(super) stretched: std::collections::BTreeMap<Id, Point<f32>>,
    fields: Vec<(Id, String, editing::Value, Option<(usize, f32)>, bool)>,
    pub(super) caret: bool,
    /// The input a finger's selection shows its handles in.
    handles: Option<Id>,
    hovering: bool,
    notice: Option<(String, Option<String>)>,
    title: Option<(String, f32)>,
    panel: Option<(f32, bool)>,
    /// Where the rail's line shows, if it does.
    line: Option<Bounds<f32>>,
    fetched: usize,
    decoded: u64,
    /// Each frame's page's frame, as it was drawn.
    frames: std::collections::BTreeMap<Id, u64>,
}
