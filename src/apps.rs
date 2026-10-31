//! Apps run in workerd, Cloudflare's open-source Workers runtime, which the node starts beside itself.
//! Each deployed app is an ES module inside `runtime/shim.js`: a Worker with a Durable Object per scope that holds the
//! app's store of that scope in SQLite. The node replays entries into them and asks them to run operations.
//! When the deploys change, the node writes the config again and restarts workerd; the stores stay on disk.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;

use std::collections::BTreeMap;

use crate::entry::Content;
use crate::members::Role;

/// The workerd build every node runs, and each platform's npm package of it with its sha512.
const WORKERD: &str = "1.20261031.1";
const PACKAGES: &[(&str, &str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "linux-64", "42mhHUcRRqs0FI3Ej6HybrxuCeOoHiH9A1h8CC341bT/FSNBzJZWNLiiCHaQQH+dpYU+Vz/bC6DurKnuJZTW3Q=="),
    ("aarch64-unknown-linux-gnu", "linux-arm64", "ZGeXhiFYcUoKvQVLARBowkyjEhymW7VUMk69f4TD2B7UFj4ijq6YEDWcxm0oQLw7uTI9W2915Bnlp8sCixLYag=="),
    ("aarch64-apple-darwin", "darwin-arm64", "rN2OJjYA9DiHJPub+J1D1TCTw0+g40M+hfMUsD4sPm1qBSWyqG/H9M40ww2IKB16F8m+/0O9GmdlzYI932GduA=="),
    ("x86_64-apple-darwin", "darwin-64", "qjXbqX7J50hqThWbYzT2c5yxr5Ymq6m6WETgPGqOAbIY+aZ11NQhnEV1Vsd/3Ljt97ctApQ1v8aCwX4Fa16nYQ=="),
];
const COMPATIBILITY_DATE: &str = "2026-10-01";
/// How long an app may take to answer before its runtime is restarted.
const TIME_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize)]
pub struct Me {
    pub key: String,
    pub user: String,
    pub name: String,
    pub role: Option<Role>,
}

/// An entry as an app's `on` handlers see it.
#[derive(Clone, Debug, Serialize)]
pub struct AppEntry {
    #[serde(skip)]
    pub app: String,
    pub id: String,
    pub scope: String,
    pub kind: String,
    pub user: String,
    pub name: String,
    pub role: Option<Role>,
    pub groups: BTreeMap<String, Role>,
    pub time: u64,
    pub data: Value,
    pub blobs: Vec<String>,
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
}

/// What a call asks the node to write once it returns.
pub enum Append {
    Entry { scope: String, content: Content },
    Install { app: String, scope: String, installed: bool },
}

pub struct Outcome {
    pub result: Value,
    pub appends: Vec<Append>,
}

/// One app's store of one scope: its new entries, or all of them into an empty store.
pub struct Replay {
    pub app: String,
    pub scope: String,
    pub fresh: bool,
    pub entries: Vec<AppEntry>,
}

#[derive(Clone)]
pub struct Apps(Arc<Runtime>);

struct Runtime {
    dir: PathBuf,
    base: String,
    /// The apps loaded, (name, module source), and how many times the runtime was configured.
    loaded: Mutex<(Vec<(String, String)>, u64)>,
    /// The running workerd's process id, so a hung one can be stopped and started again.
    pid: Mutex<Option<u32>>,
}

impl Apps {
    /// Starts workerd with no apps yet, keeping its files in `dir`.
    pub async fn start(dir: PathBuf) -> Result<Self> {
        tokio::task::spawn_blocking(move || Apps::start_blocking(&dir)).await?
    }

    fn start_blocking(dir: &Path) -> Result<Self> {
        let workerd = workerd()?;
        std::fs::create_dir_all(dir.join("storage"))?;
        stop_previous(dir);
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let config = write_config(dir, &[], port, 0)?;
        let runtime = Arc::new(Runtime { dir: dir.to_path_buf(), base: format!("http://127.0.0.1:{port}"), loaded: Mutex::new((Vec::new(), 0)), pid: Mutex::new(None) });
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

    /// Runs exactly these apps, (name, module source), restarting the runtime if they changed. Stores stay on disk.
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
                write_config(&runtime.dir, &apps, port, loaded.1)?;
                loaded.0 = apps;
                loaded.1
            };
            runtime.restart();
            runtime.wait_ready(generation)
        })
        .await?
    }

    pub fn names(&self) -> Vec<String> {
        self.0.loaded.lock().unwrap().0.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn has(&self, app: &str) -> bool {
        self.0.loaded.lock().unwrap().0.iter().any(|(n, _)| n == app)
    }

    async fn ask(&self, app: &str, what: &str, body: Value) -> Result<Value> {
        if !self.has(app) {
            bail!("no app named {app} runs here");
        }
        let url = format!("{}/{app}/{what}", self.0.base);
        let runtime = self.0.clone();
        let reply: Value = tokio::task::spawn_blocking(move || -> Result<Value> {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIME_LIMIT)).build().into();
            match agent.post(&url).send_json(&body) {
                Ok(mut resp) => Ok(resp.body_mut().with_config().limit(64 << 20).read_json()?),
                Err(ureq::Error::Timeout(_)) => {
                    runtime.restart();
                    bail!("the app took longer than {}s; it was stopped", TIME_LIMIT.as_secs())
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

    pub async fn replay(&self, jobs: Vec<Replay>) {
        for job in jobs {
            if job.entries.is_empty() && !job.fresh {
                continue;
            }
            if let Err(err) = self.ask(&job.app, "replay", json!({ "scope": job.scope, "fresh": job.fresh, "entries": job.entries })).await {
                tracing::warn!("{} replay of {}: {err:#}", job.app, job.scope);
            }
        }
    }

    /// Deletes every app's store of a scope.
    pub async fn drop_scope(&self, scope: &str) {
        for app in self.names() {
            if let Err(err) = self.ask(&app, "drop", json!({ "scope": scope })).await {
                tracing::warn!("{app}: dropping {scope}: {err:#}");
            }
        }
    }

    pub async fn call(&self, app: &str, op: &str, input: Value, ctx: &Context) -> Result<Outcome> {
        let reply = self.ask(app, "call", json!({ "op": op, "input": input, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": ctx.read_only })).await?;
        let mut appends = Vec::new();
        for a in reply["appends"].as_array().into_iter().flatten() {
            let scope = a["scope"].as_str().unwrap_or("").to_string();
            match a["install"].as_str() {
                Some(name) => appends.push(Append::Install { app: name.into(), scope, installed: a["installed"] != false }),
                None => {
                    let blobs = a["blobs"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
                    let content = Content { app: app.into(), kind: a["kind"].as_str().unwrap_or("").into(), data: a["data"].clone(), blobs };
                    appends.push(Append::Entry { scope, content });
                }
            }
        }
        Ok(Outcome { result: reply["ok"].clone(), appends })
    }

    pub async fn widget(&self, app: &str, name: &str, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "widget", json!({ "name": name, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true })).await?["ok"].clone())
    }

    pub async fn hook(&self, app: &str, name: &str, input: Value, ctx: &Context) -> Result<Value> {
        Ok(self.ask(app, "hook", json!({ "name": name, "input": input, "me": ctx.me, "now": ctx.now, "scopes": ctx.scopes, "read_only": true })).await?["ok"].clone())
    }

    /// A screen's HTML, as the app wrote it.
    pub async fn screen(&self, app: &str, name: &str) -> Result<String> {
        let reply = self.ask(app, &format!("screen?name={name}"), json!({})).await?;
        reply["ok"].as_str().map(str::to_string).context("the screen isn't text")
    }

    pub async fn describe(&self, app: &str) -> Result<Value> {
        Ok(self.ask(app, "describe", json!({})).await?["ok"].clone())
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

/// A runtime left behind by an earlier run of the node would hold on to its storage; stop it.
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
fn workerd() -> Result<PathBuf> {
    let (_, package, integrity) = PACKAGES
        .iter()
        .find(|(target, _, _)| *target == crate::brand::TARGET)
        .ok_or_else(|| anyhow!("apps can't run on {} yet: workerd has no build for it", crate::brand::TARGET))?;
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    let dir = cache.join(crate::brand::CLI).join(format!("workerd-{WORKERD}"));
    let bin = dir.join("workerd");
    if bin.exists() {
        return Ok(bin);
    }
    tracing::info!("downloading the app runtime (workerd {WORKERD}, about 50 MB)");
    let url = format!("https://registry.npmjs.org/@cloudflare/workerd-{package}/-/workerd-{package}-{WORKERD}.tgz");
    let tarball = ureq::get(&url).call()?.body_mut().with_config().limit(400 << 20).read_to_vec()?;
    let digest = data_encoding::BASE64.encode(&sha2::Sha512::digest(&tarball)[..]);
    ensure!(digest == *integrity, "the downloaded runtime doesn't match its pinned hash");
    // Unpack into a folder of our own and move the binary into place in one step,
    // so nodes starting at the same time can't see a half-written runtime.
    let tmp = dir.with_extension(format!("{}.part", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    std::fs::File::create(tmp.join("package.tgz"))?.write_all(&tarball)?;
    let ok = std::process::Command::new("tar").args(["-xzf", "package.tgz", "--strip-components=2", "package/bin/workerd"]).current_dir(&tmp).status()?;
    ensure!(ok.success(), "couldn't unpack the runtime");
    std::fs::create_dir_all(&dir)?;
    std::fs::rename(tmp.join("workerd"), &bin)?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(bin)
}

/// Writes each app's modules and the workerd config that runs them all behind one router.
fn write_config(dir: &Path, apps: &[(String, String)], port: u16, generation: u64) -> Result<PathBuf> {
    let modules = dir.join("apps");
    let _ = std::fs::remove_dir_all(&modules);
    std::fs::create_dir_all(&modules)?;
    std::fs::write(dir.join("router.js"), include_str!("runtime/router.js").replace("GENERATION", &generation.to_string()))?;
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut services = vec![
        "(name = \"router\", worker = .router)".to_string(),
        format!("(name = \"storage\", disk = (path = {}, writable = true))", quote(&dir.join("storage").to_string_lossy())),
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
            "const app{i} :Workerd.Worker = (\n  modules = [(name = \"shim.js\", esModule = embed {}), (name = \"app.js\", esModule = embed {})],\n  compatibilityDate = \"{COMPATIBILITY_DATE}\",\n  bindings = [(name = \"STORES\", durableObjectNamespace = \"Store\")],\n  durableObjectNamespaces = [(className = \"Store\", uniqueKey = {}, enableSql = true)],\n  durableObjectStorage = (localDisk = \"storage\"),\n);",
            quote(&format!("apps/{name}/shim.js")),
            quote(&format!("apps/{name}/app.js")),
            quote(name),
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
