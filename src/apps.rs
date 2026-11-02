//! Apps run in workerd, Cloudflare's open-source Workers runtime, which the node starts beside itself.
//! Each deployed app is an ES module inside `runtime/shim.js`, which asks the node to run its queries and writes on
//! its databases (`src/db.rs`), and runs its operations, widgets, hooks and screens.
//! When the deploys change, the node writes the config again and restarts workerd.
//! Every request an app makes (its `fetch`es, and the shim's asks of the node) goes to the node's outbound port,
//! `src/outbound.rs`, carrying the token of the call it's made in.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;


use crate::call::{Append, appends};
use crate::members::Role;

/// The workerd build every node runs, and each platform's npm package of it with its sha512.
const WORKERD: &str = "1.20261031.1";
const PACKAGES: &[(&str, &str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "linux-64", "42mhHUcRRqs0FI3Ej6HybrxuCeOoHiH9A1h8CC341bT/FSNBzJZWNLiiCHaQQH+dpYU+Vz/bC6DurKnuJZTW3Q=="),
    ("aarch64-unknown-linux-gnu", "linux-arm64", "ZGeXhiFYcUoKvQVLARBowkyjEhymW7VUMk69f4TD2B7UFj4ijq6YEDWcxm0oQLw7uTI9W2915Bnlp8sCixLYag=="),
    ("aarch64-apple-darwin", "darwin-arm64", "rN2OJjYA9DiHJPub+J1D1TCTw0+g40M+hfMUsD4sPm1qBSWyqG/H9M40ww2IKB16F8m+/0O9GmdlzYI932GduA=="),
    ("x86_64-apple-darwin", "darwin-64", "qjXbqX7J50hqThWbYzT2c5yxr5Ymq6m6WETgPGqOAbIY+aZ11NQhnEV1Vsd/3Ljt97ctApQ1v8aCwX4Fa16nYQ=="),
];
pub const COMPATIBILITY_DATE: &str = "2026-10-01";
/// How long an app may take to answer before its runtime is restarted.
const TIME_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize)]
pub struct Me {
    pub key: String,
    pub user: String,
    pub name: String,
    pub role: Option<Role>,
    /// The user's name.
    pub user_name: String,
    /// The agent that asked (an MCP client's name, like "Claude Code"), when one did.
    pub agent: Option<String>,
}

/// A scope a call may reach, and the caller's role there.
#[derive(Clone, Debug, Serialize)]
pub struct ScopeHandle {
    pub id: String,
    pub name: String,
    pub role: Option<Role>,
}

/// Who a call, widget or hook runs for, and what it may reach.
#[derive(Clone, Debug, Serialize)]
pub struct Context {
    pub me: Me,
    pub now: u64,
    pub scopes: Vec<ScopeHandle>,
    /// A call on data opened for its caller: appends fail.
    pub read_only: bool,
    /// What the call's requests carry, so the node knows whose they are (see `src/outbound.rs`).
    pub token: String,
    /// The app whose `ctx.apps` made the call, if one did.
    pub from: Option<String>,
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
    /// The node's port that apps' requests go out through.
    outbound: u16,
    /// The apps loaded, (name, module source), and how many times the runtime was configured.
    loaded: Mutex<(Vec<(String, String)>, u64)>,
    /// The running workerd's process id, so a hung one can be stopped and started again.
    pid: Mutex<Option<u32>>,
    /// What each loaded app says it is, once asked.
    described: Mutex<std::collections::HashMap<String, Value>>,
}

impl Apps {
    /// Starts workerd with no apps yet, keeping its files in `dir`; apps' requests go out through the node's
    /// `outbound` port.
    pub async fn start(dir: PathBuf, outbound: u16) -> Result<Self> {
        tokio::task::spawn_blocking(move || Apps::start_blocking(&dir, outbound)).await?
    }

    fn start_blocking(dir: &Path, outbound: u16) -> Result<Self> {
        let workerd = workerd()?;
        stop_previous(dir);
        // Where apps' stores were kept before each app's databases were in the bucket.
        let _ = std::fs::remove_dir_all(dir.join("storage"));
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let config = write_config(dir, &[], port, outbound, 0)?;
        let runtime = Arc::new(Runtime { dir: dir.to_path_buf(), base: format!("http://127.0.0.1:{port}"), outbound, loaded: Mutex::new((Vec::new(), 0)), pid: Mutex::new(None), described: Mutex::new(Default::default()) });
        let watch = runtime.clone();
        let (dir, log) = (dir.to_path_buf(), dir.join("workerd.log"));
        std::thread::Builder::new().name("workerd".into()).spawn(move || loop {
            let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).expect("workerd log");
            let mut command = std::process::Command::new(&workerd);
            command.arg("serve").arg(&config).stdout(out.try_clone().unwrap()).stderr(out);
            // On Linux the runtime dies with its node; elsewhere the next start stops it (stop_previous).
            #[cfg(target_os = "linux")]
            unsafe {
                use std::os::unix::process::CommandExt;
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
            match command.spawn() {
                Ok(mut child) => {
                    *watch.pid.lock().unwrap() = Some(child.id());
                    let _ = std::fs::write(dir.join("workerd.pid"), child.id().to_string());
                    let status = child.wait();
                    tracing::debug!("the app runtime stopped ({status:?}); starting it again");
                }
                Err(err) => tracing::warn!("couldn't start the app runtime: {err}"),
            }
            std::thread::sleep(Duration::from_millis(100));
        })?;
        runtime.wait_ready(0)?;
        Ok(Apps(runtime))
    }

    /// Runs exactly these apps, (name, module source), restarting the runtime if they changed.
    pub async fn load(&self, apps: Vec<(String, String)>) -> Result<()> {
        let runtime = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let generation = {
                let mut loaded = runtime.loaded.lock().unwrap();
                if loaded.0 == apps {
                    return Ok(());
                }
                loaded.1 += 1;
                let port = runtime.base.rsplit(':').next().unwrap().parse()?;
                write_config(&runtime.dir, &apps, port, runtime.outbound, loaded.1)?;
                loaded.0 = apps;
                runtime.described.lock().unwrap().clear();
                loaded.1
            };
            runtime.restart();
            runtime.wait_ready(generation)
        })
        .await?
    }

    /// The port workerd serves the node on.
    pub fn port(&self) -> u16 {
        self.0.base.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0)
    }

    pub fn names(&self) -> Vec<String> {
        self.0.loaded.lock().unwrap().0.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn has(&self, app: &str) -> bool {
        self.0.loaded.lock().unwrap().0.iter().any(|(n, _)| n == app)
    }

    async fn ask(&self, app: &str, what: &str, body: Value) -> Result<Value> {
        self.ask_for(app, what, body, TIME_LIMIT).await
    }

    /// Asks the runtime, stopping it if it takes longer than `limit`.
    async fn ask_for(&self, app: &str, what: &str, body: Value, limit: Duration) -> Result<Value> {
        if !self.has(app) {
            bail!("no app named {app} runs here");
        }
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
                Err(ureq::Error::Timeout(_)) => {
                    runtime.restart();
                    bail!("the app took longer than {}s; it was stopped", limit.as_secs())
                }
                Err(err) => Err(err.into()),
            }
        })
        .await??;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("the app failed"));
        }
        Ok(reply)
    }

    /// Runs an operation, for at most `limit`: longer for an app that waits on sidecars.
    pub async fn call(&self, app: &str, op: &str, input: Value, ctx: &Context, limit: Duration) -> Result<Outcome> {
        let reply = self.ask_for(app, "call", json!({ "op": op, "input": input, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": ctx.read_only, "token": ctx.token, "from": ctx.from }), limit).await?;
        Ok(Outcome { result: reply["ok"].clone(), appends: appends(&reply["appends"])?, later: reply["later"] == true })
    }

    pub async fn badge(&self, app: &str, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "badge", json!({ "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token })).await?["ok"].clone())
    }

    /// A widget's tree at `size` (`small`, `medium`, `large` or `xl`), which its render sees as `ctx.size`, with the
    /// options a person chose for it (`ctx.options`, its own defaults for the rest).
    pub async fn widget(&self, app: &str, name: &str, size: &str, options: &Value, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "widget", json!({ "name": name, "size": size, "options": options, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token })).await?["ok"].clone())
    }

    /// A widget's options, each `{ name, title, choices, default }`, with choices the app works out from its data.
    pub async fn widget_options(&self, app: &str, name: &str, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "options", json!({ "name": name, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token })).await?["ok"].clone())
    }

    pub async fn hook(&self, app: &str, name: &str, input: Value, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "hook", json!({ "name": name, "input": input, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true, "token": ctx.token })).await?["ok"].clone())
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
        let generation = self.0.loaded.lock().unwrap().1;
        let d = self.ask(app, "describe", json!({})).await?["ok"].clone();
        if self.0.loaded.lock().unwrap().1 == generation {
            self.0.described.lock().unwrap().insert(app.to_string(), d.clone());
        }
        Ok(d)
    }
}

impl Runtime {
    /// Stops the runtime; the watcher starts it again, with the config as it is now.
    fn restart(&self) {
        if let Some(pid) = *self.pid.lock().unwrap() {
            let _ = std::process::Command::new("kill").arg("-9").arg(pid.to_string()).status();
        }
    }

    /// Waits until the runtime answers with the config of `generation`.
    fn wait_ready(&self, generation: u64) -> Result<()> {
        let started = Instant::now();
        loop {
            if let Ok(mut r) = ureq::get(format!("{}/ready", self.base)).call() {
                if r.body_mut().read_to_string().is_ok_and(|g| g.trim() == generation.to_string()) {
                    return Ok(());
                }
            }
            ensure!(started.elapsed() < Duration::from_secs(60), "the app runtime didn't start; see {}", self.dir.join("workerd.log").display());
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A runtime left behind by an earlier run of the node would still be running; stop it.
fn stop_previous(dir: &Path) {
    let Ok(pid) = std::fs::read_to_string(dir.join("workerd.pid")) else { return };
    let pid = pid.trim();
    let is_workerd = std::process::Command::new("ps").args(["-p", pid, "-o", "comm="]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("workerd"));
    if is_workerd {
        let _ = std::process::Command::new("kill").arg(pid).status();
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// The workerd binary for this platform, downloaded once into the cache and checked against its pinned hash.
pub fn workerd() -> Result<PathBuf> {
    let (_, package, integrity) = PACKAGES
        .iter()
        .find(|(target, _, _)| *target == crate::config::TARGET)
        .ok_or_else(|| anyhow!("apps can't run on {} yet: workerd has no build for it", crate::config::TARGET))?;
    let url = format!("https://registry.npmjs.org/@cloudflare/workerd-{package}/-/workerd-{package}-{WORKERD}.tgz");
    pinned(&format!("workerd-{WORKERD}"), "workerd", &url, integrity)
}

/// A program from an npm package's tarball (`package/bin/<bin>`), downloaded once into the node's cache and checked
/// against the package's pinned sha512.
pub fn pinned(dir: &str, bin: &str, url: &str, integrity: &str) -> Result<PathBuf> {
    let dir = crate::cache().join(dir);
    let path = dir.join(bin);
    if path.exists() {
        return Ok(path);
    }
    tracing::info!("downloading {url}");
    let tarball = ureq::get(url).call()?.body_mut().with_config().limit(400 << 20).read_to_vec()?;
    let digest = data_encoding::BASE64.encode(&sha2::Sha512::digest(&tarball)[..]);
    ensure!(digest == *integrity, "the download of {bin} doesn't match its pinned hash");
    // Unpack into a folder of our own and move the binary into place in one step,
    // so nodes starting at the same time can't see a half-written one.
    // Several nodes in one process (as in tests) can download at once, so the folder is this call's alone.
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.with_extension(format!("{}-{call}.part", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    std::fs::File::create(tmp.join("package.tgz"))?.write_all(&tarball)?;
    let ok = std::process::Command::new("tar").args(["-xzf", "package.tgz", "--strip-components=2", &format!("package/bin/{bin}")]).current_dir(&tmp).status()?;
    ensure!(ok.success(), "couldn't unpack {bin}");
    std::fs::create_dir_all(&dir)?;
    std::fs::rename(tmp.join(bin), &path)?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(path)
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
        // Every request an app makes goes to the node, which makes it (filling in secrets) or answers it itself.
        format!("(name = \"outbound\", external = (address = \"127.0.0.1:{outbound}\", http = (style = host, forwardedProtoHeader = \"x-forwarded-proto\")))"),
    ];
    let mut bindings = Vec::new();
    let mut workers = Vec::new();
    for (i, (name, source)) in apps.iter().enumerate() {
        ensure!(crate::members::valid_app_name(name), "app names are lowercase words joined by -");
        let app_dir = modules.join(name);
        std::fs::create_dir_all(&app_dir)?;
        std::fs::write(app_dir.join("app.js"), source)?;
        std::fs::write(app_dir.join("shim.js"), include_str!("runtime/shim.js"))?;
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
