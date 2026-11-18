//! An app's JS, in a QuickJS sandbox of its own (rquickjs), with no Web APIs: only the language, its timers and
//! `console` (core/ui/src/strict/types/globals.d.ts), and how it talks with its host (`Host` and `Sandbox` in
//! core/ui/src/strict/protocol.ts): `globalThis.host` takes its ops and what it posts its client, and the UI library
//! gives `globalThis.ui`, which takes what the person did and the client's messages. Its page's module and what that
//! imports are read by `Files`, and each runs as QuickJS's bytecode, compiled once and kept (`cache`), as QuickJS's
//! own `qjsc` compiles a program ahead of time.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::anyhow;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::{Declared, WriteOptions};
use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Runtime};
use sha2::{Digest, Sha256};
use url::Url;

/// The sandbox's globals, before the app's own code runs. `__update`, `__post`, `__now` and `__log` are the host's. A
/// fixed clock (`FIXED`, in ms since 1970, or `null`) makes `Date`, `performance.now` and `Math.random` the same on
/// every run.
const PRELUDE: &str = r#"
const timers = new Map()
let nextTimer = 1
globalThis.host = {
  update: (update) => __update(JSON.stringify(update)),
  post: (message) => __post(JSON.stringify(message)),
}
globalThis.queueMicrotask = (task) => void Promise.resolve().then(task)
globalThis.setTimeout = (callback, delay = 0, ...args) => {
  const id = nextTimer++
  timers.set(id, { due: __now() + Math.max(0, delay), run: () => callback(...args) })
  return id
}
globalThis.clearTimeout = (id) => void timers.delete(id)
globalThis.setInterval = (callback, delay = 0, ...args) => {
  const id = nextTimer++
  const every = Math.max(1, delay)
  const run = () => {
    timers.set(id, { due: __now() + every, every, run })
    callback(...args)
  }
  timers.set(id, { due: __now() + every, every, run })
  return id
}
globalThis.clearInterval = globalThis.clearTimeout
const say = (level) => (...parts) => __log(level, parts.map((part) => typeof part === "string" ? part : part instanceof Error ? `${part}\n${part.stack}` : JSON.stringify(part) ?? String(part)).join(" "))
globalThis.console = { log: say("log"), info: say("info"), debug: say("debug"), warn: say("warn"), error: say("error") }
const START = __now()
if (FIXED !== null) {
  const RealDate = Date
  globalThis.Date = class extends RealDate {
    constructor(...args) {
      if (args.length) super(...args)
      else super(FIXED + __now() - START)
    }
    static now() { return FIXED + __now() - START }
  }
  let seed = 0x9e3779b9
  Math.random = () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
// The host's own: when the next timer is due, each timer, and running those that are.
;({
  due: () => Math.min(...[...timers.values()].map((timer) => timer.due)),
  timers: () => JSON.stringify([...timers.values()].map((timer) => ({ in: timer.due - __now(), every: timer.every }))),
  fire: () => {
    const now = __now()
    for (const [id, timer] of [...timers]) {
      if (timer.due > now || !timers.has(id)) continue
      timers.delete(id)
      timer.run()
    }
  },
})
"#;

/// How much memory and stack an app's JS may have.
const MEMORY: usize = 64 << 20;
const STACK: usize = 1 << 20;

/// What the sandbox reads its clock from, in ms since it started: the real time, or, for output that's the same every
/// time, one that starts at a fixed time and moves only as it's told. Either is paused, stepped and sought by the
/// inspector (`crate::inspect`): paused, it moves only as it's told (`advance`).
#[derive(Clone)]
pub struct Clock(Rc<Ticking>);

struct Ticking {
    /// What `Date` says at its start, in ms since 1970, for a fixed clock; the real date otherwise.
    epoch_ms: Option<f64>,
    started: std::time::Instant,
    /// How far it's been moved on, beyond the real time since it started.
    ahead: Cell<f64>,
    /// Where it stopped, while it's paused.
    paused: Cell<Option<f64>>,
}

impl Clock {
    pub fn real() -> Clock {
        Clock(Rc::new(Ticking { epoch_ms: None, started: std::time::Instant::now(), ahead: Cell::new(0.0), paused: Cell::new(None) }))
    }

    /// 1 January 2026, 00:00 UTC, paused.
    pub fn fixed() -> Clock {
        Clock(Rc::new(Ticking { epoch_ms: Some(1_767_225_600_000.0), started: std::time::Instant::now(), ahead: Cell::new(0.0), paused: Cell::new(Some(0.0)) }))
    }

    pub fn now(&self) -> f64 {
        self.0.paused.get().unwrap_or_else(|| self.0.started.elapsed().as_secs_f64() * 1000.0 + self.0.ahead.get())
    }

    pub fn paused(&self) -> bool {
        self.0.paused.get().is_some()
    }

    pub fn pause(&self) {
        if !self.paused() {
            self.0.paused.set(Some(self.now()));
        }
    }

    pub fn resume(&self) {
        if let Some(at) = self.0.paused.take() {
            self.0.ahead.set(at - self.0.started.elapsed().as_secs_f64() * 1000.0);
        }
    }

    /// Moves it on by `ms`.
    pub fn advance(&self, ms: f64) {
        match self.0.paused.get() {
            Some(at) => self.0.paused.set(Some(at + ms)),
            None => self.0.ahead.set(self.0.ahead.get() + ms),
        }
    }
}

/// Reads a module's source by its address.
pub type Files = Box<dyn FnMut(&Url) -> anyhow::Result<String>>;

/// What the app's JS gave its host.
pub enum Posted {
    /// A batch of ops (`host.update`), as JSON.
    Update(String),
    /// A JSON-RPC message for its client (`host.post`), as JSON.
    Message(String),
}

/// One app's JS, running.
/// Its fields drop in order: what's held in the runtime before the runtime.
pub struct Sandbox {
    /// The prelude's `due` and `fire`.
    timers: rquickjs::Persistent<rquickjs::Object<'static>>,
    context: Context,
    _runtime: Runtime,
    posted: Rc<RefCell<Vec<Posted>>>,
    clock: Clock,
}

impl Sandbox {
    /// Starts the module at `page`, which a page's `<script type="module">` names, running it until it waits on
    /// something. It and its imports are read by `files`, each compiled once and kept in `cache`.
    pub fn start(page: &Url, files: Files, cache: Option<PathBuf>, clock: Clock) -> anyhow::Result<Sandbox> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(MEMORY);
        runtime.set_max_stack_size(STACK);
        // A promise nothing handles is an error a browser reports (`unhandledrejection`), where QuickJS says nothing.
        let unhandled = page.to_string();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(move |_, _, reason, handled| {
            if !handled {
                eprintln!("{unhandled} error: unhandled rejection: {}", reason.get::<rquickjs::Coerced<String>>().map_or_else(|_| "?".into(), |reason| reason.0));
            }
        })));
        runtime.set_loader(Resolve, Modules { files, cache });
        let context = Context::full(&runtime)?;
        let posted: Rc<RefCell<Vec<Posted>>> = Rc::default();
        let app = page.to_string();
        let timers = context.with(|ctx| -> anyhow::Result<_> {
            let globals = ctx.globals();
            let to = posted.clone();
            globals.set("__update", Function::new(ctx.clone(), move |json: String| to.borrow_mut().push(Posted::Update(json)))?)?;
            let to = posted.clone();
            globals.set("__post", Function::new(ctx.clone(), move |json: String| to.borrow_mut().push(Posted::Message(json)))?)?;
            let now = clock.clone();
            globals.set("__now", Function::new(ctx.clone(), move || now.now())?)?;
            globals.set("__log", Function::new(ctx.clone(), move |level: String, line: String| eprintln!("{app} {level}: {line}"))?)?;
            let fixed = clock.0.epoch_ms.map_or_else(|| "null".into(), |epoch_ms| epoch_ms.to_string());
            let timers: rquickjs::Object = run(&ctx, ctx.eval(format!("const FIXED = {fixed};\n{PRELUDE}")))?;
            let started = run(&ctx, Module::import(&ctx, page.as_str()))?;
            while ctx.execute_pending_job() {}
            // What its module threw as it ran, which its promise holds.
            if let Some(Err(error)) = started.result::<rquickjs::Value>() {
                run(&ctx, Err(error))?;
            }
            Ok(rquickjs::Persistent::save(&ctx, timers))
        })?;
        Ok(Sandbox { timers, context, _runtime: runtime, posted, clock })
    }

    /// Tells the UI library what the person did (`ui.event`), or a message from its client (`ui.message`), and runs
    /// it until it waits again.
    pub fn send(&self, to: &str, message: &serde_json::Value) -> anyhow::Result<()> {
        self.context.with(|ctx| {
            let ui: rquickjs::Object = run(&ctx, ctx.globals().get("ui"))?;
            let function: Function = run(&ctx, ui.get(to))?;
            let message: rquickjs::Value = run(&ctx, ctx.json_parse(message.to_string()))?;
            run(&ctx, function.call::<_, ()>((rquickjs::function::This(ui), message)))?;
            while ctx.execute_pending_job() {}
            Ok(())
        })
    }

    /// When its next timer is due, in ms from now on its clock, if it has one.
    pub fn due(&self) -> Option<f64> {
        let due: f64 = self.context.with(|ctx| -> rquickjs::Result<f64> {
            let timers = self.timers.clone().restore(&ctx)?;
            timers.get::<_, Function>("due")?.call(())
        }).ok()?;
        due.is_finite().then_some(due - self.clock.now())
    }

    /// Each of its timers: when it's due, in ms from now (`in`), and how often an interval runs (`every`).
    pub fn timers(&self) -> Vec<serde_json::Value> {
        self.context
            .with(|ctx| -> rquickjs::Result<String> {
                let timers = self.timers.clone().restore(&ctx)?;
                timers.get::<_, Function>("timers")?.call(())
            })
            .ok()
            .and_then(|timers| serde_json::from_str(&timers).ok())
            .unwrap_or_default()
    }

    /// Runs the timers that are due.
    pub fn fire(&self) -> anyhow::Result<()> {
        self.context.with(|ctx| {
            let timers = run(&ctx, self.timers.clone().restore(&ctx))?;
            run(&ctx, timers.get::<_, Function>("fire").and_then(|fire| fire.call::<_, ()>(())))?;
            while ctx.execute_pending_job() {}
            Ok(())
        })
    }

    /// What it has given its host since this was last asked.
    pub fn take(&self) -> Vec<Posted> {
        std::mem::take(&mut *self.posted.borrow_mut())
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }
}

/// What JS threw, as an error with its message and stack.
fn run<'js, T>(ctx: &Ctx<'js>, result: rquickjs::Result<T>) -> anyhow::Result<T> {
    result.catch(ctx).map_err(|error| anyhow!("{error}"))
}

/// Where an import is: beside what imports it, or from the network's root, where the UI library's renderer and
/// components (`/ui/client/web.js` and `ui.js`) are the ones built for a native host (`/ui/client/native/`).
struct Resolve;

impl Resolver for Resolve {
    fn resolve<'js>(&mut self, _: &Ctx<'js>, base: &str, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<String> {
        let refused = || rquickjs::Error::new_resolving(base, name);
        let mut url = Url::parse(name).or_else(|_| Url::parse(base).and_then(|base| base.join(name))).map_err(|_| refused())?;
        if let Some(file @ ("web.js" | "ui.js")) = url.path().strip_prefix("/ui/client/") {
            url.set_path(&format!("/ui/client/native/{file}"));
        }
        Ok(url.into())
    }
}

/// Each module, read, and compiled once: its bytecode is kept by its address and what's in it.
struct Modules {
    files: Files,
    cache: Option<PathBuf>,
}

impl Loader for Modules {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, Declared>> {
        let refused = |why: String| rquickjs::Error::new_loading_message(name, why);
        let url = Url::parse(name).map_err(|error| refused(error.to_string()))?;
        let source = (self.files)(&url).map_err(|error| refused(format!("{error:#}")))?;
        let kept = self.cache.as_ref().map(|cache| {
            let mut key = Sha256::new();
            key.update(name.as_bytes());
            key.update([0]);
            key.update(source.as_bytes());
            cache.join(format!("{}.qjsc", data_encoding::HEXLOWER.encode(&key.finalize())))
        });
        if let Some(bytecode) = kept.as_ref().and_then(|kept| std::fs::read(kept).ok()) {
            // Bytecode this program compiled itself, from these very bytes at this very address.
            if let Ok(module) = unsafe { Module::load(ctx.clone(), &bytecode) } {
                return Ok(module);
            }
        }
        let module = Module::declare(ctx.clone(), name, source)?;
        if let (Some(kept), Ok(bytecode)) = (kept, module.write(WriteOptions::default())) {
            let _ = std::fs::create_dir_all(kept.parent().expect("it's in the cache"));
            let _ = std::fs::write(kept, bytecode);
        }
        Ok(module)
    }
}
