//! A page's JS as its screen reads it (`Thread`), and the channel each page's calls and messages go through.

use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use super::{call, call_with, soon};
use crate::js::{Clock, Posted};

thread_local! {
    /// The JS of the page, and of each frame's page by its frame's node (`Thread`), as its screen hears it.
    static CHANNELS: RefCell<std::collections::HashMap<Option<crate::tree::Id>, Channel>> = RefCell::new(std::collections::HashMap::new());
}

/// What a page's JS gave its host since the screen last took it, how many of the calls the screen sent it it's run,
/// whether its module ran (or why not), whether it asked for a frame (a frame's page, whose `requestAnimationFrame`
/// is the screen's, as a native host's is), what wakes the screen as more comes, and what the screen sent it that it
/// hasn't been called with yet: `ui.event` or `ui.message`.
#[derive(Default)]
pub(super) struct Channel {
    pub(super) posted: Vec<Posted>,
    pub(super) ran: u64,
    pub(super) started: Option<Result<(), String>>,
    pub(super) framing: bool,
    wake: Option<std::rc::Rc<dyn Fn()>>,
    woken: bool,
    outbox: VecDeque<(&'static str, Value)>,
}

/// Does `change` to the channel of the page in frame `frame`, or the frame's own page's with none.
pub(super) fn channel<T: Default>(frame: Option<crate::tree::Id>, change: impl FnOnce(&mut Channel) -> T) -> T {
    CHANNELS.with_borrow_mut(|channels| channels.get_mut(&frame).map(change).unwrap_or_default())
}

/// Calls each page with what its screen sent it, in order, once its module has run, as a native host's JS thread runs
/// each call it's sent: the frame's own page here, and a frame's page in its own frame (`frameCall`), which says what
/// it's run as it says what it gave (`frame_posted`).
pub(super) fn deliver() {
    let frames: Vec<Option<crate::tree::Id>> = CHANNELS.with_borrow(|channels| channels.iter().filter(|(_, channel)| matches!(channel.started, Some(Ok(())))).map(|(frame, _)| *frame).collect());
    for frame in frames {
        let mut ran = false;
        while let Some((to, message)) = channel(frame, |channel| channel.outbox.pop_front()) {
            let message = js_sys::JSON::parse(&message.to_string()).unwrap_or(JsValue::NULL);
            let Some(node) = frame else {
                let ui = js_sys::Reflect::get(&js_sys::global(), &"ui".into()).unwrap_or(JsValue::UNDEFINED);
                if let Ok(function) = js_sys::Reflect::get(&ui, &to.into()).and_then(|function| function.dyn_into::<js_sys::Function>())
                    && let Err(error) = function.call1(&ui, &message)
                {
                    channel(None, |channel| channel.posted.push(Posted::Threw(format!("{error:?}"))));
                }
                channel(None, |channel| channel.ran += 1);
                ran = true;
                continue;
            };
            call_with("frameCall", &[JsValue::from(node as f64), to.into(), message]);
        }
        if ran {
            wake(frame);
        }
    }
}

/// Wakes the screen to take what a page gave it and what it's run, once until it has (`Thread::take`).
pub(super) fn wake(frame: Option<crate::tree::Id>) {
    let wake = channel(frame, |channel| (!std::mem::replace(&mut channel.woken, true)).then(|| channel.wake.clone()).flatten());
    if let Some(wake) = wake {
        wake();
    }
}

/// A page's JS, as its screen reads it: the browser's own, which `globalThis.mutations` and `host.post` queue
/// what it gives, and which is called with what it's sent between the windows' turns (`deliver`). The frame's own
/// page's timers and frames are the browser's. A frame's page (`frame`) runs in a frame of its own (renderer.js), as a
/// native host's runs on a thread of its own: what it gives comes from there, and its frames are the screen's.
pub struct Thread {
    clock: Clock,
    sent: u64,
    frame: Option<crate::tree::Id>,
}

impl Thread {
    pub fn new(clock: Clock) -> Thread {
        Thread::of(clock, None)
    }

    /// The JS of the page frame `node` shows.
    pub(super) fn of(clock: Clock, frame: Option<crate::tree::Id>) -> Thread {
        CHANNELS.with_borrow_mut(|channels| channels.insert(frame, Channel::default()));
        Thread { clock, sent: 0, frame }
    }

    pub fn send(&mut self, to: &'static str, message: Value) {
        channel(self.frame, |channel| channel.outbox.push_back((to, message)));
        self.sent += 1;
        soon();
    }

    pub fn fire(&mut self) {}

    /// A frame's page is told the display showed a frame, as it asked: what asked for one runs.
    pub fn frame(&mut self, time: f64) {
        if let Some(node) = self.frame {
            channel(self.frame, |channel| channel.framing = false);
            call_with("frameFrame", &[JsValue::from(node as f64), JsValue::from(time)]);
        }
    }

    pub fn framing(&self) -> bool {
        channel(self.frame, |channel| channel.framing)
    }

    /// Nothing: the browser runs the page's JS, beside the renderer, and its own Performance panel times it.
    pub fn script(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    pub fn take(&self) -> (Vec<Posted>, u64) {
        channel(self.frame, |channel| {
            channel.woken = false;
            (std::mem::take(&mut channel.posted), channel.ran)
        })
    }

    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Whether it's posted what its window hasn't taken yet (`take`).
    pub fn has_posted(&self) -> bool {
        channel(self.frame, |channel| !channel.posted.is_empty())
    }

    /// Nothing seeds a browser's page's thread.
    pub fn holding(&self) -> Option<f64> {
        None
    }

    /// What it's sent always tells it something.
    pub fn told(&self) -> u64 {
        self.sent
    }

    pub fn on_post(&self, wake: impl Fn() + Send + 'static) {
        channel(self.frame, |channel| {
            channel.wake = Some(std::rc::Rc::new(wake));
            channel.woken = false;
        });
        self::wake(self.frame);
    }

    pub fn started(&self) -> Option<Result<(), String>> {
        channel(self.frame, |channel| channel.started.clone())
    }

    /// Never: a page the browser runs that crashes is its browser's to say (Chrome's "Aw, Snap!").
    pub fn crashed(&self) -> Option<String> {
        None
    }

    pub fn busy(&self) -> bool {
        channel(self.frame, |channel| channel.started.is_none() || channel.ran < self.sent)
    }

    /// A browser's JS never ends.
    pub fn ended(&self) -> bool {
        false
    }

    /// A browser's JS can't be waited on: what it was sent runs once this turn ends.
    pub fn wait(&self) {}

    /// A browser collects its JS itself, as it sees fit.
    pub fn collect(&mut self) {}

    /// None that it says: what a browser's JS holds is its own (`performance.memory` is Chrome's alone).
    pub fn memory(&self) -> usize {
        0
    }

    pub fn due(&self) -> Option<f64> {
        None
    }

    pub fn timers(&self) -> Vec<Value> {
        Vec::new()
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }
}

/// A frame's page that went: its frame goes, and its client forgets it.
impl Drop for Thread {
    fn drop(&mut self) {
        CHANNELS.with_borrow_mut(|channels| channels.remove(&self.frame));
        if let Some(node) = self.frame {
            call("unframe", &JsValue::from(node as f64));
        }
    }
}
