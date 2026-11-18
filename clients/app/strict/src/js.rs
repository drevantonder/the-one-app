//! An app's JS, in a QuickJS sandbox of its own (rquickjs), with the Web APIs a Worker has, the WinterTC Minimum Common
//! API's, but the network's (`fetch`): AWS LLRT's implementations of them, the sandbox's own clock, timers and
//! `console` (`prelude.js`), and `Intl` on ICU4X (`intl.js`, `intl.rs`); core/ui/src/strict/types/globals.d.ts says
//! which. It talks with its host over two channels. What it draws is the renderer's (core/ui/src/strict/renderer.ts):
//! `globalThis.mutations` is the host's functions it calls with each mutation of its tree, with its values as they are,
//! never JSON, as React Native's JSI calls its host, and the UI library's `ui.event` is called the same way with each
//! event. What it asks its client is MCP Apps' JSON-RPC (core/ui/src/strict/calls.ts): `globalThis.host.post` and
//! `ui.message`. Its page's module
//! and what that imports are read by `Files`, and each runs as QuickJS's bytecode, compiled once and kept (`cache`), as
//! QuickJS's own `qjsc` compiles a program ahead of time. What the Web APIs answer with a promise runs on rquickjs's
//! async runtime, which nothing here waits on: each call into the sandbox runs its jobs until none is left.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::anyhow;
use futures_lite::future::block_on;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::{Declared, WriteOptions};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Exception, Function, Module, Object, Persistent, Type, Value};
use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use sha2::{Digest, Sha256};
use url::Url;

use crate::tree::{Id, Kind, Mutation};

/// The sandbox's own globals (`prelude.js`), with the DOM's events (`event-target-shim.js`).
const PRELUDE: &str = include_str!("prelude.js");
const EVENT_TARGET_SHIM: &str = include_str!("event-target-shim.js");
/// Intl, as ECMA-402 says, on what intl.rs formats with.
const INTL: &str = include_str!("intl.js");

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

    /// The language the sandbox's JS formats in (`navigator.language`): the host's, or for a fixed clock, whose output
    /// is the same everywhere, `en-US`.
    pub fn locale(&self) -> String {
        match self.0.epoch_ms {
            Some(_) => "en-US".into(),
            None => sys_locale::get_locale().unwrap_or_else(|| "en-US".into()),
        }
    }

    /// The time zone the sandbox's JS shows times in, by its IANA name: the host's, or for a fixed clock, `UTC`.
    pub fn time_zone(&self) -> String {
        match self.0.epoch_ms {
            Some(_) => "UTC".into(),
            None => crate::intl::host_time_zone(),
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
    /// A mutation of its tree (`mutations`).
    Mutation(Mutation),
    /// A JSON-RPC message for its client (`host.post`), as JSON.
    Message(String),
}

/// One app's JS, running.
/// Its fields drop in order: what's held in the runtime before the runtime.
pub struct Sandbox {
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
    pub fn start(page: &Url, files: Files, cache: Option<PathBuf>, clock: Clock) -> anyhow::Result<Sandbox> {
        let runtime = AsyncRuntime::new()?;
        let rejected: Rc<RefCell<Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>>> = Rc::default();
        let to = rejected.clone();
        block_on(async {
            runtime.set_memory_limit(MEMORY).await;
            runtime.set_max_stack_size(STACK).await;
            runtime.set_loader(Resolve, Modules { files, cache }).await;
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
            let host: rquickjs::Object = run(&ctx, ctx.eval(format!("const FIXED = {fixed}, LOCALE = {locale}, shim = (() => {{\n{EVENT_TARGET_SHIM}\n}})();\n{PRELUDE}")))?;
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
    pub fn send(&self, to: &str, message: &serde_json::Value) -> anyhow::Result<()> {
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
    pub fn due(&self) -> Option<f64> {
        let due: f64 = block_on(self.context.with(|ctx| -> rquickjs::Result<f64> {
            let host = self.host.clone().restore(&ctx)?;
            host.get::<_, Function>("due")?.call(())
        }))
        .ok()?;
        due.is_finite().then_some(due - self.clock.now())
    }

    /// Each of its timers: when it's due, in ms from now (`in`), and how often an interval runs (`every`).
    pub fn timers(&self) -> Vec<serde_json::Value> {
        block_on(self.context.with(|ctx| -> rquickjs::Result<String> {
            let host = self.host.clone().restore(&ctx)?;
            host.get::<_, Function>("timers")?.call(())
        }))
        .ok()
        .and_then(|timers| serde_json::from_str(&timers).ok())
        .unwrap_or_default()
    }

    /// Runs the timers that are due.
    pub fn fire(&self) -> anyhow::Result<()> {
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

    /// What it has given its host since this was last asked.
    pub fn take(&self) -> Vec<Posted> {
        std::mem::take(&mut *self.posted.borrow_mut())
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }
}

/// The host's functions the renderer calls with each mutation (`Mutations` in core/ui/src/strict/renderer.ts), each
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
    let set = move |id: Id, name: String, value: Value<'js>| to.borrow_mut().push(Posted::Mutation(Mutation::Set { id, name, value: data(&value) }));
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
    Ok(mutations)
}

/// A JS value as JSON's data model, read from it directly: a function, a symbol or `undefined` is `null`, and a number
/// that's whole is an integer, as JSON's text would have it.
fn data(value: &Value) -> serde_json::Value {
    use serde_json::Value as Data;
    match value.type_of() {
        Type::Bool => Data::Bool(value.as_bool().unwrap_or_default()),
        Type::Int => Data::from(value.as_int().unwrap_or_default()),
        Type::Float => {
            let number = value.as_float().unwrap_or_default();
            if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 { Data::from(number as i64) } else { serde_json::Number::from_f64(number).map_or(Data::Null, Data::Number) }
        }
        Type::String => value.as_string().and_then(|text| text.to_string().ok()).map_or(Data::Null, Data::String),
        Type::Array => value.as_array().map_or(Data::Null, |array| Data::Array(array.iter::<Value>().map(|item| item.map_or(Data::Null, |item| data(&item))).collect())),
        Type::Object => value.as_object().map_or(Data::Null, |object| {
            Data::Object(object.props::<String, Value>().filter_map(Result::ok).filter(|(_, value)| !value.is_undefined() && !value.is_function()).map(|(name, value)| (name, data(&value))).collect())
        }),
        _ => Data::Null,
    }
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
