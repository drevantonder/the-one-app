//! A code mode script, run by OpenCode's interpreter (core/codemode/run.js, bundled) in a QuickJS sandbox of its own,
//! on a thread of its own, as `OneCore.CodeMode.Runner` starts it. It reaches nothing but its host: each tool call it
//! makes is a message to the process that started it, `{:codemode, tag, {:call, json}}`, answered with `reply/2`, and
//! it ends with `{:codemode, tag, {:done, ok, text}}`, or `{:codemode, tag, {:exit, why}}` where it couldn't finish.
//! `stop/1`, or its process no longer holding it, stops it at once, even in a loop that never waits, and so does its
//! holding more memory than it may: QuickJS checks both as it runs (its interrupt handler).
//!
//!     running ─done / failed─► ended (told)
//!     running ─holds more than MEMORY─► out of memory (QuickJS interrupted, told so)
//!     running ─stop / dropped─► stopped (QuickJS interrupted, nothing told)

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::module::WriteOptions;
use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Object, Persistent, Promise, Runtime};
use rustler::{Binary, Encoder, Env, OwnedEnv, ResourceArc};

/// What the sandbox's JS may hold, and how deep it may call: a script's data is a tool's answers, which a run keeps
/// to 60 KB of what it gives back (run.js), and the interpreter is a loop, not a recursion. It's stopped once it holds
/// more than MEMORY, and what it asks for past REFUSED isn't given, as one string can pass MEMORY between QuickJS's
/// checks. Stopped by QuickJS's own limit instead, it collected its garbage at nearly every allocation, for seconds,
/// and the interpreter said only "null".
const MEMORY: usize = 256 << 20;
const REFUSED: usize = MEMORY + (32 << 20);
const OUT_OF_MEMORY: &str = "OutOfMemory: the script held more than its 256 MB.";
const STACK: usize = 4 << 20;
const PRELUDE: &str = include_str!("prelude.js");

mod atoms {
    rustler::atoms! { codemode, call, done, exit, ok }
}

/// A run, as its process holds it. Dropped, its thread stops.
pub struct Run {
    replies: Mutex<Option<mpsc::Sender<String>>>,
    stop: Arc<AtomicBool>,
}

#[rustler::resource_impl]
impl rustler::Resource for Run {}

impl Drop for Run {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

enum Said {
    Call(String),
    Done(bool, String),
    Exit(String),
}

/// Starts `script`, the JSON run.js's `run` is given, with the interpreter's `bundle`, telling the calling process
/// what it says, each message tagged `tag`.
#[rustler::nif]
fn start(env: Env, bundle: Binary, script: String, tag: i64) -> ResourceArc<Run> {
    let pid = env.pid();
    let bundle = bundle.as_slice().to_vec();
    let (replies, answers) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::Builder::new()
        .name("codemode".into())
        .stack_size(STACK * 2)
        .spawn(move || {
            let mut owned = OwnedEnv::new();
            let quiet = stopped.clone();
            let tell = Rc::new(RefCell::new(move |said: Said| {
                // Stopped, it says nothing more; gone, its process hears nothing, which is as good.
                if !quiet.load(Ordering::Relaxed) {
                    let _ = owned.send_and_clear(&pid, |env| (atoms::codemode(), tag, encode(env, said)).encode(env));
                }
            }));
            let told = tell.clone();
            let ended = run(&bundle, &script, answers, &stopped, move |line| (told.borrow_mut())(Said::Call(line)));
            (tell.borrow_mut())(ended);
        })
        .expect("a thread for a code mode run");
    ResourceArc::new(Run { replies: Mutex::new(Some(replies)), stop })
}

/// Answers a call the script made, as run.js's `reply` takes it.
#[rustler::nif]
fn reply(run: ResourceArc<Run>, line: String) -> rustler::Atom {
    if let Some(replies) = run.replies.lock().unwrap().as_ref() {
        let _ = replies.send(line);
    }
    atoms::ok()
}

/// Stops the run where it is.
#[rustler::nif]
fn stop(run: ResourceArc<Run>) -> rustler::Atom {
    run.stop.store(true, Ordering::Relaxed);
    run.replies.lock().unwrap().take();
    atoms::ok()
}

fn encode<'a>(env: Env<'a>, said: Said) -> rustler::Term<'a> {
    match said {
        Said::Call(line) => (atoms::call(), line).encode(env),
        Said::Done(ok, text) => (atoms::done(), ok, text).encode(env),
        Said::Exit(why) => (atoms::exit(), why).encode(env),
    }
}

/// Runs the script until it's done, as run.js's `run` resolves, running its jobs, its timers and each answer it's
/// given, and waiting on nothing but an answer or its next timer.
fn run(bundle: &[u8], script: &str, answers: mpsc::Receiver<String>, stop: &Arc<AtomicBool>, say: impl Fn(String) + 'static) -> Said {
    let logged = Rc::new(RefCell::new(Vec::<String>::new()));
    let failed = |why: String| Said::Exit(format!("{why}{}", logged.borrow().iter().map(|line| format!("\n{line}")).collect::<String>()));
    let memory = Counted::default();
    let held = memory.clone();
    let Ok(runtime) = Runtime::new_with_alloc(memory.clone()) else { return failed("QuickJS didn't start.".into()) };
    runtime.set_max_stack_size(STACK);
    let interrupt = stop.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt.load(Ordering::Relaxed) || held.over(MEMORY))));
    // Out of memory, however the script came to it, unless it caught that and finished anyway.
    let failed = |why: String| if memory.out.load(Ordering::Relaxed) { Said::Done(false, OUT_OF_MEMORY.into()) } else { failed(why) };
    let Ok(context) = Context::full(&runtime) else { return failed("QuickJS didn't start.".into()) };
    let started = Instant::now();

    let begun = context.with(|ctx| -> Result<_, String> {
        let globals = ctx.globals();
        let caught = |error: rquickjs::Error| error.to_string();
        globals.set("__say", Function::new(ctx.clone(), move |line: String| say(line)).map_err(caught)?).map_err(caught)?;
        globals.set("__now", Function::new(ctx.clone(), move || started.elapsed().as_secs_f64() * 1000.0).map_err(caught)?).map_err(caught)?;
        let log = logged.clone();
        globals.set("__log", Function::new(ctx.clone(), move |line: String| log.borrow_mut().push(line)).map_err(caught)?).map_err(caught)?;
        js(&ctx, llrt_util::init(&ctx))?;
        js(&ctx, llrt_crypto::init(&ctx))?;
        let host: Object = js(&ctx, ctx.eval(PRELUDE))?;
        let compiled = compiled(&ctx, bundle)?;
        // SAFETY: the bytecode is this QuickJS's own, written by `compiled` in this process.
        let (_, evaluated) = js(&ctx, unsafe { Module::load(ctx.clone(), &compiled) }.and_then(|module| module.eval()))?;
        js(&ctx, evaluated.finish::<()>())?;
        let input = js(&ctx, ctx.json_parse(script))?;
        let run: Function = js(&ctx, globals.get("run"))?;
        let promise: Promise = js(&ctx, run.call((input,)))?;
        Ok((Persistent::save(&ctx, host), Persistent::save(&ctx, promise)))
    });
    let (host, promise) = match begun {
        Ok(begun) => begun,
        Err(why) => return failed(why),
    };

    loop {
        while runtime.is_job_pending() {
            if let Err(error) = runtime.execute_pending_job() {
                let why = error.0.with(|ctx| ctx.catch().into_exception().map_or_else(|| "A job failed.".to_string(), |error| error.to_string()));
                return failed(why);
            }
        }
        if stop.load(Ordering::Relaxed) {
            return Said::Exit("It was stopped.".into());
        }
        let now = context.with(|ctx| -> Result<_, String> {
            let promise = promise.clone().restore(&ctx).map_err(|error| error.to_string())?;
            match promise.result::<Object>() {
                Some(Ok(result)) => match js(&ctx, result.get("ok"))? {
                    false if memory.out.load(Ordering::Relaxed) => Ok(Err(Said::Done(false, OUT_OF_MEMORY.into()))),
                    ok => Ok(Err(Said::Done(ok, js(&ctx, result.get("text"))?))),
                },
                Some(Err(error)) => Err(js::<()>(&ctx, Err(error)).unwrap_err()),
                None => {
                    let host = host.clone().restore(&ctx).map_err(|error| error.to_string())?;
                    let fire: Function = js(&ctx, host.get("fire"))?;
                    Ok(Ok(js::<f64>(&ctx, fire.call((rquickjs::function::This(host),)))?))
                }
            }
        });
        let next = match now {
            Ok(Ok(next)) => next,
            Ok(Err(done)) => return done,
            Err(why) => return failed(why),
        };
        // A timer ran, and left its jobs.
        if runtime.is_job_pending() {
            continue;
        }
        let answer = if next < 0.0 {
            answers.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            answers.recv_timeout(Duration::from_secs_f64(next / 1000.0))
        };
        match answer {
            Ok(line) => {
                let answered = context.with(|ctx| -> Result<(), String> {
                    let reply: Function = js(&ctx, ctx.globals().get("reply"))?;
                    js(&ctx, reply.call((line,)))
                });
                if let Err(why) = answered {
                    return failed(why);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Said::Exit("It was stopped.".into()),
        }
    }
}

/// The interpreter's bundle as QuickJS's bytecode, compiled the first time a run is given it and kept, as QuickJS's own
/// `qjsc` compiles a program ahead of time: it's parsed once, not for every run, where parsing it took 60 ms.
fn compiled(ctx: &Ctx, bundle: &[u8]) -> Result<Arc<Vec<u8>>, String> {
    static COMPILED: Mutex<Option<(Vec<u8>, Arc<Vec<u8>>)>> = Mutex::new(None);
    let mut kept = COMPILED.lock().unwrap();
    if let Some((source, compiled)) = kept.as_ref()
        && source == bundle
    {
        return Ok(compiled.clone());
    }
    let compiled = Arc::new(js(ctx, Module::declare(ctx.clone(), "run.js", bundle).and_then(|module| module.write(WriteOptions::default())))?);
    *kept = Some((bundle.to_vec(), compiled.clone()));
    Ok(compiled)
}

/// QuickJS's memory, from Rust's allocator, counted: what a run holds, and whether it ran out.
#[derive(Clone, Default)]
struct Counted {
    held: Arc<AtomicUsize>,
    out: Arc<AtomicBool>,
}

impl Counted {
    /// Whether it holds more than `limit`, which is its running out.
    fn over(&self, limit: usize) -> bool {
        let over = self.held.load(Ordering::Relaxed) > limit;
        if over {
            self.out.store(true, Ordering::Relaxed);
        }
        over
    }

    /// Whether `more` fits under REFUSED, which isn't given where it doesn't.
    fn room(&self, more: usize) -> bool {
        !self.over(REFUSED.saturating_sub(more))
    }

    fn given(&self, ptr: *mut u8) -> *mut u8 {
        if !ptr.is_null() {
            // SAFETY: RustAllocator just gave it.
            self.held.fetch_add(unsafe { RustAllocator::usable_size(ptr) }, Ordering::Relaxed);
        }
        ptr
    }
}

// SAFETY: each call is RustAllocator's, which keeps every allocation's size with it.
unsafe impl Allocator for Counted {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if !self.room(size) {
            return std::ptr::null_mut();
        }
        self.given(RustAllocator.alloc(size))
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        if !self.room(count.saturating_mul(size)) {
            return std::ptr::null_mut();
        }
        self.given(RustAllocator.calloc(count, size))
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        self.held.fetch_sub(unsafe { RustAllocator::usable_size(ptr) }, Ordering::Relaxed);
        unsafe { RustAllocator.dealloc(ptr) }
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        if ptr.is_null() {
            return self.alloc(new_size);
        }
        let old = unsafe { RustAllocator::usable_size(ptr) };
        if new_size > old && !self.room(new_size - old) {
            return std::ptr::null_mut();
        }
        let moved = unsafe { RustAllocator.realloc(ptr, new_size) };
        if !moved.is_null() {
            self.held.fetch_sub(old, Ordering::Relaxed);
            self.given(moved);
        }
        moved
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        unsafe { RustAllocator::usable_size(ptr) }
    }
}

/// What QuickJS gave, or what it threw, with its stack.
fn js<'js, T>(ctx: &Ctx<'js>, result: rquickjs::Result<T>) -> Result<T, String> {
    result.catch(ctx).map_err(|error| error.to_string())
}

include!(concat!(env!("OUT_DIR"), "/init.rs"));
