//! An app's JS, in a QuickJS sandbox of its own (rquickjs), with the Web APIs a Worker has, the WinterTC Minimum Common
//! API's, but the network's (`fetch`): AWS LLRT's implementations of them, the sandbox's own clock, timers and
//! `console` (`prelude.js`), and `Intl` on ICU4X (`intl.js`, `intl.rs`); core/ui/src/types/globals.d.ts says
//! which. It talks with its host over two channels. What it draws is the renderer's (core/ui/src/drawing-primitives/renderer.ts):
//! `globalThis.mutations` is the host's functions it calls with each mutation of its tree, with its values as they are,
//! never JSON, as React Native's JSI calls its host, and the UI library's `ui.event` is called the same way with each
//! event. What it asks its client is MCP Apps' JSON-RPC (core/ui/src/calls.ts): `globalThis.host.post` and
//! `ui.message`. Its page's module
//! and what that imports are read by `Files`, and each runs as QuickJS's bytecode, compiled once and kept (`cache`), as
//! QuickJS's own `qjsc` compiles a program ahead of time. What the Web APIs answer with a promise runs on rquickjs's
//! async runtime, which nothing here waits on: each call into the sandbox runs its jobs until none is left.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use anyhow::anyhow;
use futures_lite::future::block_on;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::{Declared, WriteOptions};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Exception, Function, Module, Object, Persistent, Type, Value};
use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use sha2::{Digest, Sha256};
use url::Url;

use super::{Clock, Files, Posted};
use crate::tree::{Id, Kind, Mutation};

/// The sandbox's own globals (`prelude.js`), with the DOM's events (`event-target-shim.js`).
const PRELUDE: &str = include_str!("../prelude.js");
const EVENT_TARGET_SHIM: &str = include_str!("../event-target-shim.js");
/// Intl, as ECMA-402 says, on what intl.rs formats with.
const INTL: &str = include_str!("../intl.js");

/// The Web APIs that are LLRT's, as its own runtime adds them to a context.
fn web(ctx: &Ctx) -> rquickjs::Result<()> {
    llrt_exceptions::init(ctx)?;
    llrt_url::init(ctx)?;
    llrt_util::init(ctx)?;
    llrt_stream_web::init(ctx)?;
    llrt_buffer::init(ctx)?;
    llrt_crypto::init(ctx)
}

/// How much memory and stack an app's JS may have.
const MEMORY: usize = 64 << 20;
const STACK: usize = 1 << 20;

/// One app's JS, running.
/// Its fields drop in order: what's held in the runtime before the runtime.
struct Sandbox {
    /// The prelude's `due`, `fire` and `unhandled`.
    host: Persistent<rquickjs::Object<'static>>,
    context: AsyncContext,
    runtime: AsyncRuntime,
    posted: Rc<RefCell<Vec<Posted>>>,
    /// Promises rejected with nothing to handle them yet, and why, as QuickJS tells of them.
    rejected: Rc<RefCell<Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>>>,
    clock: Clock,
}

impl Sandbox {
    /// Starts the module at `page`, which a page's `<script type="module">` names, running it until it waits on
    /// something. It and its imports are read by `files`, each compiled once and kept in `cache`.
    /// QuickJS stops what it runs, as an uncatchable error, once `stop` is set: its window closed.
    fn start(page: &Url, files: Files, cache: Option<PathBuf>, clock: Clock, touch: bool, stop: Arc<AtomicBool>) -> anyhow::Result<Sandbox> {
        let runtime = AsyncRuntime::new()?;
        let rejected: Rc<RefCell<Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>>> = Rc::default();
        let to = rejected.clone();
        block_on(async {
            runtime.set_memory_limit(MEMORY).await;
            // Collected while it's idle (`collect`), not as it allocates.
            runtime.set_gc_threshold(MEMORY / 4).await;
            runtime.set_max_stack_size(STACK).await;
            let components = Arc::new(Mutex::new(HashSet::new()));
            runtime.set_loader(Resolve { components: components.clone() }, Modules { files, cache, components, styled: HashSet::new() }).await;
            runtime.set_interrupt_handler(Some(Box::new(move || stop.load(Ordering::Relaxed)))).await;
            // QuickJS tells of a rejected promise with no handler as it's rejected, and again if one is added after.
            runtime
                .set_host_promise_rejection_tracker(Some(Box::new(move |ctx, promise, reason, handled| {
                    let mut rejected = to.borrow_mut();
                    if handled {
                        rejected.retain(|(was, _)| was.clone().restore(&ctx).ok().as_ref() != Some(&promise));
                    } else {
                        rejected.push((Persistent::save(&ctx, promise), Persistent::save(&ctx, reason)));
                    }
                })))
                .await;
        });
        let context = block_on(AsyncContext::full(&runtime))?;
        let posted: Rc<RefCell<Vec<Posted>>> = Rc::default();
        let app = page.to_string();
        let (host, started) = block_on(context.with(|ctx| -> anyhow::Result<_> {
            let globals = ctx.globals();
            globals.set("mutations", mutations(&ctx, &posted)?)?;
            let to = posted.clone();
            globals.set("__post", Function::new(ctx.clone(), move |json: String| to.borrow_mut().push(Posted::Message(json)))?)?;
            let now = clock.clone();
            globals.set("__now", Function::new(ctx.clone(), move || now.now())?)?;
            globals.set("__log", Function::new(ctx.clone(), move |level: String, line: String| eprintln!("{app} {level}: {line}"))?)?;
            globals.set("structuredClone", rquickjs::prelude::Func::from(|ctx, value, options| crate::clone::structured_clone(&ctx, value, options)))?;
            run(&ctx, web(&ctx))?;
            let fixed = clock.0.epoch_ms.map_or_else(|| "null".into(), |epoch_ms| epoch_ms.to_string());
            let locale = serde_json::to_string(&clock.locale())?;
            // How many fingers its screen takes at once, as a browser says it (`navigator.maxTouchPoints`): a phone's
            // five, as iOS's and Android's browsers say, a computer's none.
            let touch_points = if touch { 5 } else { 0 };
            let host: rquickjs::Object = run(&ctx, ctx.eval(format!("const FIXED = {fixed}, LOCALE = {locale}, TOUCH_POINTS = {touch_points}, shim = (() => {{\n{EVENT_TARGET_SHIM}\n}})();\n{PRELUDE}")))?;
            run(&ctx, crate::intl::init(&ctx, &clock.time_zone()))?;
            run(&ctx, ctx.eval::<(), _>(INTL))?;
            let started = run(&ctx, Module::import(&ctx, page.as_str()))?;
            Ok((Persistent::save(&ctx, host), Persistent::save(&ctx, started)))
        }))?;
        let sandbox = Sandbox { host, context, runtime, posted, rejected, clock };
        sandbox.settle()?;
        // What its module threw as it ran, which its promise holds.
        block_on(sandbox.context.with(|ctx| {
            let started = run(&ctx, started.restore(&ctx))?;
            match started.result::<Value>() {
                Some(Err(error)) => run(&ctx, Err(error)),
                _ => Ok(()),
            }
        }))?;
        Ok(sandbox)
    }

    /// Tells the UI library what the person did (`ui.event`), or a message from its client (`ui.message`), as a
    /// plain object made directly, and runs it until it waits again.
    fn send(&self, to: &str, message: &serde_json::Value) -> anyhow::Result<()> {
        let called = block_on(self.context.with(|ctx| {
            let ui: rquickjs::Object = run(&ctx, ctx.globals().get("ui"))?;
            let function: Function = run(&ctx, ui.get(to))?;
            let message = run(&ctx, js(&ctx, message))?;
            run(&ctx, function.call::<_, ()>((rquickjs::function::This(ui), message)))
        }));
        // What it threw is its error, as a browser reports an event handler's, once what it queued has run.
        self.settle()?;
        called
    }

    /// When its next timer is due, in ms from now on its clock, if it has one.
    fn due(&self) -> Option<f64> {
        let due: f64 = block_on(self.context.with(|ctx| -> rquickjs::Result<f64> {
            let host = self.host.clone().restore(&ctx)?;
            host.get::<_, Function>("due")?.call(())
        }))
        .ok()?;
        due.is_finite().then_some(due - self.clock.now())
    }

    /// Each of its timers: when it's due, in ms from now (`in`), and how often an interval runs (`every`).
    fn timers(&self) -> Vec<serde_json::Value> {
        block_on(self.context.with(|ctx| -> rquickjs::Result<String> {
            let host = self.host.clone().restore(&ctx)?;
            host.get::<_, Function>("timers")?.call(())
        }))
        .ok()
        .and_then(|timers| serde_json::from_str(&timers).ok())
        .unwrap_or_default()
    }

    /// Whether it asked for a frame (`requestAnimationFrame`).
    fn framing(&self) -> bool {
        block_on(self.context.with(|ctx| -> rquickjs::Result<bool> {
            let host = self.host.clone().restore(&ctx)?;
            host.get::<_, Function>("framing")?.call(())
        }))
        .unwrap_or(false)
    }

    /// Runs what asked for a frame, told `time`, on its clock, as the frame's shown.
    fn frame(&self, time: f64) -> anyhow::Result<()> {
        let ran = block_on(self.context.with(|ctx| {
            let host = run(&ctx, self.host.clone().restore(&ctx))?;
            run(&ctx, host.get::<_, Function>("frame").and_then(|frame| frame.call::<_, ()>((time,))))
        }));
        self.settle()?;
        ran
    }

    /// Runs the timers that are due.
    fn fire(&self) -> anyhow::Result<()> {
        let fired = block_on(self.context.with(|ctx| {
            let host = run(&ctx, self.host.clone().restore(&ctx))?;
            run(&ctx, host.get::<_, Function>("fire").and_then(|fire| fire.call::<_, ()>(())))
        }));
        self.settle()?;
        fired
    }

    /// Runs its jobs, and what the Web APIs answer with a promise, until none is left, then tells it of each promise
    /// rejected that nothing handled (`unhandledrejection`), as a browser does once its microtasks have run.
    fn settle(&self) -> anyhow::Result<()> {
        loop {
            loop {
                match block_on(self.runtime.execute_pending_job()) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => block_on(error.0.with(|ctx| eprintln!("a job threw: {:?}", ctx.catch()))),
                }
            }
            let rejected = std::mem::take(&mut *self.rejected.borrow_mut());
            if rejected.is_empty() {
                return Ok(());
            }
            block_on(self.context.with(|ctx| {
                let told = (|| {
                    let list = rquickjs::Array::new(ctx.clone())?;
                    for (at, (promise, reason)) in rejected.into_iter().enumerate() {
                        let pair = rquickjs::Array::new(ctx.clone())?;
                        pair.set(0, promise.restore(&ctx)?)?;
                        pair.set(1, reason.restore(&ctx)?)?;
                        list.set(at, pair)?;
                    }
                    let host = self.host.clone().restore(&ctx)?;
                    host.get::<_, Function>("unhandled")?.call::<_, ()>((list,))
                })();
                run(&ctx, told)
            }))?;
        }
    }

    /// Collects what its JS no longer holds, as a browser does while nothing's happening, and puts off its next
    /// collection until it holds twice as much and 8 MB more, within its memory, so what it's sent doesn't wait on one:
    /// QuickJS's collector goes through all it holds at once, 15 to 25 ms for a long list's page.
    fn collect(&self) {
        block_on(async {
            self.runtime.run_gc().await;
            let held = usize::try_from(self.runtime.memory_usage().await.malloc_size).unwrap_or(0);
            self.runtime.set_gc_threshold((held * 2 + (8 << 20)).min(MEMORY * 3 / 4).max(held + (4 << 20))).await;
        })
    }

    /// What it has given its host since this was last asked.
    fn take(&self) -> Vec<Posted> {
        std::mem::take(&mut *self.posted.borrow_mut())
    }

}

/// One app's JS on a thread of its own, as React Native's runs off its UI thread: its window's thread sends it what
/// the person did and what its client said (`send`) and goes on drawing, and it sends back what that changed as one
/// batch a turn (`take`), which its window applies at once, waking that thread to (`on_post`). A window whose app is
/// busy still scrolls, animates and draws, and so does every other.
pub struct Thread {
    calls: mpsc::Sender<Call>,
    shared: Arc<Shared>,
    clock: Clock,
    /// How many calls it's been sent.
    sent: u64,
}

enum Call {
    /// `ui.event` or `ui.message`, with what it says.
    Send(&'static str, serde_json::Value),
    /// Runs the timers that are due, and looks again at when the next is, as its clock was moved or paused.
    Fire,
    /// A frame is shown, at this time on its clock: what asked for one runs.
    Frame(f64),
}

#[derive(Default)]
struct Shared {
    posted: Mutex<Posts>,
    /// Told as a batch comes, or the thread ends.
    turned: Condvar,
    /// What to call as a batch comes and none was waiting, and whether one is.
    wake: Mutex<Option<Box<dyn Fn() + Send>>>,
    waiting: AtomicBool,
    /// Its window closed: what's running stops.
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct Posts {
    posted: VecDeque<Posted>,
    /// How many calls it's run, whether its module has run (or why it didn't), and whether the thread ended.
    ran: u64,
    started: Option<Result<(), String>>,
    ended: bool,
    /// When its next timer is due, on its clock, and each of its timers (`Sandbox::timers`), as its last turn left them.
    due: Option<f64>,
    timers: Vec<serde_json::Value>,
    /// Whether it asked for a frame, as its last turn left it.
    framing: bool,
    /// When, on its clock, its last turn ended.
    turned: f64,
}

/// How long its JS is still before what it dropped is collected: a pause in scrolling or typing, as a browser's idle
/// time is.
const IDLE: Duration = Duration::from_millis(250);

/// How much stack its thread has: the sandbox's, and room for Rust's around it.
const THREAD_STACK: usize = STACK * 4;

impl Thread {
    /// Starts the module at `page` on a thread of its own, as `Sandbox::start` says, for a finger where `touch`.
    pub fn start(page: &Url, files: Files, cache: Option<PathBuf>, clock: Clock, touch: bool) -> anyhow::Result<Thread> {
        let (calls, queue) = mpsc::channel::<Call>();
        let shared = Arc::new(Shared::default());
        let (page, on, time) = (page.clone(), shared.clone(), clock.clone());
        std::thread::Builder::new().name(format!("js {}", page.host_str().unwrap_or_default())).stack_size(THREAD_STACK).spawn(move || {
            // Told as it ends, however it does, so nothing waits on it for ever.
            struct Ended(Arc<Shared>);
            impl Drop for Ended {
                fn drop(&mut self) {
                    self.0.post(|posts| posts.ended = true);
                }
            }
            let _ended = Ended(on.clone());
            let sandbox = match Sandbox::start(&page, files, cache, time.clone(), touch, on.stop.clone()) {
                Ok(sandbox) => sandbox,
                Err(error) => return on.post(|posts| posts.started = Some(Err(format!("{error:#}")))),
            };
            // Whether it's run something since it last collected.
            let (mut threw, mut ran, mut collect) = (Vec::new(), 0, true);
            loop {
                // A timer due now runs now, its clock paused or not.
                while sandbox.due().is_some_and(|due| due <= 0.0) {
                    threw.extend(sandbox.fire().err());
                    collect = true;
                }
                let (due, timers, framing) = (sandbox.due(), sandbox.timers(), sandbox.framing());
                let now = time.now();
                // It's started once what its module posted as it ran is here too, so its window shows that.
                on.post(|posts| {
                    posts.started = Some(Ok(()));
                    posts.posted.extend(sandbox.take());
                    posts.posted.extend(threw.drain(..).map(|error: anyhow::Error| Posted::Threw(format!("{error:#}"))));
                    posts.ran += std::mem::take(&mut ran);
                    posts.due = due.map(|due| now + due);
                    (posts.timers, posts.turned, posts.framing) = (timers, now, framing);
                });
                // Its next timer, by the real time, unless its clock is paused, or what it dropped collected once it's
                // been still a while.
                let timer = due.filter(|_| !time.paused()).map(|due| Duration::from_secs_f64(due.max(0.0) / 1000.0));
                let still = collect.then_some(IDLE);
                let first = match timer.into_iter().chain(still).min() {
                    Some(wait) => match queue.recv_timeout(wait) {
                        Ok(call) => Some(call),
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if still.is_some_and(|still| still <= wait) {
                                collect = false;
                                sandbox.collect();
                            }
                            None
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    },
                    None => match queue.recv() {
                        Ok(call) => Some(call),
                        Err(_) => return,
                    },
                };
                collect |= first.is_some();
                // Every call that's come runs in this turn, and what they changed goes back as one batch.
                for call in first.into_iter().chain(std::iter::from_fn(|| queue.try_recv().ok())) {
                    ran += 1;
                    match call {
                        Call::Send(to, message) => threw.extend(sandbox.send(to, &message).err()),
                        Call::Frame(time) => threw.extend(sandbox.frame(time).err()),
                        Call::Fire => {}
                    }
                }
            }
        })?;
        Ok(Thread { calls, shared, clock, sent: 0 })
    }

    /// Tells the UI library what the person did (`ui.event`), or a message from its client (`ui.message`), for its
    /// thread to run when it can.
    pub fn send(&mut self, to: &'static str, message: serde_json::Value) {
        self.call(Call::Send(to, message));
    }

    /// Has it run the timers that are due, as its clock was moved or paused, and wait again till its next.
    pub fn fire(&mut self) {
        self.call(Call::Fire);
    }

    /// A frame is shown at `time`, on its clock: what asked for one runs (`requestAnimationFrame`).
    pub fn frame(&mut self, time: f64) {
        self.call(Call::Frame(time));
    }

    /// Whether it asked for a frame, as its last turn left it.
    pub fn framing(&self) -> bool {
        self.shared.posts().framing
    }

    fn call(&mut self, call: Call) {
        if self.calls.send(call).is_ok() {
            self.sent += 1;
        }
    }

    /// What it's given its host since this was last asked: what it did with what it was sent, a batch at a time, and
    /// how many of the calls it was sent it had run by then (`sent`).
    pub fn take(&self) -> (Vec<Posted>, u64) {
        self.shared.waiting.store(false, Ordering::Release);
        let mut posts = self.shared.posts();
        (posts.posted.drain(..).collect(), posts.ran)
    }

    /// How many calls it's been sent: the last one's number.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Calls `wake`, from its thread, as it posts what its window hasn't taken: at once if it already has.
    pub fn on_post(&self, wake: impl Fn() + Send + 'static) {
        *self.shared.wake.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(wake));
        self.shared.waiting.store(false, Ordering::Release);
        self.shared.woken();
    }

    /// Whether its module has run (`Ok`), or why it didn't, once it's known.
    pub fn started(&self) -> Option<Result<(), String>> {
        self.shared.posts().started.clone()
    }

    /// Whether it has something it was sent to run yet, or hasn't started: what it then posts is on its way.
    pub fn busy(&self) -> bool {
        let posts = self.shared.posts();
        !posts.ended && (posts.started.is_none() || posts.ran < self.sent)
    }

    /// Whether it has run call number `call` (`sent`), or ended.
    pub fn ran(&self, call: u64) -> bool {
        let posts = self.shared.posts();
        posts.ended || (posts.started.is_some() && posts.ran >= call)
    }

    /// Waits until it has run all it was sent, or ended.
    pub fn wait(&self) {
        let mut posts = self.shared.posts();
        while !posts.ended && (posts.started.is_none() || posts.ran < self.sent) {
            posts = self.shared.turned.wait(posts).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// When its next timer is due, in ms from now on its clock, if it has one.
    pub fn due(&self) -> Option<f64> {
        Some(self.shared.posts().due? - self.clock.now())
    }

    /// Each of its timers: when it's due, in ms from now (`in`), and how often an interval runs (`every`).
    pub fn timers(&self) -> Vec<serde_json::Value> {
        let posts = self.shared.posts();
        let since = self.clock.now() - posts.turned;
        let mut timers = posts.timers.clone();
        for timer in &mut timers {
            timer["in"] = serde_json::json!(timer["in"].as_f64().unwrap_or_default() - since);
        }
        timers
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }
}

impl Drop for Thread {
    /// Its window closed: what it's running stops, and with no more to run, its thread ends.
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl Shared {
    fn posts(&self) -> std::sync::MutexGuard<'_, Posts> {
        self.posted.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Changes what's posted, and tells whoever waits on it, and its window, if it's not been told already.
    fn post(&self, change: impl FnOnce(&mut Posts)) {
        change(&mut self.posts());
        self.turned.notify_all();
        self.woken();
    }

    fn woken(&self) {
        if !self.waiting.swap(true, Ordering::AcqRel) {
            if let Some(wake) = self.wake.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                wake();
            }
        }
    }
}

/// The host's functions the renderer calls with each mutation (`Mutations` in core/ui/src/drawing-primitives/renderer.ts), each
/// kept to be applied once the sandbox's turn has run.
fn mutations<'js>(ctx: &Ctx<'js>, posted: &Rc<RefCell<Vec<Posted>>>) -> rquickjs::Result<Object<'js>> {
    let mutations = Object::new(ctx.clone())?;
    let to = posted.clone();
    let create = move |ctx: Ctx<'js>, id: Id, kind: String, value: Option<String>| -> rquickjs::Result<()> {
        let kind = Kind::deserialize(kind.as_str().into_deserializer())
            .map_err(|error: serde::de::value::Error| Exception::throw_type(&ctx, &error.to_string()))?;
        to.borrow_mut().push(Posted::Mutation(Mutation::Create { id, kind, value }));
        Ok(())
    };
    mutations.set("create", Function::new(ctx.clone(), create)?)?;
    let to = posted.clone();
    // A value that holds itself, or is nested past what's read, is the page's error, thrown to it, as JSON.stringify
    // throws one: the window and every other's go on.
    let set = move |ctx: Ctx<'js>, id: Id, name: String, value: Value<'js>| -> rquickjs::Result<()> {
        let value = data(&value, &mut Vec::new()).map_err(|why| Exception::throw_type(&ctx, why))?;
        to.borrow_mut().push(Posted::Mutation(Mutation::Set { id, name, value }));
        Ok(())
    };
    mutations.set("set", Function::new(ctx.clone(), set)?)?;
    let to = posted.clone();
    let insert = move |parent: Id, id: Id, before: Option<Id>| to.borrow_mut().push(Posted::Mutation(Mutation::Insert { parent, id, before }));
    mutations.set("insert", Function::new(ctx.clone(), insert)?)?;
    let to = posted.clone();
    mutations.set("remove", Function::new(ctx.clone(), move |parent: Id, id: Id| to.borrow_mut().push(Posted::Mutation(Mutation::Remove { parent, id })))?)?;
    let to = posted.clone();
    mutations.set("drop", Function::new(ctx.clone(), move |id: Id| to.borrow_mut().push(Posted::Mutation(Mutation::Drop { id })))?)?;
    let to = posted.clone();
    mutations.set("unsupported", Function::new(ctx.clone(), move |what: String| to.borrow_mut().push(Posted::Mutation(Mutation::Unsupported { what })))?)?;
    // It draws a `frame`, another app's page in a sandbox of its own (`view::frames`).
    mutations.set("frame", true)?;
    Ok(mutations)
}

/// How deep a value's arrays and objects are read at most, as each is read on the stack.
const NESTED_MOST: usize = 512;

/// A JS value as JSON's data model, read from it directly: a function, a symbol or `undefined` is `null`, and a number
/// that's whole is an integer, as JSON's text would have it. `within` is the arrays and objects it's in. → why it can't
/// be: it holds itself, or it's nested too deep.
fn data<'js>(value: &Value<'js>, within: &mut Vec<Value<'js>>) -> Result<serde_json::Value, &'static str> {
    use serde_json::Value as Data;
    if matches!(value.type_of(), Type::Array | Type::Object) {
        if within.contains(value) {
            return Err("cyclic object value");
        }
        if within.len() >= NESTED_MOST {
            return Err("object nested too deeply");
        }
    }
    Ok(match value.type_of() {
        Type::Bool => Data::Bool(value.as_bool().unwrap_or_default()),
        Type::Int => Data::from(value.as_int().unwrap_or_default()),
        Type::Float => {
            let number = value.as_float().unwrap_or_default();
            if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 { Data::from(number as i64) } else { serde_json::Number::from_f64(number).map_or(Data::Null, Data::Number) }
        }
        Type::String => value.as_string().and_then(|text| text.to_string().ok()).map_or(Data::Null, Data::String),
        Type::Array => {
            let Some(array) = value.as_array() else { return Ok(Data::Null) };
            within.push(value.clone());
            let items = array.iter::<Value>().map(|item| item.map_or(Ok(Data::Null), |item| data(&item, within))).collect::<Result<_, &'static str>>();
            within.pop();
            Data::Array(items?)
        }
        Type::Object => {
            let Some(object) = value.as_object() else { return Ok(Data::Null) };
            within.push(value.clone());
            let props = object
                .props::<String, Value>()
                .filter_map(Result::ok)
                .filter(|(_, value)| !value.is_undefined() && !value.is_function())
                .map(|(name, value)| Ok((name, data(&value, within)?)))
                .collect::<Result<_, &'static str>>();
            within.pop();
            Data::Object(props?)
        }
        _ => Data::Null,
    })
}

/// JSON's data as a JS value, made directly.
fn js<'js>(ctx: &Ctx<'js>, data: &serde_json::Value) -> rquickjs::Result<Value<'js>> {
    use serde_json::Value as Data;
    Ok(match data {
        Data::Null => Value::new_null(ctx.clone()),
        Data::Bool(bool) => Value::new_bool(ctx.clone(), *bool),
        Data::Number(number) => match number.as_i64().and_then(|int| i32::try_from(int).ok()) {
            Some(int) => Value::new_int(ctx.clone(), int),
            None => Value::new_float(ctx.clone(), number.as_f64().unwrap_or_default()),
        },
        Data::String(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
        Data::Array(items) => {
            let array = rquickjs::Array::new(ctx.clone())?;
            for (at, item) in items.iter().enumerate() {
                array.set(at, js(ctx, item)?)?;
            }
            array.into_value()
        }
        Data::Object(fields) => {
            let object = Object::new(ctx.clone())?;
            for (name, field) in fields {
                object.set(name.as_str(), js(ctx, field)?)?;
            }
            object.into_value()
        }
    })
}

/// What JS threw, as an error with its message and stack.
fn run<'js, T>(ctx: &Ctx<'js>, result: rquickjs::Result<T>) -> anyhow::Result<T> {
    result.catch(ctx).map_err(|error| anyhow!("{error}"))
}

/// Where an import is: beside what imports it, or from the network's root, where the UI library's renderer and
/// components (`/ui/client/web.js` and `ui.js`) are the ones built for a native host (`/ui/client/native/`). One of
/// another app's modules, as `component` imports it (`/<app>/client/<name>.js`), is kept in `components`, by where it
/// is, so it's drawn with its app's styles.
struct Resolve {
    components: Arc<Mutex<HashSet<String>>>,
}

impl Resolver for Resolve {
    fn resolve<'js>(&mut self, _: &Ctx<'js>, base: &str, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<String> {
        let refused = || rquickjs::Error::new_resolving(base, name);
        let mut url = Url::parse(name).or_else(|_| Url::parse(base).and_then(|base| base.join(name))).map_err(|_| refused())?;
        if let Some(file @ ("web.js" | "ui.js")) = url.path().strip_prefix("/ui/client/") {
            url.set_path(&format!("/ui/client/native/{file}"));
        } else if let ["", app, "client", file] = name.split('/').collect::<Vec<_>>()[..]
            && app != "ui"
            && file.ends_with(".js")
        {
            self.components.lock().expect("nothing panics holding it").insert(url.to_string());
        }
        Ok(url.into())
    }
}

/// Each module, read, and compiled once: its bytecode is kept by its address and what's in it. Another app's module
/// (`components`) is drawn with its app's styles, its `styles.json` beside it, loaded once for each folder (`styled`).
struct Modules {
    files: Files,
    cache: Option<PathBuf>,
    components: Arc<Mutex<HashSet<String>>>,
    styled: HashSet<Url>,
}

impl Modules {
    /// Loads the styles of the app whose module `url` is, if it's another app's and they aren't loaded yet.
    fn style(&mut self, url: &Url) -> anyhow::Result<()> {
        let table = url.join("styles.json")?;
        if !self.components.lock().expect("nothing panics holding it").contains(url.as_str()) || !self.styled.insert(table.clone()) {
            return Ok(());
        }
        let table = (self.files)(&table)?;
        crate::tokens::load_component(&table)?;
        crate::load_styles(&table)?;
        Ok(())
    }
}

impl Loader for Modules {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, Declared>> {
        let refused = |why: String| rquickjs::Error::new_loading_message(name, why);
        let url = Url::parse(name).map_err(|error| refused(error.to_string()))?;
        let source = (self.files)(&url).map_err(|error| refused(format!("{error:#}")))?;
        self.style(&url).map_err(|error| refused(format!("its styles: {error:#}")))?;
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
