//! Apps run in workerd, Cloudflare's open-source Workers runtime. The app runtime is our wrapper around it, a program of
//! its own (`<cli>-apps`, `run`) that oned starts while the device's `apps` part is switched on and again if it stops: it
//! holds no key and no endpoint, and only keeps a workerd serving the config the app runtime writes, on a port the app runtime holds,
//! so a call waits there while it starts again rather than being refused. Outside oned (tests), the app runtime keeps one in a
//! thread of its own.
//! Each deployed app is an ES module inside `runtime/shim.js`, which asks the app runtime to run its queries and writes on
//! its databases (`database/src/stored.rs`), and runs its tools, widgets, hooks and screens.
//! When the deploys change, the app runtime writes the config again and restarts workerd.
//! Every request an app makes (its `fetch`es, and the shim's asks of the app runtime) goes to the app runtime's outbound port,
//! `app-runtime/src/app/outbound.rs`, carrying the token of the call it's made in.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};


use ::api::call::{Append, appends};
use ::deploy::build::{COMPATIBILITY_DATE, workerd};
use ::scope::state::{Me, ScopeHandle};

/// How long an app may take to answer.
const TIME_LIMIT: Duration = Duration::from_secs(30);
/// How long the runtime may take to start before it's given up on: seconds on a quiet machine, a minute or more on
/// one whose cores are all busy.
const START_LIMIT: Duration = Duration::from_secs(300);

/// Who a call, widget or hook runs for, and what it may reach.
#[derive(Clone, Debug, Serialize)]
pub struct Context {
    pub me: Me,
    pub now: u64,
    pub scopes: Vec<ScopeHandle>,
    /// A call on data opened for its caller: appends fail.
    pub read_only: bool,
    /// What the call's requests carry, so the app runtime knows whose they are (see `app-runtime/src/app/outbound.rs`).
    pub token: String,
    /// The app whose `ctx.apps` made the call, if one did.
    pub from: Option<String>,
    /// The network's address, where a sign-in elsewhere sends the user back (`<address>/back`).
    pub address: Option<String>,
    /// Everyone's and every group's handle (`ScopeDb::handles`).
    pub handles: Value,
    /// The settings of the network's labs that are on (`labs::on_keys`); each app sees its own, as `ctx.labs`.
    #[serde(skip)]
    pub labs: Vec<String>,
}

pub struct Outcome {
    pub result: Value,
    pub appends: Vec<Append>,
    /// Whether the call left work running after it returned (`ctx.waitUntil`), which hands its appends in later.
    pub later: bool,
}

#[derive(Clone)]
pub struct Apps(Arc<Runtime>);

struct Runtime {
    dir: PathBuf,
    base: String,
    /// The app runtime's port that apps' requests go out through.
    outbound: u16,
    /// The apps loaded, (name, module source); the built-in apps the network hasn't added, loaded only to be described;
    /// and how many times the runtime was configured.
    loaded: Mutex<(Vec<(String, String)>, Vec<(String, String)>, u64)>,
    /// The port workerd serves, held for as long as the app runtime runs and handed to each that serves it. None on Windows,
    /// where workerd can't be handed one: it binds the config's port itself, so a call while it restarts is refused.
    listener: Option<std::net::TcpListener>,
    /// What each loaded app says it is, once asked.
    described: Mutex<std::collections::HashMap<String, Value>>,
    /// What each app last said it is, kept when the apps load again, for a service to be listed as its app says.
    identity: Mutex<std::collections::HashMap<String, Value>>,
}

impl Apps {
    /// Starts workerd with no apps yet, keeping its files in `dir`; apps' requests go out through the app runtime's
    /// `outbound` port.
    pub async fn start(dir: PathBuf, outbound: u16) -> Result<Self> {
        tokio::task::spawn_blocking(move || Apps::start_blocking(&dir, outbound)).await?
    }

    fn start_blocking(dir: &Path, outbound: u16) -> Result<Self> {
        // Held for as long as the app runtime runs, and handed to each workerd: no one takes the port meanwhile.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        write_config(dir, &[], port, outbound, 0)?;
        let listener = cfg!(unix).then_some(listener);
        let runtime = Arc::new(Runtime { dir: dir.to_path_buf(), base: format!("http://127.0.0.1:{port}"), outbound, loaded: Mutex::new((Vec::new(), Vec::new(), 0)), listener, described: Mutex::new(Default::default()), identity: Mutex::new(Default::default()) });
        let workerd = workerd()?;
        stop_previous(dir);
        let (dir, listener) = (dir.to_path_buf(), runtime.listener.as_ref().map(|l| l.try_clone()).transpose()?);
        std::thread::Builder::new().name("workerd".into()).spawn(move || keep(&workerd, &dir, listener.as_ref()))?;
        runtime.wait_ready(0)?;
        Ok(Apps(runtime))
    }

    /// Runs exactly these apps, (name, module source), restarting the runtime if they changed. `unadded` are built-in
    /// apps no one has added: they run no calls, but `describe_unadded` asks them what they are.
    pub async fn load(&self, apps: Vec<(String, String)>, unadded: Vec<(String, String)>) -> Result<()> {
        let runtime = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let generation = {
                let mut loaded = runtime.loaded.lock().unwrap();
                if loaded.0 == apps && loaded.1 == unadded {
                    return Ok(());
                }
                loaded.2 += 1;
                let port = runtime.base.rsplit(':').next().unwrap().parse()?;
                write_config(&runtime.dir, &[apps.as_slice(), unadded.as_slice()].concat(), port, runtime.outbound, loaded.2)?;
                loaded.0 = apps;
                loaded.1 = unadded;
                runtime.described.lock().unwrap().clear();
                loaded.2
            };
            runtime.restart();
            runtime.wait_ready(generation)?;
            // The runtime it replaced answers until this one is up: what it said about its apps goes with it.
            runtime.described.lock().unwrap().clear();
            Ok(())
        })
        .await?
    }

    /// The port workerd serves the app runtime on.
    pub fn port(&self) -> u16 {
        self.0.base.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0)
    }

    pub fn names(&self) -> Vec<String> {
        self.0.loaded.lock().unwrap().0.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn has(&self, app: &str) -> bool {
        self.0.loaded.lock().unwrap().0.iter().any(|(n, _)| n == app)
    }

    /// The built-in apps loaded that no one has added.
    pub fn unadded(&self) -> Vec<String> {
        self.0.loaded.lock().unwrap().1.iter().map(|(n, _)| n.clone()).collect()
    }

    async fn ask(&self, app: &str, what: &str, body: Value) -> Result<Value> {
        self.ask_for(app, what, body, TIME_LIMIT).await
    }

    /// Asks the runtime, failing if it takes longer than `limit`, and restarting it if it's stuck.
    async fn ask_for(&self, app: &str, what: &str, body: Value, limit: Duration) -> Result<Value> {
        if !self.has(app) {
            bail!("no app named {app} runs here");
        }
        self.send(app, what, body, limit).await
    }

    async fn send(&self, app: &str, what: &str, body: Value, limit: Duration) -> Result<Value> {
        let url = format!("{}/{app}/{what}", self.0.base);
        let runtime = self.0.clone();
        let reply: Value = tokio::task::spawn_blocking(move || -> Result<Value> {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(limit)).build().into();
            // While the runtime restarts (for a deploy, or after a crash), wait for it rather than fail.
            let started = Instant::now();
            let sent = loop {
                match agent.post(&url).send_json(&body) {
                    Err(ureq::Error::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused && started.elapsed() < Duration::from_secs(20) => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    sent => break sent,
                }
            };
            match sent {
                Ok(mut resp) => Ok(resp.body_mut().with_config().limit(64 << 20).read_json()?),
                // Only this call fails, unless the runtime is stuck (an app spinning holds its one thread): then
                // every call is, and it starts again.
                Err(ureq::Error::Timeout(_)) => {
                    if !runtime.answers() {
                        tracing::warn!("the app runtime is stuck; starting it again");
                        runtime.restart();
                    }
                    ::api::trace::stage(::api::traceparent::stage::LIMIT);
                    bail!("the app took longer than {}s", limit.as_secs())
                }
                Err(err) => Err(err.into()),
            }
        })
        .await??;
        for span in reply["spans"].as_array().into_iter().flatten().take(::insights::MOST_APP_SPANS) {
            ::api::trace::note(json!({ "step": "span", "name": span["name"], "began": span["began"], "failed": span["failed"], "attrs": span["attrs"] }), Duration::from_millis(span["ms"].as_u64().unwrap_or(0)));
        }
        if let Some(err) = reply.get("error") {
            if reply["exception"].is_object() {
                ::api::trace::note(json!({ "step": "exception", "exception": reply["exception"] }), Duration::ZERO);
            }
            return Err(custom_build::protocol::Coded::or_plain(reply["code"].as_str().map(str::to_string), err.as_str().unwrap_or("the app failed")));
        }
        Ok(reply)
    }

    /// Runs a tool, for at most `limit`: longer for an app that waits on services.
    pub async fn call(&self, app: &str, tool: &str, input: Value, ctx: &Context, limit: Duration) -> Result<Outcome> {
        let reply = self.ask_for(app, "call", json!({ "tool": tool, "input": input, "labs": ::labs::of_app(&ctx.labs, app), "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": ctx.read_only, "token": ctx.token, "from": ctx.from, "port": ::api::local::port(), "address": ctx.address, "handles": ctx.handles }), limit).await?;
        let result = reply["ok"].clone();
        let described = self.describe(app).await?;
        if let Some(output) = described["tools"].as_array().into_iter().flatten().find(|t| t["name"] == tool).and_then(|t| t.get("output")) {
            ::api::tools::check(output, &result).with_context(|| format!("{app} {tool} answered what its output schema doesn't say"))?;
        }
        Ok(Outcome { result, appends: appends(&reply["appends"])?, later: reply["later"] == true })
    }

    pub async fn badge(&self, app: &str, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "badge", json!({ "labs": ::labs::of_app(&ctx.labs, app), "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token, "handles": ctx.handles })).await?["ok"].clone())
    }

    /// A widget's tree at `size` (`small`, `medium`, `large` or `xl`), which its render sees as `ctx.size`, with the
    /// options a user chose for it (`ctx.options`, its own defaults for the rest).
    pub async fn widget(&self, app: &str, name: &str, size: &str, options: &Value, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "widget", json!({ "name": name, "size": size, "options": options, "labs": ::labs::of_app(&ctx.labs, app), "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token, "handles": ctx.handles })).await?["ok"].clone())
    }

    /// A widget's options, each `{ name, title, choices, default }`, with choices the app works out from its data.
    pub async fn widget_options(&self, app: &str, name: &str, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "options", json!({ "name": name, "labs": ::labs::of_app(&ctx.labs, app), "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token, "handles": ctx.handles })).await?["ok"].clone())
    }

    pub async fn hook(&self, app: &str, name: &str, input: Value, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "hook", json!({ "name": name, "input": input, "labs": ::labs::of_app(&ctx.labs, app), "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token, "handles": ctx.handles })).await?["ok"].clone())
    }

    /// A screen's HTML, as the app wrote it, or None if the app has no such screen.
    pub async fn screen(&self, app: &str, name: &str) -> Result<Option<String>> {
        let reply = self.ask(app, &format!("screen?name={name}"), json!({})).await?;
        Ok(reply["ok"].as_str().map(str::to_string))
    }

    /// What an app says it is and does, as its loaded deploy says it: asked once per deploy.
    pub async fn describe(&self, app: &str) -> Result<Value> {
        if let Some(d) = self.0.described.lock().unwrap().get(app) {
            return Ok(d.clone());
        }
        self.describing(app, self.ask(app, "describe", json!({}))).await
    }

    /// What a built-in app no one has added says it is and does: it describes itself, touching no data.
    pub async fn describe_unadded(&self, app: &str) -> Result<Value> {
        if let Some(d) = self.0.described.lock().unwrap().get(app) {
            return Ok(d.clone());
        }
        ensure!(self.unadded().iter().any(|n| n == app), "no app named {app} waits to be added here");
        self.describing(app, self.send(app, "describe", json!({}), TIME_LIMIT)).await
    }

    /// Asks for `app`'s description, keeping it unless the runtime was configured again meanwhile.
    async fn describing(&self, app: &str, asking: impl std::future::Future<Output = Result<Value>>) -> Result<Value> {
        let generation = self.0.loaded.lock().unwrap().2;
        let d = asking.await?["ok"].clone();
        if self.0.loaded.lock().unwrap().2 == generation {
            self.0.described.lock().unwrap().insert(app.to_string(), d.clone());
            self.0.identity.lock().unwrap().insert(app.to_string(), d.clone());
        }
        Ok(d)
    }

    /// What `app` last said it is, run here or not yet added: nothing until it's been described.
    pub fn identity(&self, app: &str) -> Option<Value> {
        self.0.identity.lock().unwrap().get(app).cloned()
    }

    /// Describes `app` as it runs here or, if no one has added it, as a built-in waiting to be: what a service of it
    /// is listed as.
    pub async fn describe_any(&self, app: &str) -> Result<Value> {
        match self.has(app) {
            true => self.describe(app).await,
            false => self.describe_unadded(app).await,
        }
    }

    /// `app`'s title, if it runs here and declares one.
    pub async fn title(&self, app: &str) -> Option<String> {
        let cached = self.0.described.lock().unwrap().get(app).map(|d| d["title"].as_str().map(str::to_string));
        let title = match cached {
            Some(t) => t,
            None => self.describe(app).await.ok()?["title"].as_str().map(str::to_string),
        };
        title.filter(|t| !t.trim().is_empty())
    }
}

impl Runtime {
    /// Stops the workerd serving this runtime's config, if one is: what keeps it starts it again, with the config as
    /// it is now.
    fn restart(&self) {
        kill_serving(&self.dir);
    }

    /// Whether the runtime answers at all within a few seconds.
    fn answers(&self) -> bool {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).build().into();
        agent.get(format!("{}/ready", self.base)).call().is_ok()
    }

    /// Waits until the runtime answers with the config of `generation`, or fails at once with `Broken` when an app's
    /// code doesn't start (workerd then starts again and again, saying so in its log). Only a runtime that never
    /// answers runs out the time: one that's slow to start, on a machine whose cores are all busy, is waited for.
    fn wait_ready(&self, generation: u64) -> Result<()> {
        let started = Instant::now();
        let log = self.dir.join("workerd.log");
        let from = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
        // A runtime that keeps failing to start leaves its port's connections unanswered: ask for a moment only.
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(2))).build().into();
        loop {
            if let Ok(mut r) = agent.get(format!("{}/ready", self.base)).call() {
                if r.body_mut().read_to_string().is_ok_and(|g| g.trim() == generation.to_string()) {
                    return Ok(());
                }
            }
            if let Some(broken) = broken_since(&log, from) {
                return Err(broken.into());
            }
            ensure!(started.elapsed() < START_LIMIT, "Apps couldn't start on this device: see {}", log.display());
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// An app whose code doesn't start: the runtime can't run with it.
#[derive(Debug)]
pub struct Broken {
    pub app: String,
    pub error: String,
}

impl std::fmt::Display for Broken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} doesn't start: {}", self.app, self.error)
    }
}

impl std::error::Error for Broken {}

/// The first app the runtime's log says didn't start, after `from` bytes: workerd writes `service app-<name>: Uncaught
/// <error>`.
fn broken_since(log: &Path, from: u64) -> Option<Broken> {
    use std::io::{Read, Seek};
    let mut file = std::fs::File::open(log).ok()?;
    file.seek(std::io::SeekFrom::Start(from)).ok()?;
    let mut text = String::new();
    file.take(1 << 20).read_to_string(&mut text).ok()?;
    text.lines().find_map(|line| {
        let (app, error) = line.strip_prefix("service app-")?.split_once(": Uncaught ")?;
        Some(Broken { app: app.to_string(), error: error.trim().to_string() })
    })
}

/// The file in a runtime's folder that says the id of the workerd running there.
const WORKERD_PID: &str = "workerd.pid";

/// Set once this process is stopping, under `RUNNING`'s lock: no workerd starts again.
static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The workerds this process runs, by process id.
static RUNNING: Mutex<std::collections::BTreeSet<u32>> = Mutex::new(std::collections::BTreeSet::new());

/// Stops every workerd this process runs, for good, as it stops: elsewhere than Linux they'd outlive it.
pub fn stop_all() {
    let mut running = RUNNING.lock().unwrap();
    STOPPING.store(true, std::sync::atomic::Ordering::SeqCst);
    for pid in std::mem::take(&mut *running) {
        platform::os::kill(pid);
    }
}

/// Keeps a workerd serving `dir`'s config on `listener`, starting it again whenever it stops, until this process stops.
fn keep(workerd: &Path, dir: &Path, listener: Option<&std::net::TcpListener>) {
    let (config, log) = (dir.join("config.capnp"), dir.join("workerd.log"));
    loop {
        let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).expect("workerd log");
        let mut command = std::process::Command::new(workerd);
        command.arg("serve").arg(&config).stdout(out.try_clone().unwrap()).stderr(out);
        // Elsewhere than Linux, where it dies with what started it, that stops it as it stops (stop_all), and the
        // next start stops one a killed one left (stop_previous).
        platform::os::child(&mut command, &listener.iter().map(|l| ("http", *l)).collect::<Vec<_>>());
        let spawned = {
            let mut running = RUNNING.lock().unwrap();
            if STOPPING.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            command.spawn().inspect(|c| {
                running.insert(c.id());
            })
        };
        match spawned {
            Ok(mut child) => {
                let _ = std::fs::write(dir.join(WORKERD_PID), child.id().to_string());
                let status = child.wait();
                RUNNING.lock().unwrap().remove(&child.id());
                tracing::debug!("workerd stopped ({status:?}); starting it again");
            }
            Err(err) => tracing::warn!("couldn't start workerd: {err}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Stops the workerd serving `dir`'s config, by the id `keep` wrote there (`WORKERD_PID`), saying whether there was one:
/// Windows' list of processes doesn't say what each was started with.
#[cfg(windows)]
fn kill_serving(dir: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(dir.join(WORKERD_PID)).ok().and_then(|p| p.trim().parse().ok()) else { return false };
    let running = platform::os::is_program(pid, "workerd.exe");
    if running {
        platform::os::kill(pid);
    }
    running
}

/// Stops each workerd serving `dir`'s config, saying whether there was one.
#[cfg(unix)]
fn kill_serving(dir: &Path) -> bool {
    let serving = format!(" serve {}", dir.join("config.capnp").display());
    let Ok(out) = std::process::Command::new("ps").args(["-A", "-ww", "-o", "pid=,command="]).output() else { return false };
    let mut stopped = false;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((pid, command)) = line.trim().split_once(' ') else { continue };
        if command.contains("workerd") && (command.ends_with(&serving) || command.contains(&format!("{serving} "))) {
            stopped |= std::process::Command::new("kill").arg("-9").arg(pid).status().is_ok_and(|s| s.success());
        }
    }
    stopped
}

/// Stops each workerd still serving this runtime's config, which an earlier one left running: one killed, or
/// elsewhere than Linux, where they don't die with it.
fn stop_previous(dir: &Path) {
    if kill_serving(dir) {
        tracing::info!("stopped the workerd an earlier app runtime left running");
        std::thread::sleep(Duration::from_millis(300));
    }
}


/// Writes each app's modules and the workerd config that runs them all behind one router.
fn write_config(dir: &Path, apps: &[(String, String)], port: u16, outbound: u16, generation: u64) -> Result<PathBuf> {
    let modules = dir.join("apps");
    let _ = std::fs::remove_dir_all(&modules);
    std::fs::create_dir_all(&modules)?;
    std::fs::write(dir.join("router.js"), include_str!("runtime/router.js").replace("GENERATION", &generation.to_string()))?;
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut services = vec![
        "(name = \"router\", worker = .router)".to_string(),
        // Every request an app makes goes to the app runtime, which makes it (filling in secrets) or answers it itself.
        format!("(name = \"outbound\", external = (address = \"127.0.0.1:{outbound}\", http = (style = host, forwardedProtoHeader = \"x-forwarded-proto\")))"),
    ];
    let mut bindings = Vec::new();
    let mut workers = Vec::new();
    for (i, (name, source)) in apps.iter().enumerate() {
        ensure!(::scope::scopes::valid_app_name(name), "app names are lowercase words joined by -");
        let app_dir = modules.join(name);
        std::fs::create_dir_all(&app_dir)?;
        std::fs::write(app_dir.join("app.js"), source)?;
        std::fs::write(app_dir.join("shim.js"), ::api::call::shim())?;
        services.push(format!("(name = {}, worker = .app{i})", quote(&format!("app-{name}"))));
        bindings.push(format!("(name = {}, service = {})", quote(name), quote(&format!("app-{name}"))));
        workers.push(format!(
            "const app{i} :Workerd.Worker = (\n  modules = [(name = \"shim.js\", esModule = embed {}), (name = \"app.js\", esModule = embed {})],\n  compatibilityDate = \"{COMPATIBILITY_DATE}\",\n  compatibilityFlags = [\"nodejs_als\"],\n  globalOutbound = \"outbound\",\n);",
            quote(&format!("apps/{name}/shim.js")),
            quote(&format!("apps/{name}/app.js")),
        ));
    }
    let config = format!(
        "using Workerd = import \"/workerd/workerd.capnp\";\n\nconst config :Workerd.Config = (\n  services = [\n    {}\n  ],\n  sockets = [(name = \"http\", address = \"127.0.0.1:{port}\", http = (), service = \"router\")],\n);\n\nconst router :Workerd.Worker = (\n  modules = [(name = \"router.js\", esModule = embed \"router.js\")],\n  compatibilityDate = \"{COMPATIBILITY_DATE}\",\n  bindings = [{}],\n);\n\n{}\n",
        services.join(",\n    "),
        bindings.join(", "),
        workers.join("\n\n"),
    );
    let path = dir.join("config.capnp");
    std::fs::write(&path, config)?;
    Ok(path)
}
