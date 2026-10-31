//! Apps run in workerd, Cloudflare's open-source Workers runtime, which the node starts beside itself.
//! Each app is an ES module inside `runtime/shim.js`: a Worker with a Durable Object per space that
//! holds the app's views in SQLite. The node replays entries into them and asks them to run operations.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;

use crate::entry::{Content, Key, hex};

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
    pub space: String,
    pub device: String,
    pub user: String,
    pub name: String,
}

/// An entry as an app's `on` handlers see it.
#[derive(Clone, Debug, Serialize)]
pub struct AppEntry {
    pub app: String,
    pub kind: String,
    pub author: String,
    pub user: String,
    pub name: String,
    pub time: u64,
    pub id: String,
    pub data: Value,
}

pub struct Outcome {
    pub result: Value,
    pub appends: Vec<Content>,
}

/// One app's part of a replay: its new entries, or all of them into fresh views.
pub struct Replay {
    pub app: String,
    pub fresh: bool,
    pub entries: Vec<AppEntry>,
}

#[derive(Clone)]
pub struct Apps(Arc<Runtime>);

struct Runtime {
    base: String,
    names: Vec<String>,
    /// The running workerd's process id, so a hung one can be stopped and started again.
    pid: Mutex<Option<u32>>,
}

impl Apps {
    /// Starts workerd with these apps, (name, ES module source), keeping its files in `dir`.
    pub async fn start(dir: PathBuf, apps: Vec<(String, String)>) -> Result<Self> {
        tokio::task::spawn_blocking(move || Apps::start_blocking(&dir, apps)).await?
    }

    fn start_blocking(dir: &Path, apps: Vec<(String, String)>) -> Result<Self> {
        let workerd = workerd()?;
        std::fs::create_dir_all(dir.join("storage"))?;
        stop_previous(dir);
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let config = write_config(dir, &apps, port)?;
        let runtime = Arc::new(Runtime { base: format!("http://127.0.0.1:{port}"), names: apps.into_iter().map(|(n, _)| n).collect(), pid: Mutex::new(None) });
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
                    tracing::warn!("the app runtime stopped ({status:?}); starting it again");
                }
                Err(err) => tracing::warn!("couldn't start the app runtime: {err}"),
            }
            std::thread::sleep(Duration::from_secs(1));
        })?;
        let started = Instant::now();
        while ureq::get(format!("{}/ready", runtime.base)).call().is_err() {
            ensure!(started.elapsed() < Duration::from_secs(60), "the app runtime didn't start; see {}", dir_log(&runtime));
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(Apps(runtime))
    }

    pub fn names(&self) -> &[String] {
        &self.0.names
    }

    async fn ask(&self, app: &str, what: &str, space: Option<Key>, body: Value) -> Result<(Value, Vec<Content>)> {
        if !self.0.names.iter().any(|n| n == app) {
            bail!("no app named {app}");
        }
        let url = match space {
            Some(s) => format!("{}/{app}/{what}?space={}", self.0.base, hex(&s)),
            None => format!("{}/{app}/{what}", self.0.base),
        };
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
        let appends = reply["appends"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| Content { app: app.into(), kind: a["kind"].as_str().unwrap_or("").into(), data: a["data"].clone() })
            .collect();
        Ok((reply["ok"].clone(), appends))
    }

    pub async fn replay(&self, space: Key, jobs: Vec<Replay>) -> Result<()> {
        for job in jobs {
            if job.entries.is_empty() && !job.fresh {
                continue;
            }
            if let Err(err) = self.ask(&job.app, "replay", Some(space), json!({ "fresh": job.fresh, "entries": job.entries })).await {
                tracing::warn!("{} replay: {err:#}", job.app);
            }
        }
        Ok(())
    }

    pub async fn call(&self, space: Key, app: &str, op: &str, input: Value, me: Me, now: u64) -> Result<Outcome> {
        let (result, appends) = self.ask(app, "call", Some(space), json!({ "op": op, "input": input, "me": me, "now": now })).await?;
        Ok(Outcome { result, appends })
    }

    pub async fn widget(&self, space: Key, app: &str, name: &str, me: Me, now: u64) -> Result<Value> {
        Ok(self.ask(app, "widget", Some(space), json!({ "name": name, "me": me, "now": now })).await?.0)
    }

    pub async fn hook(&self, space: Key, app: &str, name: &str, input: Value, me: Me, now: u64) -> Result<Value> {
        Ok(self.ask(app, "hook", Some(space), json!({ "name": name, "input": input, "me": me, "now": now })).await?.0)
    }

    /// A screen's HTML, as the app wrote it.
    pub async fn screen(&self, app: &str, name: &str) -> Result<String> {
        let (html, _) = self.ask(app, &format!("screen?name={name}"), None, json!({})).await?;
        html.as_str().map(str::to_string).context("the screen isn't text")
    }

    pub async fn describe(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for app in self.0.names.clone() {
            match self.ask(&app, "describe", None, json!({})).await {
                Ok((d, _)) => out.push(d),
                Err(err) => tracing::warn!("{app}: {err:#}"),
            }
        }
        Ok(out)
    }
}

impl Runtime {
    /// Stops a hung runtime; the watcher starts it again.
    fn restart(&self) {
        if let Some(pid) = *self.pid.lock().unwrap() {
            let _ = std::process::Command::new("kill").arg("-9").arg(pid.to_string()).status();
        }
    }
}

fn dir_log(runtime: &Runtime) -> String {
    format!("workerd.log (runtime at {})", runtime.base)
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
fn write_config(dir: &Path, apps: &[(String, String)], port: u16) -> Result<PathBuf> {
    let modules = dir.join("apps");
    let _ = std::fs::remove_dir_all(&modules);
    std::fs::write(dir.join("router.js"), include_str!("runtime/router.js"))?;
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut services = vec![
        "(name = \"router\", worker = .router)".to_string(),
        format!("(name = \"storage\", disk = (path = {}, writable = true))", quote(&dir.join("storage").to_string_lossy())),
    ];
    let mut bindings = Vec::new();
    let mut workers = Vec::new();
    for (i, (name, source)) in apps.iter().enumerate() {
        ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'), "app names are lowercase words joined by -");
        let app_dir = modules.join(name);
        std::fs::create_dir_all(&app_dir)?;
        std::fs::write(app_dir.join("app.js"), source)?;
        std::fs::write(app_dir.join("shim.js"), include_str!("runtime/shim.js"))?;
        services.push(format!("(name = {}, worker = .app{i})", quote(&format!("app-{name}"))));
        bindings.push(format!("(name = {}, service = {})", quote(name), quote(&format!("app-{name}"))));
        workers.push(format!(
            "const app{i} :Workerd.Worker = (\n  modules = [(name = \"shim.js\", esModule = embed {}), (name = \"app.js\", esModule = embed {})],\n  compatibilityDate = \"{COMPATIBILITY_DATE}\",\n  bindings = [(name = \"VIEWS\", durableObjectNamespace = \"Views\")],\n  durableObjectNamespaces = [(className = \"Views\", uniqueKey = {}, enableSql = true)],\n  durableObjectStorage = (localDisk = \"storage\"),\n);",
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
