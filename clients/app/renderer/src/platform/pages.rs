use super::*;

impl Windows {
    /// Whether a window holds something its person typed that its app hasn't taken (`Screen::typed`): any app's, and
    /// a panel while it's showing.
    pub fn typed(&self) -> bool {
        self.windows.values().any(|window| (!window.panel || window.visible) && window.screen.as_ref().is_ok_and(Screen::typed))
    }

    /// Window `id` shows `page`, a new version of its page, in that one's place (`Screen::reload`), as a browser's tab
    /// loads again: its frame stays on screen until the new page has heard back from what it first asked, or has waited
    /// long enough, as a new window shows (`open`), and it says it's drawn again then (`Drawn`). → whether it's open.
    pub fn reload(&mut self, id: WindowId, page: Screen) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        let Ok(screen) = &mut window.screen else { return false };
        screen.reload(page);
        window.waiting.clear();
        window.opened = now();
        (window.drawn, window.live, window.looked, window.reloading, window.dirty) = (false, false, false, true, true);
        window.looking = window.opened;
        true
    }

    /// Does `f` to window `id`'s screen, if it's open, and draws what it changed.
    pub fn with<R>(&mut self, id: WindowId, f: impl FnOnce(&mut Screen) -> R) -> Option<R> {
        let window = self.windows.get_mut(&id)?;
        window.dirty = true;
        window.screen.as_mut().ok().map(f)
    }

    /// Does `job` once window `id`'s page isn't in use (`Screen::when_unused`): where it's still opening, once its page
    /// is attached, so what came between its page being read and its being given it isn't lost.
    pub fn when_unused(&mut self, id: WindowId, job: Box<dyn FnOnce() + Send>) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        match &mut window.screen {
            Ok(screen) => {
                screen.when_unused.push(job);
                screen.unused();
            }
            Err(_) if window.pending => window.when_unused.push(job),
            Err(_) => {}
        }
    }

    /// Every window, for the inspector's `windows`: what it says it is, its id (`window`), and its `window` op's
    /// answer.
    pub fn list(&mut self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for (id, window) in &mut self.windows {
            let mut listed = window.tag.clone();
            listed["window"] = serde_json::json!(id);
            if let Ok(screen) = &mut window.screen
                && let Ok(request) = crate::inspect::Request::parse(r#"{"v":1,"id":0,"op":"window"}"#)
            {
                let answer = std::rc::Rc::new(std::cell::RefCell::new(serde_json::Value::Null));
                let to = answer.clone();
                screen.inspect(request, Box::new(move |answered| *to.borrow_mut() = answered));
                if let Some(result) = answer.borrow_mut()["result"].as_object() {
                    for (name, value) in result {
                        listed[name] = value.clone();
                    }
                }
            }
            out.push(listed);
        }
        out
    }

    /// Answers an inspector's request of window `id`: `snap` and `profile`, whose frames are drawn again offscreen,
    /// and read back, and the rest by its screen.
    pub fn inspect(&mut self, id: WindowId, request: crate::inspect::Request, reply: crate::Reply) {
        let Some(window) = self.windows.get_mut(&id) else {
            return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("there's no window {id} open"))));
        };
        // Closed as its system closes it, as its close button, ⌘W or Alt-F4 do: a panel's put away.
        if request.op == "close" {
            let closed = !window.panel;
            self.input(id, 0, Input::Close);
            return reply(crate::inspect::answer(&request.id, Ok(serde_json::json!({"closed": closed}))));
        }
        // Every input the window gets for `ms`, as a trace's events.
        if request.op == "record" {
            if self.recording.is_some() {
                return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("a recording is under way: ask again once it's said what came"))));
            }
            let started = web_time::Instant::now();
            let until = started + std::time::Duration::from_secs_f64(request.f64("ms").unwrap_or(5000.0).clamp(0.0, 60_000.0) / 1000.0);
            self.recording = Some(conformance::Recording { window: id, started, until, events: Vec::new(), id: request.id, reply });
            return;
        }
        // Whether the host keeps its contract: its window kept moving, then still, what it does meanwhile recorded.
        if request.op == "host" {
            if self.check.is_some() {
                return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("a check is under way: ask again once it's said what held"))));
            }
            self.record = Default::default();
            self.check = Some(conformance::Check { window: id, started: web_time::Instant::now(), id: request.id, reply });
            return;
        }
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let device = Size { width: DevicePixels((room.width.0 * scale).round() as i32), height: DevicePixels((room.height.0 * scale).round() as i32) };
        let Ok(screen) = &mut window.screen else {
            // One whose page isn't there yet, or didn't open, is pictured as it was last drawn: its loading state, or why.
            if request.op != "snap" {
                let why = window.screen.as_ref().err().filter(|why| !why.is_empty()).map_or("it's still loading", String::as_str);
                return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("its page isn't there: {why}"))));
            }
            let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
            renderer.draw(&window.scene, &[[0.0, 0.0]]);
            renderer.finish();
            let answered = renderer.read(0).ok_or_else(|| anyhow::anyhow!("nothing was drawn")).and_then(|(width, height, rgba)| crate::inspect::png(width, height, rgba));
            renderer.destroy();
            return reply(crate::inspect::answer(&request.id, answered));
        };
        window.dirty = true;
        if !matches!(request.op.as_str(), "snap" | "profile" | "replay") {
            return screen.inspect(request, reply);
        }
        let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
        // Painted afresh, as it is now: a window with no surface (Android's view while it's hidden) has drawn nothing,
        // and the scene it last drew may be older than what it shows. Each of a profile's or a replay's frames after the
        // first is painted as the window paints one, from what changed.
        let mut scene = crate::render::scene::Scene::default();
        let (text, atlas) = (&mut self.text, &self.atlas);
        let snaps = crate::inspect::snaps(&request);
        let mut afresh = true;
        let mut draw = |screen: &mut Screen| -> anyhow::Result<serde_json::Value> {
            if std::mem::take(&mut afresh) {
                screen.paint_afresh(room, scale, text, atlas, &mut scene);
            } else {
                screen.paint(None, room, scale, text, atlas, &mut scene);
            }
            renderer.draw(&scene, &[[0.0, 0.0]]);
            renderer.finish();
            let mut frame = screen.cost(&renderer);
            if snaps {
                let (width, height, rgba) = renderer.read(0).ok_or_else(|| anyhow::anyhow!("nothing was drawn"))?;
                for (name, value) in crate::inspect::png(width, height, rgba)?.as_object().into_iter().flatten() {
                    frame[name] = value.clone();
                }
            }
            Ok(frame)
        };
        let answered = match request.op.as_str() {
            "snap" => draw(screen),
            "replay" => screen.replay(&request, &mut draw),
            _ => screen.profile(&request, &mut draw),
        };
        renderer.destroy();
        reply(crate::inspect::answer(&request.id, answered))
    }

    /// The device's Back, on window `id`: what's open over its page shuts, or an app's window closes. The root window
    /// leaves Back with nothing open over it to the system. → whether it took it.
    pub fn back(&mut self, id: WindowId) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        if window.screen.as_mut().is_ok_and(Screen::back) {
            window.dirty = true;
            return true;
        }
        if window.root {
            return false;
        }
        self.close(id);
        true
    }

    /// Whether something's open over window `id`'s page (a sheet, a menu), which Back shuts first.
    pub fn covered(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.screen.as_ref().is_ok_and(|screen| screen.covered().is_some()))
    }

    /// Where its memory is, in bytes: the GPU's allocator (`Context::memory`, where its backend says), the atlas,
    /// and each window's, by its entrypoint, whether it's a panel (the launcher) and on screen, what its renderers hold
    /// (`Renderer::memory`) and what its JS held as it was last collected (`Screen::js_memory`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn memory(&self) -> serde_json::Value {
        let gpu = self.context.memory();
        let windows: Vec<serde_json::Value> = self
            .windows
            .values()
            .map(|window| {
                let renderers: Vec<serde_json::Value> = window.renderer.iter().map(Renderer::memory).collect();
                let screen = window.screen.as_ref().ok();
                serde_json::json!({
                    "entrypoint": window.tag["entrypoint"],
                    "panel": window.panel,
                    "on_screen": window.on_screen(),
                    "renderers": renderers,
                    "js": screen.map_or(0, Screen::js_memory),
                })
            })
            .collect();
        serde_json::json!({"gpu": gpu, "atlas": self.atlas.usage()["bytes"], "windows": windows})
    }
}
