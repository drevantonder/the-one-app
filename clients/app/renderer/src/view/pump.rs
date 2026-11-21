//! What its app's JS posts and what it's told: events, messages and the answers to what it asked, applied each turn (`pump`), and when it next has something to do.

use super::*;

impl Screen {
    /// Tells the app what the person did to node `id`, and takes what it changed.
    pub fn event(&mut self, id: Id, name: &str, value: Option<String>) {
        self.tell(id, name, json!({"value": value}));
    }

    /// Tells the app event `name` on node `id`, with what it says of it (`told`), when it happened on the clock the
    /// app's own is, as every DOM event has its `timeStamp`, and takes what it changed.
    pub(crate) fn tell(&mut self, id: Id, name: &str, told: Value) {
        let mut event = json!({"id": id, "name": name, "timeStamp": self.js.clock().now()});
        if let (Some(event), Value::Object(told)) = (event.as_object_mut(), told) {
            event.extend(told);
        }
        self.js.send("event", event);
    }

    /// Tells the app a message from its client: an answer to what it asked (`Asked`), or a notification.
    pub fn message(&mut self, message: &Value) {
        let mut message = message.clone();
        let asked = if message.get("method").is_none() { self.unanswered.remove(&message["id"].to_string()) } else { None };
        if asked.as_ref().is_some_and(|asked| asked.method == "ui/initialize") && message["result"].is_object() {
            self.place(&mut message);
        }
        self.js.send("message", message);
        if asked.is_some() {
            self.heard = self.js.sent();
        }
    }

    /// Adds to the client's answer to `ui/initialize` what only the window knows of where the page is, whether it
    /// answered at once (`Asked`) or later, as a browser's frame's client does.
    fn place(&mut self, answer: &mut Value) {
        answer["result"]["hostContext"]["containerDimensions"] = json!({"width": self.room.width, "height": self.room.height});
        answer["result"]["hostContext"]["reducedMotion"] = json!(self.reduced_motion);
        answer["result"]["hostContext"]["pressure"] = json!(self.pressure.name());
        answer["result"]["hostContext"]["safeAreaInsets"] = self.safe_area();
        answer["result"]["hostContext"]["visible"] = json!(self.visible);
        self.told = Some((self.room, self.reduced_motion, self.insets, self.pressure));
    }

    /// Logs what the app's JS threw, as a browser's console does, and carries on.
    fn threw(&mut self, error: String) {
        eprintln!("native app error: {error}");
        self.failed = Some(error);
    }

    /// How long until the screen has something to do by itself: an inspector's `settle` coming to its deadline, a
    /// frame of what moves, or the caret's blink. The window calls `fire` then. The app's timers are its JS thread's.
    pub fn due(&self) -> Option<Duration> {
        let now = Instant::now();
        let settle = self.settling.iter().map(|(until, _)| *until).chain(self.tapping.iter().map(inspected::Tapping::deadline)).map(|until| until.saturating_duration_since(now)).min();
        let frames = self.frames.values().filter_map(|child| child.screen.due());
        // For a harness, a picture decoded is there at its time on its clock (`Pictures::due`).
        let pictures = self.pictures.due().map(|ms| Duration::from_secs_f64(ms / 1000.0));
        let hover = self.hover_after.map(|at| Duration::from_secs_f64((at - self.js.clock().now()).max(0.0) / 1000.0));
        settle.into_iter().chain(self.blinks()).chain(self.finger_hold()).chain(frames).chain(pictures).chain(hover).min()
    }

    /// Whether its page, or a frame's in it, asked for the display's next frame (`requestAnimationFrame`).
    pub fn framing(&self) -> bool {
        self.js.framing() || self.late.as_ref().is_some_and(Late::holding) || self.frames.values().any(|child| child.screen.framing())
    }

    /// The display showed a frame `at`: what asked for one runs, told when, on its clock.
    pub fn frame(&mut self, at: Instant) {
        for child in self.frames.values_mut() {
            child.screen.frame(at);
        }
        if self.js.framing() {
            let time = crate::platform::time_at(self.js.clock(), at);
            self.js.frame(time);
        }
    }

    /// Applies what the app's JS has posted, and tells the `settle`s whose time has come.
    pub fn fire(&mut self) {
        for child in self.frames.values_mut() {
            child.screen.fire();
        }
        self.pump();
        self.tell_settled();
        if self.hover_after.is_some_and(|at| at <= self.js.clock().now()) {
            self.hover_after = None;
            if let Some(at) = self.pointer.filter(|_| !self.touch) {
                self.hover(at);
            }
        }
        self.finger_held();
    }

    /// Applies what the app's JS has posted since it was last asked, all at once, as it's one turn's or more, and
    /// answers what it asked that the host answers at once. → whether anything came.
    pub fn pump(&mut self) -> bool {
        let arrived = std::mem::take(&mut *self.arrived.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let fetched = !arrived.is_empty();
        for (src, data) in arrived {
            self.fetched(&src, data);
        }
        let (posted, heard) = self.js.take();
        // Where its page's batches are held back (`late`), each is applied once its turns have gone by, in order, as a
        // page slower than its window gets there.
        let posted = match self.late.as_mut() {
            None => {
                self.editing.heard = heard;
                posted
            }
            Some(late) => {
                // What ran and changed nothing is heard with the batch before it, or at once with none held.
                match late.held.back_mut() {
                    _ if !posted.is_empty() => late.held.push_back(((late.hold)(), posted, heard)),
                    Some((.., held)) => *held = heard,
                    None => self.editing.heard = heard,
                }
                let mut ready = Vec::new();
                while let Some((turns, ..)) = late.held.front_mut() {
                    if *turns > 0 {
                        *turns -= 1;
                        break;
                    }
                    let (_, batch, heard) = late.held.pop_front().expect("there's one");
                    ready.extend(batch);
                    self.editing.heard = heard;
                }
                ready
            }
        };
        let mut came = fetched || !posted.is_empty() || self.started.is_none() || self.pictures.take();
        #[cfg(not(target_arch = "wasm32"))]
        {
            came |= self.video_pump();
        }
        let chosen = std::mem::take(&mut *self.chosen_files.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        for (id, files) in chosen {
            self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "result": {"files": files_told(&files)}}));
            came = true;
        }
        let mut cant = None;
        for child in self.frames.values_mut() {
            came |= child.screen.pump();
            // A frame's page that can't be drawn here takes the page it's in to where it can, as one of its own would.
            if let Some(why) = child.screen.mismatched.clone().or_else(|| child.screen.didnt_start().map(str::to_owned)) {
                cant = Some(format!("its frame's page, {}, can't be drawn natively: {why}", child.src));
            }
        }
        if let Some(why) = cant {
            self.mismatched.get_or_insert(why);
        }
        if self.started.is_none() {
            self.started = self.js.started();
        }
        let appearance = self.theme.unwrap_or(self.appearance);
        // What's sent it here is what answers what it asked.
        let answering = self.js.sent();
        for posted in posted {
            match posted {
                Posted::Threw(error) => self.threw(error),
                // What comes after what this host can't draw, or refused, isn't drawn: the window says why in its place,
                // rather than draw a page that's no longer what it says.
                Posted::Mutation(_) if self.mismatched.is_some() => {}
                Posted::Mutation(mutation) => match {
                    #[cfg(not(target_arch = "wasm32"))]
                    self.video_mutation(&mutation);
                    self.tree.apply([mutation])
                } {
                    Err(error) => self.mismatched = Some(format!("{error:#}")),
                    Ok(()) => {
                        self.cost.mutations += 1;
                        self.changed();
                    }
                },
                Posted::Message(message) => {
                    let Ok(message) = serde_json::from_str::<Value>(&message) else { continue };
                    // How tall its content is, as an MCP App tells its host, which a frame's page is drawn as.
                    if message["method"] == "ui/notifications/size-changed" {
                        self.content_height = message["params"]["height"].as_f64().map(|height| height as f32);
                    }
                    if message["method"] == "notifications/one/over" {
                        self.over = message["params"]["over"] == true;
                        self.layer = message["params"]["layer"] == true;
                    }
                    let Some(asked) = self.asked.as_mut() else { continue };
                    let (id, method, tool) = (message.get("id").map(Value::to_string), message["method"].as_str().map(str::to_owned), message["params"]["name"].as_str().map(str::to_owned));
                    // What it shares goes to the device's share sheet; what it copies, or shares where there's none
                    // (`done`: "copied"), on the clipboard.
                    if let (Some(id), Some(asked @ ("one/copy" | "one/share")), Some(text)) = (message.get("id"), method.as_deref(), message["params"]["text"].as_str()) {
                        let result = if asked == "one/share" && self.device.share(text) {
                            json!({"done": "shared"})
                        } else {
                            self.clipboard.write(false, text);
                            if asked == "one/share" { json!({"done": "copied"}) } else { json!({}) }
                        };
                        self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "result": result}));
                        continue;
                    }
                    // The device's own picker of files, whose choice comes back as the page's next turn (`pump`); a device
                    // without one says so.
                    if let (Some(id), Some("one/files")) = (message.get("id"), method.as_deref()) {
                        let accept: Vec<String> = message["params"]["accept"].as_array().into_iter().flatten().filter_map(|kind| kind.as_str().map(str::to_owned)).collect();
                        let (chosen, wake, asked) = (self.chosen_files.clone(), self.wake.clone(), id.clone());
                        let done = Box::new(move |files: Vec<std::path::PathBuf>| {
                            chosen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((asked, files));
                            if let Some(wake) = wake {
                                wake();
                            }
                        });
                        if !self.device.open_files(&accept, message["params"]["multiple"] == true, done) {
                            self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "This device has no file picker"}}));
                        }
                        continue;
                    }
                    // A haptic plays where the device has them, and a device without them says so, as a client does.
                    if let (Some(id), Some("one/haptic")) = (message.get("id"), method.as_deref()) {
                        let kind = message["params"]["kind"].as_str().unwrap_or_default();
                        let answer = if self.device.haptic(kind) {
                            json!({"jsonrpc": "2.0", "id": id, "result": {}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "This device plays no haptics"}})
                        };
                        self.js.send("message", answer);
                        continue;
                    }
                    match asked(message, appearance) {
                        // What only the window knows of where the page is, which it adds to its client's answer.
                        Some(mut answer) if method.as_deref() == Some("ui/initialize") && answer["result"].is_object() => {
                            self.place(&mut answer);
                            self.js.send("message", answer);
                        }
                        Some(answer) => self.js.send("message", answer),
                        None => {
                            if let (Some(id), Some(method)) = (id, method) {
                                self.unanswered.insert(id, inspect::Asked { method, tool, since: Instant::now() });
                            }
                        }
                    }
                }
            }
        }
        if self.js.sent() > answering {
            self.heard = self.js.sent();
        }
        // What was let go springs to where its page now puts it, once its page has run what told it so.
        if let Some((id, call)) = self.letting_go
            && self.editing.heard >= call
        {
            self.letting_go = None;
            self.transitions.answered(id);
            self.changed();
        }
        if came {
            // What says `autofocus` takes the keyboard as it says so, as an active menu item does.
            let wants: Vec<Id> = self.tree.autofocused().collect();
            self.focus.autofocused.retain(|id| wants.contains(id));
            if let Some(id) = wants.into_iter().filter(|id| !self.focus.autofocused.contains(id)).max() {
                self.focus.autofocused.insert(id);
                self.focus(Some(id));
                self.focus.reveal = Some(id);
            }
            self.sync_fields();
            self.sync_frames();
            #[cfg(not(target_arch = "wasm32"))]
            self.sync_videos();
        }
        // The inspector's answers wait for what they did to have run.
        if !self.replies.is_empty() && !self.busy() {
            for (answer, reply) in std::mem::take(&mut self.replies) {
                reply(answer);
            }
        }
        self.tell_settled();
        came
    }

    /// The picture at `src`, as its page names it, fetched by its window: as a `data:` address, or none where it wasn't
    /// found.
    pub fn fetched(&mut self, src: &str, data: Option<String>) {
        self.fetching.remove(src);
        self.fetched.insert(src.to_owned(), data);
    }
}

/// Files the device gave a page, as it's told them: each one's name, and its bytes as base64, the only way a file
/// crosses its JSON channel. One that can't be read is left out, saying why.

pub(crate) fn files_told(files: &[std::path::PathBuf]) -> Vec<Value> {
    let told = |file: &std::path::PathBuf| -> anyhow::Result<Value> {
        let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(json!({"name": name, "body": data_encoding::BASE64.encode(&std::fs::read(file)?)}))
    };
    files.iter().filter_map(|file| told(file).map_err(|error| log::warn!("{} isn't given to its page: {error:#}", file.display())).ok()).collect()
}
