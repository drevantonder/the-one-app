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
use std::time::{Duration, Instant};

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

/// How much memory a page's JS may have: a sixteenth of its device's, from 256 MB to 2 GB, as V8 sizes its heap by its
/// device's memory and iOS an app's: 256 MB on a phone of 4 GB, 1 GB on a computer of 16. It's for a page that runs
/// away, not a budget: a page holds 4 to 25 MB (21 Nov: Telemetry's, with a few hundred alerts, 22 MB), and a refresh
/// briefly twice that. Several pages share the app's process (the launcher's and each window's), so it's a share a few
/// can reach at once and the app still keep within what its system lets it have (iOS stops an app at about half its
/// device's memory). A quarter of it is told to Telemetry (the client's `memory`), and at three quarters its page is
/// warned (`notifications/one/memory_warning`), as iOS's `didReceiveMemoryWarning` warns an app.
pub fn memory_limit() -> usize {
    static MEMORY: std::sync::LazyLock<usize> = std::sync::LazyLock::new(device_memory);
    let memory = match DEVICE.load(Ordering::Relaxed) {
        0 => *MEMORY,
        bytes => bytes,
    };
    (memory / 16).clamp(256 << 20, 2 << 30)
}

/// The memory of the device a harness stands for, where it isn't this machine's (`device_memory_is`).
static DEVICE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Pages have the memory they'd have on a device with `bytes` of it, as a phone of 4 GB: for a harness, so a page that
/// runs away runs out as soon as it would there.
#[cfg(feature = "fake")]
pub fn device_memory_is(bytes: usize) {
    DEVICE.store(bytes, Ordering::Relaxed);
}

/// The device's memory, in bytes, as its system says it; 0 where it doesn't.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn device_memory() -> usize {
    let (pages, size) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
    usize::try_from(pages).unwrap_or(0).saturating_mul(usize::try_from(size).unwrap_or(0))
}

#[cfg(target_vendor = "apple")]
fn device_memory() -> usize {
    let (mut bytes, mut size) = (0u64, std::mem::size_of::<u64>());
    let read = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut bytes).cast(), &raw mut size, std::ptr::null_mut(), 0) };
    if read == 0 { usize::try_from(bytes).unwrap_or(usize::MAX) } else { 0 }
}

#[cfg(windows)]
fn device_memory() -> usize {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&raw mut status) }.map_or(0, |()| usize::try_from(status.ullTotalPhys).unwrap_or(usize::MAX))
}

/// How much stack an app's JS may have.
const STACK: usize = 1 << 20;

/// What a node of a page's tree costs its window, in bytes: the node, its layout, styles and text as drawn. 20 Nov:
/// 100,000 nodes (a box holding a line of text, each) took the headless harness's window 323 MB more.
const NODE: usize = 3 << 10;

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
    /// Whether it's been warned it holds near all the memory it may have (`collect`).
    warned: std::cell::Cell<bool>,
    /// What it threw since this was last taken (`threw`): what a call into it threw, and what threw past a timer, a
    /// frame, a listener or a job, or a promise nothing handled, which it carried on from (`__threw`).
    threw: Rc<RefCell<Vec<anyhow::Error>>>,
    /// How many nodes of its tree it's made and not dropped (`mutations`), and how much memory its JS may have as they
    /// were last counted (`hold`).
    nodes: Rc<std::cell::Cell<usize>>,
    limit: std::cell::Cell<usize>,
}

impl Sandbox {
    /// Starts the module at `page`, which a page's `<script type="module">` names, running it until it waits on
    /// something. It and its imports are read by `files`, each compiled once and kept in `cache`.
    /// QuickJS stops what it runs, as an uncatchable error, once `stop` is set: its window closed.
    fn start(page: &Url, files: Files, cache: Option<PathBuf>, clock: Clock, touch: bool, stop: Arc<AtomicBool>) -> anyhow::Result<Sandbox> {
        let began = Instant::now();
        let loads: Rc<RefCell<Loads>> = Rc::default();
        let runtime = AsyncRuntime::new()?;
        let rejected: Rc<RefCell<Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>>> = Rc::default();
        let to = rejected.clone();
        block_on(async {
            runtime.set_memory_limit(memory_limit()).await;
            // Collected while it's idle (`collect`), not as it allocates.
            runtime.set_gc_threshold(16 << 20).await;
            runtime.set_max_stack_size(STACK).await;
            let components = Arc::new(Mutex::new(HashSet::new()));
            runtime.set_loader(Resolve { components: components.clone() }, Modules { files, cache, components, styled: HashSet::new(), loads: loads.clone() }).await;
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
        let threw: Rc<RefCell<Vec<anyhow::Error>>> = Rc::default();
        let nodes: Rc<std::cell::Cell<usize>> = Rc::default();
        let app = page.to_string();
        let (host, started, said) = block_on(context.with(|ctx| -> anyhow::Result<_> {
            let globals = ctx.globals();
            globals.set("mutations", mutations(&ctx, &posted, &nodes)?)?;
            let to = threw.clone();
            globals.set("__threw", Function::new(ctx.clone(), move |error: Value| to.borrow_mut().push(uncaught(error)))?)?;
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
            let preluded = began.elapsed();
            run(&ctx, crate::intl::init(&ctx, &clock.time_zone()))?;
            run(&ctx, ctx.eval::<(), _>(INTL))?;
            let intl = began.elapsed() - preluded;
            let started = run(&ctx, Module::import(&ctx, page.as_str()))?;
            // Where its start went, in its log: what a page's opening says (`Opening`) is the window's, and this is
            // its JS's, as it ran before its first frame.
            let ms = |took: Duration| took.as_millis();
            let loads = loads.borrow();
            let said = format!(
                "its runtime and prelude {} ms, Intl {} ms, its modules read in {} ms, {} compiled and {} loaded as kept bytecode, read, compiled and run in {} ms",
                ms(preluded),
                ms(intl),
                ms(loads.read),
                loads.compiled,
                loads.kept,
                ms(began.elapsed() - preluded - intl),
            );
            drop(loads);
            Ok((Persistent::save(&ctx, host), Persistent::save(&ctx, started), said))
        }))?;
        let sandbox = Sandbox { host, context, runtime, posted, rejected, clock, warned: std::cell::Cell::new(false), threw, nodes, limit: std::cell::Cell::new(memory_limit()) };
        let running = Instant::now();
        sandbox.settle();
        eprintln!("{page}'s JS started in {} ms: {said}, then ran until it waited in {} ms", began.elapsed().as_millis(), running.elapsed().as_millis());
        // What its module threw as it ran, which its promise holds, and running out of memory as it did.
        block_on(sandbox.context.with(|ctx| {
            let started = run(&ctx, started.restore(&ctx))?;
            match started.result::<Value>() {
                Some(Err(error)) => run(&ctx, Err(error)),
                _ => Ok(()),
            }
        }))?;
        if let Some(error) = sandbox.threw.borrow_mut().extract_if(.., |error| error.is::<OutOfMemory>()).next() {
            return Err(error);
        }
        Ok(sandbox)
    }

    /// Tells the UI library what the person did (`ui.event`), or a message from its client (`ui.message`), as a
    /// plain object made directly, and runs it until it waits again.
    fn send(&self, to: &str, message: &serde_json::Value) {
        let called = block_on(self.context.with(|ctx| {
            let ui: rquickjs::Object = run(&ctx, ctx.globals().get("ui"))?;
            let function: Function = run(&ctx, ui.get(to))?;
            let message = run(&ctx, js(&ctx, message))?;
            run(&ctx, function.call::<_, ()>((rquickjs::function::This(ui), message)))
        }));
        // What it threw is its error, as a browser reports an event handler's.
        self.threw.borrow_mut().extend(called.err());
        self.settle();
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
    fn frame(&self, time: f64) {
        let ran = block_on(self.context.with(|ctx| {
            let host = run(&ctx, self.host.clone().restore(&ctx))?;
            run(&ctx, host.get::<_, Function>("frame").and_then(|frame| frame.call::<_, ()>((time,))))
        }));
        self.threw.borrow_mut().extend(ran.err());
        self.settle();
    }

    /// Runs the timers that are due.
    fn fire(&self) {
        let fired = block_on(self.context.with(|ctx| {
            let host = run(&ctx, self.host.clone().restore(&ctx))?;
            run(&ctx, host.get::<_, Function>("fire").and_then(|fire| fire.call::<_, ()>(())))
        }));
        self.threw.borrow_mut().extend(fired.err());
        self.settle();
    }

    /// What it's thrown since this was last asked.
    fn threw(&self) -> Vec<anyhow::Error> {
        std::mem::take(&mut *self.threw.borrow_mut())
    }

    /// Runs its jobs, and what the Web APIs answer with a promise, until none is left, then tells it of each promise
    /// rejected that nothing handled (`unhandledrejection`), as a browser does once its microtasks have run. What a job
    /// threw is what it threw (`threw`); out of memory, it runs none after.
    fn settle(&self) {
        loop {
            loop {
                match block_on(self.runtime.execute_pending_job()) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        let caught = block_on(error.0.with(|ctx| caught(rquickjs::CaughtError::from_error(&ctx, rquickjs::Error::Exception))));
                        let out = caught.is::<OutOfMemory>();
                        self.threw.borrow_mut().push(caught);
                        if out {
                            return;
                        }
                    }
                }
            }
            let rejected = std::mem::take(&mut *self.rejected.borrow_mut());
            if rejected.is_empty() {
                return;
            }
            let told = block_on(self.context.with(|ctx| {
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
            }));
            if let Err(error) = told {
                return self.threw.borrow_mut().push(error);
            }
        }
    }

    /// Collects what its JS no longer holds, as a browser does while nothing's happening, and puts off its next
    /// collection until it holds twice as much and 8 MB more, within half its memory, so what it's sent doesn't wait on
    /// one: QuickJS's collector goes through all it holds at once, 15 to 25 ms for a long list's page. Holding three
    /// quarters of its memory, it's warned once, until it holds less than half (`memory_limit`, less its tree's: `hold`).
    /// → the bytes it holds once it's collected.
    fn collect(&self) -> usize {
        let limit = self.limit.get();
        let held = block_on(async {
            self.runtime.run_gc().await;
            let held = usize::try_from(self.runtime.memory_usage().await.malloc_size).unwrap_or(0);
            self.runtime.set_gc_threshold((held * 2 + (8 << 20)).min(limit / 2).max(held + (4 << 20))).await;
            held
        });
        if held > limit / 4 * 3 && !self.warned.replace(true) {
            self.send("message", &serde_json::json!({"jsonrpc": "2.0", "method": "notifications/one/memory_warning"}));
        }
        if held < limit / 2 {
            self.warned.set(false);
        }
        held
    }

    /// Its tree counts toward the memory it may have, as a browser tab's DOM is its renderer's memory: what its JS may
    /// hold is what's left of `memory_limit` once its nodes are counted, so a page that makes nodes without end runs out
    /// as one that keeps objects does. Its tree taking all of it, it has run out.
    fn hold(&self) {
        let limit = memory_limit().saturating_sub(self.nodes.get() * NODE);
        if limit == 0 {
            return self.threw.borrow_mut().push(anyhow::Error::new(OutOfMemory));
        }
        if self.limit.replace(limit) != limit {
            block_on(self.runtime.set_memory_limit(limit));
        }
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
    /// How many calls it's been sent, and of those how many reached its thread: the rest are held (`seed`).
    sent: u64,
    posted: u64,
    /// The number of the last call that told it something (`send`), rather than run its timers or a frame.
    told: u64,
    /// With a seed (`seed`), a xorshift's state, and the calls held while its thread is busy with something else, until
    /// when on its clock.
    random: u64,
    held: Vec<Call>,
    until: f64,
}

enum Call {
    /// `ui.event` or `ui.message`, with what it says.
    Send(&'static str, serde_json::Value),
    /// Runs the timers that are due, and looks again at when the next is, as its clock was moved or paused.
    Fire,
    /// A frame is shown, at this time on its clock: what asked for one runs.
    Frame(f64),
    /// Calls that came while its thread was busy with something else, which it runs together in one turn.
    Batch(Vec<Call>),
    /// Collects what its JS no longer holds, now, as a heap snapshot does first (`Thread::collect`).
    Collect,
}

impl Call {
    /// How many calls it is.
    fn len(&self) -> u64 {
        match self {
            Call::Batch(calls) => calls.len() as u64,
            _ => 1,
        }
    }

    fn run(self, sandbox: &Sandbox, memory: &std::sync::atomic::AtomicUsize) {
        match self {
            Call::Send(to, message) => sandbox.send(to, &message),
            Call::Frame(time) => sandbox.frame(time),
            Call::Fire => {}
            Call::Collect => memory.store(sandbox.collect(), Ordering::Relaxed),
            Call::Batch(calls) => calls.into_iter().for_each(|call| call.run(sandbox, memory)),
        }
    }
}

/// A xorshift's next number below `below`, from `state`, where it's seeded (not 0).
fn roll(state: &mut u64, below: u64) -> u64 {
    if *state == 0 || below == 0 {
        return 0;
    }
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state % below
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
    /// The bytes its JS held as it was last collected.
    memory: std::sync::atomic::AtomicUsize,
    /// With a seed (`Thread::seed`), a xorshift's state for which calls that came together it runs a turn apart.
    split: Mutex<u64>,
}

#[derive(Default)]
struct Posts {
    posted: VecDeque<Posted>,
    /// How many calls it's run, whether its module has run (or why it didn't), and whether the thread ended.
    ran: u64,
    started: Option<Result<(), String>>,
    ended: bool,
    /// Why it crashed, if it did (`crashed`).
    crashed: Option<String>,
    /// When its next timer is due, on its clock, and each of its timers (`Sandbox::timers`), as its last turn left them.
    due: Option<f64>,
    timers: Vec<serde_json::Value>,
    /// Whether it asked for a frame, as its last turn left it.
    framing: bool,
    /// When, on its clock, its last turn ended.
    turned: f64,
    /// How long it's spent running its JS (timers, calls, collecting what it dropped) since this was last taken.
    script: Duration,
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
            // Its page answers its person's keys and taps and says what's drawn, so it runs as the windows' thread does.
            let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Max);
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
            let (mut threw, mut ran, mut collect, mut spent) = (Vec::new(), 0, true, Duration::ZERO);
            // How many turns in a row have thrown, and whether this one ran anything: one that only collected says nothing.
            let (mut failing, mut worked) = (0, false);
            loop {
                // A timer due now runs now, its clock paused or not.
                let firing = crate::platform::on_cpu();
                // One that throws runs again next turn, never in a loop here: one that can't clear itself spins nothing.
                while sandbox.due().is_some_and(|due| due <= 0.0) {
                    (collect, worked) = (true, true);
                    sandbox.fire();
                    threw.extend(sandbox.threw());
                    if !threw.is_empty() {
                        break;
                    }
                }
                spent += crate::platform::on_cpu().saturating_sub(firing);
                // A page that's crashed stops, its runtime and all it held let go of, and its window says so.
                sandbox.hold();
                threw.extend(sandbox.threw());
                if std::mem::take(&mut worked) {
                    failing = if threw.is_empty() { 0 } else { failing + 1 };
                }
                if let Some(why) = crashed(&threw, failing) {
                    eprintln!("{page}'s JS crashed: {why}");
                    return on.post(|posts| {
                        posts.posted.extend(threw.drain(..).map(|error| Posted::Threw(format!("{error:#}"))));
                        posts.crashed = Some(why);
                    });
                }
                let (due, timers, framing) = (sandbox.due(), sandbox.timers(), sandbox.framing());
                let now = time.now();
                // It's started once what its module posted as it ran is here too, so its window shows that.
                on.post(|posts| {
                    posts.started = Some(Ok(()));
                    posts.posted.extend(sandbox.take());
                    posts.posted.extend(threw.drain(..).map(|error: anyhow::Error| Posted::Threw(format!("{error:#}"))));
                    posts.ran += std::mem::take(&mut ran);
                    posts.script += std::mem::take(&mut spent);
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
                                let collecting = crate::platform::on_cpu();
                                on.memory.store(sandbox.collect(), Ordering::Relaxed);
                                spent += crate::platform::on_cpu().saturating_sub(collecting);
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
                let running = crate::platform::on_cpu();
                // Every call that's come runs in this turn, and what they changed goes back as one batch; with a seed,
                // one that came with others now and then waits for a turn of its own, as a busy device's thread
                // takes what came in two.
                let apart = || roll(&mut on.split.lock().unwrap_or_else(std::sync::PoisonError::into_inner), 3) == 0;
                for call in first.into_iter().chain(std::iter::from_fn(|| (!apart()).then(|| queue.try_recv().ok()).flatten())) {
                    ran += call.len();
                    worked |= !matches!(call, Call::Fire | Call::Collect);
                    call.run(&sandbox, &on.memory);
                }
                threw.extend(sandbox.threw());
                spent += crate::platform::on_cpu().saturating_sub(running);
            }
        })?;
        Ok(Thread { calls, shared, clock, sent: 0, posted: 0, told: 0, random: 0, held: Vec::new(), until: 0.0 })
    }

    /// Tells the UI library what the person did (`ui.event`), or a message from its client (`ui.message`), for its
    /// thread to run when it can.
    pub fn send(&mut self, to: &'static str, message: serde_json::Value) {
        self.call(Call::Send(to, message));
        self.told = self.sent;
    }

    /// Has it run the timers that are due, as its clock was moved or paused, and wait again till its next.
    pub fn fire(&mut self) {
        self.call(Call::Fire);
    }

    /// A frame is shown at `time`, on its clock: what asked for one runs (`requestAnimationFrame`).
    pub fn frame(&mut self, time: f64) {
        self.call(Call::Frame(time));
    }

    /// Has it collect what its JS no longer holds now, rather than once it's been still a moment, so `memory`, once it's
    /// run (`wait`), is what it holds now.
    pub fn collect(&mut self) {
        self.post(Call::Collect);
    }

    /// The bytes its JS held as it was last collected, once it had been still a moment.
    pub fn memory(&self) -> usize {
        self.shared.memory.load(Ordering::Relaxed)
    }

    /// How long its thread has spent running its JS since this was last asked: what a frame's JS cost.
    pub fn script(&self) -> Duration {
        std::mem::take(&mut self.shared.posts().script)
    }

    /// Whether it asked for a frame, as its last turn left it.
    pub fn framing(&self) -> bool {
        self.shared.posts().framing
    }

    /// Whether, and how, it's adversarial, the same each time for the same seed, as GPUI's randomized executor is: its
    /// thread is now and then busy with something else for 10 to 120 ms of its clock, so the calls that come meanwhile
    /// reach it together in one turn (a key and the Tab after it, a press's down and up, a frame), and of calls that
    /// come together, now and then one waits for a turn of its own, what it changed meanwhile going to its window
    /// first, as a busy device's thread takes what came in two. 0 for neither.
    pub fn seed(&mut self, seed: u64) {
        self.random = seed.wrapping_mul(0xA24B_AED4_963E_E407) | u64::from(seed != 0);
        *self.shared.split.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = seed.wrapping_mul(0x9FB2_1C65_1E98_DF25) | u64::from(seed != 0);
    }

    fn call(&mut self, call: Call) {
        self.sent += 1;
        let now = self.clock.now();
        if self.held.is_empty() && roll(&mut self.random, 4) == 1 {
            self.until = now + 10.0 + roll(&mut self.random, 110) as f64;
        }
        if self.until > now {
            return self.held.push(call);
        }
        self.release();
        self.post(call);
    }

    fn post(&mut self, call: Call) {
        let calls = call.len();
        if self.calls.send(call).is_ok() {
            self.posted += calls;
        }
    }

    /// What was held while its thread was busy goes to it, together.
    fn release(&mut self) {
        if !self.held.is_empty() {
            let held = std::mem::take(&mut self.held);
            self.post(Call::Batch(held));
        }
    }

    /// What it's given its host since this was last asked: what it did with what it was sent, a batch at a time, and
    /// how many of the calls it was sent it had run by then (`sent`). What was held while its thread was busy goes to
    /// it once that's past.
    pub fn take(&mut self) -> (Vec<Posted>, u64) {
        if self.until <= self.clock.now() {
            self.release();
        }
        self.shared.waiting.store(false, Ordering::Release);
        let mut posts = self.shared.posts();
        (posts.posted.drain(..).collect(), posts.ran)
    }

    /// Whether it's posted what its window hasn't taken yet (`take`).
    pub fn has_posted(&self) -> bool {
        !self.shared.posts().posted.is_empty()
    }

    /// How many calls it's been sent: the last one's number.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Where calls are held for it while it's busy with something else (`seed`), how long till it's free, in ms on its
    /// clock.
    pub fn holding(&self) -> Option<f64> {
        (!self.held.is_empty()).then(|| self.until - self.clock.now())
    }

    /// The number of the last call that told it something (`send`): what the person did, or its client said.
    pub fn told(&self) -> u64 {
        self.told
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

    /// Why its page crashed after it started, once it has (`crashed`): its thread has ended, and what it held is gone.
    pub fn crashed(&self) -> Option<String> {
        self.shared.posts().crashed.clone()
    }

    /// Whether its thread has something it was sent to run yet, or hasn't started: what it then posts is on its way.
    /// What's held for it (`seed`) isn't its yet.
    pub fn busy(&self) -> bool {
        let posts = self.shared.posts();
        !posts.ended && (posts.started.is_none() || posts.ran < self.posted)
    }

    /// Whether its thread ended.
    pub fn ended(&self) -> bool {
        self.shared.posts().ended
    }

    /// Waits until it has run all that's reached its thread, or ended.
    pub fn wait(&self) {
        let mut posts = self.shared.posts();
        while !posts.ended && (posts.started.is_none() || posts.ran < self.posted) {
            posts = self.shared.turned.wait(posts).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// When its next timer is due, in ms from now on its clock, if it has one, or what's held for it while it's busy
    /// (`seed`), which it runs, with its timers, once it's free. A page whose thread has ended has none.
    pub fn due(&self) -> Option<f64> {
        let posts = self.shared.posts();
        if posts.ended {
            return None;
        }
        let due = posts.due;
        drop(posts);
        let due = if self.held.is_empty() { due? } else { due.unwrap_or(self.until).max(self.until) };
        Some(due - self.clock.now())
    }

    /// Each of its timers: when it's due, in ms from now (`in`), and how often an interval runs (`every`).
    pub fn timers(&self) -> Vec<serde_json::Value> {
        let posts = self.shared.posts();
        if posts.ended {
            return Vec::new();
        }
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
/// kept to be applied once the sandbox's turn has run, and how many nodes it's made and not dropped, in `nodes`.
fn mutations<'js>(ctx: &Ctx<'js>, posted: &Rc<RefCell<Vec<Posted>>>, nodes: &Rc<std::cell::Cell<usize>>) -> rquickjs::Result<Object<'js>> {
    let mutations = Object::new(ctx.clone())?;
    let (to, made) = (posted.clone(), nodes.clone());
    let create = move |ctx: Ctx<'js>, id: Id, kind: String, value: Option<String>| -> rquickjs::Result<()> {
        let kind = Kind::deserialize(kind.as_str().into_deserializer())
            .map_err(|error: serde::de::value::Error| Exception::throw_type(&ctx, &error.to_string()))?;
        to.borrow_mut().push(Posted::Mutation(Mutation::Create { id, kind, value }));
        made.set(made.get() + 1);
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
    let dropped = nodes.clone();
    let drop = move |id: Id| {
        to.borrow_mut().push(Posted::Mutation(Mutation::Drop { id }));
        dropped.set(dropped.get().saturating_sub(1));
    };
    mutations.set("drop", Function::new(ctx.clone(), drop)?)?;
    // It draws a `video`, a screen's live video over WebRTC (`video`).
    mutations.set("video", true)?;
    // It draws a `frame`, another app's page in a sandbox of its own (`view::frames`).
    mutations.set("frame", true)?;
    // It draws a `list`, holding only the rows near the window, each at its own place (`view::list`): version 2.
    mutations.set("list", 2)?;
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
    result.catch(ctx).map_err(caught)
}

/// What a page's JS threw, as an error: `OutOfMemory` where QuickJS ran out of the memory it may have, which it says with
/// an InternalError, or, where it hadn't the memory for even that, with nothing (null).
fn caught(error: rquickjs::CaughtError) -> anyhow::Error {
    match &error {
        rquickjs::CaughtError::Exception(exception) if exception.message().as_deref() == Some("out of memory") => anyhow::Error::new(OutOfMemory),
        rquickjs::CaughtError::Value(value) if value.is_null() => anyhow::Error::new(OutOfMemory),
        _ => anyhow!("{error}"),
    }
}

/// What threw past a timer, a frame, a listener or a job, or a promise nothing handled, as `caught` has it.
fn uncaught(error: Value) -> anyhow::Error {
    caught(error.clone().into_object().and_then(Exception::from_object).map_or(rquickjs::CaughtError::Value(error), rquickjs::CaughtError::Exception))
}

/// QuickJS ran out of the memory a page may have (`memory_limit`).
#[derive(Debug)]
struct OutOfMemory;

impl std::fmt::Display for OutOfMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "it ran out of memory ({} MB)", memory_limit() >> 20)
    }
}

impl std::error::Error for OutOfMemory {}

/// How many turns in a row may throw before its page has crashed.
const FAILING: u32 = 3;

/// Why a page whose last turn threw `threw`, its `failing`th in a row to throw, has crashed, if it has, as a browser's
/// tab does (Chrome's "Aw, Snap!"): it ran out of memory, after which what it was running is half done (an effect half
/// run, a timer that couldn't clear itself), or every turn of its last few threw, so it does nothing its person asks.
fn crashed(threw: &[anyhow::Error], failing: u32) -> Option<String> {
    if let Some(error) = threw.iter().find(|error| error.is::<OutOfMemory>()) {
        return Some(error.to_string());
    }
    (failing >= FAILING).then(|| format!("each of its last {FAILING} turns threw: {:#}", threw[threw.len() - 1]))
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
    loads: Rc<RefCell<Loads>>,
}

/// How a page's modules were loaded as it started, for what it says of its start (`Sandbox::start`): how long reading
/// them took, and how many were compiled and how many loaded as the bytecode kept of them (`cache`).
#[derive(Default)]
struct Loads {
    read: Duration,
    compiled: u32,
    kept: u32,
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
        let reading = Instant::now();
        let source = (self.files)(&url).map_err(|error| refused(format!("{error:#}")))?;
        self.loads.borrow_mut().read += reading.elapsed();
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
                self.loads.borrow_mut().kept += 1;
                return Ok(module);
            }
        }
        let module = Module::declare(ctx.clone(), name, source)?;
        self.loads.borrow_mut().compiled += 1;
        if let (Some(kept), Ok(bytecode)) = (kept, module.write(WriteOptions::default())) {
            let _ = std::fs::create_dir_all(kept.parent().expect("it's in the cache"));
            let _ = std::fs::write(kept, bytecode);
        }
        Ok(module)
    }
}

