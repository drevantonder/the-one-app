//! An app's JS, which talks with its host over two channels: what it draws, as the renderer's mutations
//! (core/ui/src/drawing-primitives/renderer.ts), and what it asks its client, MCP Apps' JSON-RPC (core/ui/src/calls.ts).
//! A native host runs it in QuickJS (`quickjs`); the web, in the browser's own engine, beside the renderer
//! (`platform::web`). Either is a `Thread`, read on the same clock as its window.


use std::sync::{Arc, Mutex};

use url::Url;
use web_time::Instant;

use crate::tree::Mutation;

#[cfg(not(target_arch = "wasm32"))]
mod quickjs;
#[cfg(not(target_arch = "wasm32"))]
pub use quickjs::{Thread, memory_limit};
#[cfg(all(feature = "fake", not(target_arch = "wasm32")))]
pub use quickjs::device_memory_is;
#[cfg(not(target_arch = "wasm32"))]
mod stack;
#[cfg(not(target_arch = "wasm32"))]
pub use stack::{Place, source_mapped, stack_files};
/// Which QuickJS runs a page, which the bytecode it compiled is kept for (`Thread::start`'s `cache`): another build of
/// this program on the same QuickJS loads it, and one on another compiles its pages again (renderer's build.rs).
pub const QUICKJS: &str = env!("QUICKJS");
#[cfg(target_arch = "wasm32")]
pub use crate::platform::web::Thread;

/// What the sandbox reads its clock from, in ms since it started: the real time, or, for output that's the same every
/// time, one that starts at a fixed time and moves only as it's told. Either is paused, stepped and sought by the
/// inspector (`crate::inspect`): paused, it moves only as it's told (`advance`). The app's JS thread and its window's
/// read the same one.
#[derive(Clone)]
pub struct Clock(Arc<Ticking>);

struct Ticking {
    /// What `Date` says at its start, in ms since 1970, for a fixed clock; the real date otherwise. The web's JS
    /// reads the browser's own `Date`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    epoch_ms: Option<f64>,
    /// The time zone a fixed clock shows times in, by its IANA name, where it isn't `UTC`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    zone: Option<String>,
    started: Instant,
    /// How far it's been moved on, beyond the real time since it started, and where it stopped, while it's paused.
    moved: Mutex<Moved>,
}

#[derive(Clone, Copy)]
struct Moved {
    ahead: f64,
    paused: Option<f64>,
}

impl Clock {
    pub fn real() -> Clock {
        Clock(Arc::new(Ticking { epoch_ms: None, zone: None, started: Instant::now(), moved: Mutex::new(Moved { ahead: 0.0, paused: None }) }))
    }

    /// 1 January 2026, 00:00 UTC, paused.
    pub fn fixed() -> Clock {
        Clock(Arc::new(Ticking { epoch_ms: Some(1_767_225_600_000.0), zone: None, started: Instant::now(), moved: Mutex::new(Moved { ahead: 0.0, paused: Some(0.0) }) }))
    }

    /// The same moment, shown in the time zone `zone` as a device in it would, by its IANA name.
    pub fn fixed_in(zone: &str) -> Clock {
        Clock(Arc::new(Ticking { zone: Some(zone.into()), ..Arc::into_inner(Clock::fixed().0).expect("one owner") }))
    }

    fn moved(&self) -> std::sync::MutexGuard<'_, Moved> {
        self.0.moved.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn elapsed(&self) -> f64 {
        self.0.started.elapsed().as_secs_f64() * 1000.0
    }

    /// When `instant` was, on this clock, or now while it's paused.
    pub fn at(&self, instant: Instant) -> f64 {
        let ago = Instant::now().saturating_duration_since(instant).as_secs_f64() * 1000.0;
        if self.paused() { self.now() } else { self.now() - ago }
    }

    pub fn now(&self) -> f64 {
        let moved = *self.moved();
        moved.paused.unwrap_or_else(|| self.elapsed() + moved.ahead)
    }

    pub fn paused(&self) -> bool {
        self.moved().paused.is_some()
    }

    pub fn pause(&self) {
        let now = self.now();
        self.moved().paused.get_or_insert(now);
    }

    pub fn resume(&self) {
        let elapsed = self.elapsed();
        let mut moved = self.moved();
        if let Some(at) = moved.paused.take() {
            moved.ahead = at - elapsed;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The language the sandbox's JS formats in (`navigator.language`): the host's, or for a fixed clock, whose output
    /// is the same everywhere, `en-US`.
    pub fn locale(&self) -> String {
        match self.0.epoch_ms {
            Some(_) => "en-US".into(),
            None => sys_locale::get_locale().unwrap_or_else(|| "en-US".into()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The time zone the sandbox's JS shows times in, by its IANA name: the host's, or for a fixed clock, `UTC`
    /// unless it was given one.
    pub fn time_zone(&self) -> String {
        match self.0.epoch_ms {
            Some(_) => self.0.zone.clone().unwrap_or_else(|| "UTC".into()),
            None => crate::intl::host_time_zone(),
        }
    }

    /// Moves it on by `ms`.
    pub fn advance(&self, ms: f64) {
        let mut moved = self.moved();
        match moved.paused.as_mut() {
            Some(at) => *at += ms,
            None => moved.ahead += ms,
        }
    }
}

/// Reads a module's source by its address.
pub type Files = Box<dyn FnMut(&Url) -> anyhow::Result<String> + Send>;

/// What the app's JS gave its host.
pub enum Posted {
    /// A mutation of its tree (`mutations`).
    Mutation(Mutation),
    /// A JSON-RPC message for its client (`host.post`), as JSON.
    Message(String),
    /// What it threw, as a browser's console logs an uncaught error: it carries on.
    Threw(String),
}

