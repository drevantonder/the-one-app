//! oned (`<cli>d`): a computer's always-on program, this program run by that name; the `<cli>` command line runs, answers
//! and exits. It holds no key, and is a user of its onet like any other program. It keeps two programs of its own
//! running, starting each again when it stops: the computer's onet (`<cli>t`), which holds the device's key and answers
//! its programs, and its app runtime (`<cli>-apps --listen-socket=`, app-runtime/), an endpoint of its own key that onet asks on
//! a loopback address oned gives both. It never takes a build by itself: `one update`, which Update asks of the device,
//! replaces its program and has it exit, for its supervisor to start it again on the new one.
//!
//! It runs the device's services (`keep_parts`), asking its onet what it needs (`/api/services`, which onet passes to its
//! app runtime): each a process of oned's own (`oned <name>`, `part::run`) with its own key, which it lists as its
//! device's, reached only through the device's app runtime, whose loopback address oned gives it. Each is on while the device's latest `service` step for it says so; with none, off; and each is started
//! again when it stops, the rest left running, whatever onet and the runtime do. Only the device's user writes that step
//! (its switch in Settings' Devices), and the services start and stop as it changes. Each keeps its own key in its folder
//! (`<home>/services/<name>`), so it answers only its own user whoever else's calls the device runs, and one failing
//! leaves the rest; nothing approves it by a code, since its device lists it. Its program is its app's own
//! (`apps/<name>/service/`), and only a build its app's latest deploy names for this platform runs (`bundle::SERVICES`),
//! which `<cli> deploy` builds where it's run, or, for one whose deploy names none, as a built-in app's, the newest
//! `<cli> update` published (`oned_update::latest`). It's fetched, its hash checked and put at `~/.local/bin/<cli>-<name>` when
//! it's switched on and again when another is named or a newer one's published; a Mac service in Swift is an app of its
//! own (`helper`), unpacked in the home. What the device can run (`parts`) and each one's state (on, off, starting, stopping,
//! failed and why) and how it runs (in its sandbox as its app declares, told to it in its folder, `declared.json`;
//! without one and why; or as the user) its onet gives its own app runtime, which answers another device that asks
//! as Settings' Devices shows it. Removing a device removes its services, and can erase what it keeps, which the device does
//! itself, so only while it's reachable. A phone runs none: its app is the device.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use ::services::{Build, Wanted};

pub mod part;
pub mod publish;

use ::custom_build::config::{CLI, NAME};

pub fn invoked() -> bool {
    platform::os::invoked_as().is_some_and(|me| me == ::custom_build::programs::oned())
}

pub fn command(args: &[&str]) -> std::io::Result<std::process::Command> {
    let mut command = platform::os::named(&::custom_build::programs::oned())?;
    command.args(args);
    Ok(command)
}

/// The program that offers the service `name`: the one built beside this program (`carried`), a Mac helper app's
/// (`helper`), else where oned fetches one.
pub fn program(name: &str) -> PathBuf {
    carried(name).or_else(|| helper(name)).unwrap_or_else(|| fetched(name))
}

/// Where oned puts a service's program it fetched (`Mode::bin`).
pub fn fetched(name: &str) -> PathBuf {
    platform::mode::here().bin().join(format!("{CLI}-{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A service's program built with this one and beside it, which oned runs as it is and fetches none of: a checkout's
/// (`one-dev throwaway`), which Cargo builds beside it.
pub fn carried(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(format!("{CLI}-{name}{}", std::env::consts::EXE_SUFFIX));
    (path.exists() && path != fetched(name)).then_some(path)
}

/// Where a Mac service in Swift runs from: an app of its own (`<Title> — <App>.app`, services/sdk/mac-helpers.sh), as the
/// record publishes it zipped and oned unpacks it (`put`), in this device's home. macOS asks for and records its
/// permissions under its own name, and keeps them across builds, since they're the helper's by its id and team, not
/// where it is. Its program, once it's here.
pub fn helper(name: &str) -> Option<PathBuf> {
    let program = format!("{CLI}-{name}");
    helpers().into_iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy() == program))
}

/// The folder a Mac's helper apps are unpacked in.
fn helper_apps() -> PathBuf {
    platform::home().join("helpers")
}

/// The programs of the helper apps here.
fn helpers() -> Vec<PathBuf> {
    let apps = std::fs::read_dir(helper_apps()).into_iter().flatten().flatten().map(|e| e.path().join("Contents/MacOS"));
    apps.flat_map(std::fs::read_dir).flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&format!("{CLI}-")))).collect()
}

/// The services this device could run as processes of oned's: those with a release for its platform (`released`),
/// which it fetches when one is switched on, and those whose programs are here (carried or fetched) that ran here
/// before or are switched on (`on`), not every `<cli>-` program there.
fn offered(released: &BTreeSet<String>, on: &BTreeMap<String, u64>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let dirs = [fetched("x"), std::env::current_exe().unwrap_or_default()].map(|p| p.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    let prefix = format!("{CLI}-");
    let files = dirs.iter().flat_map(std::fs::read_dir).flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string());
    for file in files.chain(helpers().iter().filter_map(|p| Some(p.file_name()?.to_string_lossy().to_string()))) {
        let Some(name) = file.strip_suffix(std::env::consts::EXE_SUFFIX).and_then(|f| f.strip_prefix(&prefix)).filter(|n| ::scope::scopes::valid_app_name(n)) else { continue };
        if on.contains_key(name) || ::services::folder(name).is_dir() {
            names.insert(name.to_string());
        }
    }
    names.extend(released.iter().cloned());
    // The app runtime's program is `<cli>-apps` and its build is released beside the services', but oned keeps it itself.
    names.remove(::scope::scopes::APPS_PART);
    names
}

/// What a part that runs as a process of oned's is doing now, while it isn't simply on or off: being set up (its
/// program fetched), starting or stopping; or what went wrong.
#[derive(Clone, Debug, Default, PartialEq)]
struct Now {
    doing: Option<&'static str>,
    problem: Option<String>,
}

/// How a part runs, as its app declares: without its sandbox, saying why, or as the user.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    sandbox: Option<String>,
    user: bool,
}

/// What `keep_parts` says of the parts, which its onet tells the network's app runtimes: which could run as processes,
/// what each is doing, and how each running runs.
#[derive(Default)]
struct Said {
    offered: BTreeSet<String>,
    now: BTreeMap<String, Now>,
    held: BTreeMap<String, Held>,
}

impl Said {
    fn now(&self, name: &str) -> Now {
        self.now.get(name).cloned().unwrap_or_default()
    }

    fn say(&mut self, name: &str, now: Now) {
        if now == Now::default() {
            self.now.remove(name);
        } else {
            self.now.insert(name.to_string(), now);
        }
    }

    /// Each part, as Settings › Devices shows it: `apps`, the app runtime, and each service this device could run as
    /// a process, with whether it's switched on, what it's doing and how it runs.
    fn parts(&self, wanted: &Wanted) -> Vec<::scope::state::Part> {
        use ::scope::scopes::APPS_PART;
        use ::scope::state::Part;
        let mut out = BTreeMap::new();
        out.insert(APPS_PART.to_string(), Part { on: !wanted.off.contains(APPS_PART), ..Part::off(APPS_PART) });
        for name in &self.offered {
            let on = wanted.on.contains_key(name);
            let now = self.now(name);
            let held = self.held.get(name);
            out.insert(name.clone(), Part { on, doing: now.doing.map(str::to_string), problem: now.problem, sandbox: held.and_then(|h| h.sandbox.clone()), user: held.is_some_and(|h| h.user), ..Part::off(name) });
        }
        out.into_values().collect()
    }
}

/// This computer's onet, as oned asks it: at the API's own address, as this computer's own program, with the token in
/// the home. What it doesn't answer itself it passes to the device's own app runtime.
struct Onet {
    home: PathBuf,
    agent: ureq::Agent,
}

impl Onet {
    fn new(home: PathBuf) -> Onet {
        let agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(120))).build().into();
        Onet { home, agent }
    }

    /// `method` of `path`, with a JSON body: → what it answered, once that's a 200.
    fn ask(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>> {
        let token = ::api::local::local_token(&self.home).context("onet hasn't started yet")?;
        let mut request = ureq::http::Request::builder().method(method).uri(format!("http://{}{path}", ::api::local::addr())).header(::custom_build::protocol::header::LOCAL_TOKEN, token);
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let mut answer = self.agent.run(request.body(body.map(|b| b.to_string().into_bytes()).unwrap_or_default())?)?;
        let status = answer.status().as_u16();
        let bytes = answer.body_mut().with_config().limit(2 << 30).read_to_vec()?;
        if status != 200 {
            let said: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            bail!("{}", said["error"].as_str().map_or_else(|| format!("{} answered {status}", ::custom_build::programs::onet()), str::to_string));
        }
        Ok(bytes)
    }

    fn json<T: serde::de::DeserializeOwned>(&self, method: &str, path: &str, body: Option<&Value>) -> Result<T> {
        Ok(serde_json::from_slice(&self.ask(method, path, body)?)?)
    }
}

/// What's put in a service's folder of the build its program is: its blob, and the release's commit time, or none for
/// the build a deploy named (`deploy <blob>`).
fn taken(name: &str) -> (Option<u64>, String) {
    let kept = std::fs::read_to_string(::services::folder(name).join("release")).unwrap_or_default();
    let (committed, blob) = kept.trim().split_once(' ').unwrap_or(("0", ""));
    (committed.parse().ok(), blob.to_string())
}

/// Puts the program of `build`, once its signature is the release key's or its hash the one its deploy names, where
/// oned runs it, in one step so it never starts half of it: a release from the network's address, a deploy's build
/// from the device's app runtime.
fn put(onet: &Onet, address: &str, name: &str, build: &Build) -> anyhow::Result<()> {
    let (bytes, kept) = match build {
        Build::Released(r) => (::oned_update::latest::download(address, r)?, format!("{} {}", r.committed, r.sha256)),
        Build::Deployed(hash) => (onet.ask("GET", &format!("/api/services/builds/{hash}"), None)?, format!("deploy {hash}")),
        Build::None => anyhow::bail!("its deploy names no build for this platform"),
    };
    if bytes.starts_with(b"PK\x03\x04") {
        put_helper(name, &bytes)?;
    } else {
        let path = fetched(name);
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
        let new = path.with_extension("new");
        std::fs::write(&new, bytes)?;
        platform::os::set_mode(&new, 0o755)?;
        std::fs::rename(&new, &path)?;
    }
    let folder = ::services::folder(name);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("release"), &kept)?;
    tracing::info!("oned put {name} {kept} in place");
    Ok(())
}

/// A Mac helper's app, zipped as the record publishes it (`helper`): unpacked beside the one before, which is then
/// swapped out for it by renames. (`ditto` is the Mac's own, which keeps what a signed app is made of.)
fn put_helper(name: &str, zip: &[u8]) -> anyhow::Result<()> {
    let apps = helper_apps();
    let work = apps.join(format!(".{name}.new"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let (file, unpacked) = (work.join("app.zip"), work.join("app"));
    std::fs::write(&file, zip)?;
    let done = std::process::Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(&file).arg(&unpacked).output()?;
    anyhow::ensure!(done.status.success(), "couldn't unpack {name}'s app: {}", String::from_utf8_lossy(&done.stderr).trim());
    let app = std::fs::read_dir(&unpacked)?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "app")).with_context(|| format!("{name}'s download has no app in it"))?;
    let at = apps.join(app.file_name().unwrap_or_default());
    // The one before, found by its program, as its title may have changed.
    if let Some(before) = helper(name).and_then(|p| Some(p.ancestors().nth(3)?.to_path_buf())) {
        std::fs::rename(before, work.join("before"))?;
    }
    if at.exists() {
        std::fs::remove_dir_all(&at)?;
    }
    std::fs::rename(&app, &at)?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

/// How often oned tells its onet what its parts are doing when nothing changed, so a runtime that started since hears.
const TELL_EVERY: Duration = Duration::from_secs(30);

/// Keeps this device's parts that run as processes of their own running as its steps say, asking its onet: when one is
/// switched on, its program fetched from the newest release for this platform if it isn't here (and again when a newer
/// one is published), then started (and again if it stops); stopped when it's switched off; and listed as this
/// device's once each asks. What each is doing meanwhile, and what went wrong, its onet tells the network's app
/// runtimes (`POST /api/services`). Asked to erase what the device keeps, it stops them, and onet and the runtime
/// (`kept`), deletes the home and exits, to start again as nobody's. In a thread of its own for good, so what it starts
/// is oned's child, whatever onet and the runtime do.
fn keep_parts(kept: std::sync::Arc<std::sync::Mutex<[Kept; 2]>>, runtime: String) {
    std::thread::spawn(move || {
        let home = platform::home();
        let onet = std::sync::Arc::new(Onet::new(home.clone()));
        let began = Instant::now();
        let said = std::sync::Arc::new(std::sync::Mutex::new(Said::default()));
        let mut running: BTreeMap<String, (std::process::Child, Instant)> = BTreeMap::new();
        // Those that answered their device since they started.
        let mut up: BTreeSet<String> = BTreeSet::new();
        // What each part last asked, and when it was listed for it: asked again later, it's listed again.
        let mut listed: BTreeMap<String, (String, Instant)> = BTreeMap::new();
        // The build of each service's program for this platform, looked at now and then.
        let mut released: BTreeMap<String, Build> = BTreeMap::new();
        let mut looked: Option<Instant> = None;
        // Programs being fetched, and when fetching each last failed, to try again a minute on.
        let fetching = std::sync::Arc::new(std::sync::Mutex::new(BTreeSet::<String>::new()));
        let failed = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::<String, Instant>::new()));
        // When each part stopped by itself lately: one that keeps stopping says so.
        let mut stops: BTreeMap<String, Vec<Instant>> = BTreeMap::new();
        // When each part on was switched on: switched on again (Try Again), it starts afresh.
        let mut switched: BTreeMap<String, u64> = BTreeMap::new();
        // What each service's app declares it may reach, looked at with the builds; and what each was started with.
        let mut declared: BTreeMap<String, Option<::services::sandbox::Declared>> = BTreeMap::new();
        let mut started_with: BTreeMap<String, Option<::services::sandbox::Declared>> = BTreeMap::new();
        // What onet was last told, and when.
        let mut told: Option<(Value, Instant)> = None;
        let say = |name: &str, now: Now| said.lock().unwrap().say(name, now);
        loop {
            let wanted = onet.json::<Wanted>("GET", "/api/services", None).map_err(|err| tracing::debug!("which services are on: {err:#}")).ok();
            if let Some(wanted) = wanted.as_ref().filter(|w| w.erase) {
                stop(std::mem::take(&mut running).into_iter().map(|(name, (child, _))| (name, child)).collect());
                // Held till it exits, so neither is started again meanwhile.
                let mut kept = kept.lock().unwrap();
                Kept::stop(&mut *kept);
                if let Err(err) = std::fs::remove_dir_all(&home) {
                    tracing::warn!("erasing {}: {err:#}", home.display());
                }
                tracing::warn!("erased {} as {} asked: stopping, to start again as nobody's", home.display(), wanted.device);
                std::process::exit(0);
            }
            if wanted.is_some() && looked.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
                match onet.json("GET", "/api/services/builds", None) {
                    Ok(builds) => {
                        released = builds;
                        declared.clear();
                        looked = Some(Instant::now());
                    }
                    Err(err) => tracing::debug!("which builds of services run here: {err:#}"),
                }
            }
            if let Some(wanted) = &wanted {
                let device = &wanted.device;
                let names = offered(&released.iter().filter(|(_, b)| **b != Build::None).map(|(n, _)| n.clone()).collect(), &wanted.on);
                said.lock().unwrap().offered = names.clone();
                for name in &names {
                    let on = wanted.on.contains_key(name);
                    // Switched on again while it runs fine (its record naming its scope), it goes on running.
                    let troubled = !running.contains_key(name) || said.lock().unwrap().now(name).problem.is_some();
                    if let Some(at) = wanted.on.get(name)
                        && switched.insert(name.clone(), *at).is_some_and(|was| was != *at)
                        && troubled
                    {
                        failed.lock().unwrap().remove(name);
                        stops.remove(name);
                        say(name, Now::default());
                        if let Some((child, _)) = running.remove(name) {
                            stop(vec![(name.to_string(), child)]);
                        }
                    }
                    let here = program(name);
                    // Only a build its app's latest deploy names runs: another deploy stops the one before, and
                    // one whose deploy names none no longer runs the build a deploy named before.
                    let (committed, blob) = taken(name);
                    let named = carried(name).is_some()
                        || match released.get(name) {
                            Some(Build::Deployed(hash)) => *hash == blob,
                            Some(Build::Released(_)) => true,
                            Some(Build::None) => false,
                            None => committed.is_some() || blob.is_empty(),
                        };
                    // Its program, fetched: none is here, or a newer release is out than the one here (or than
                    // one put here by hand, by when it was).
                    if on && carried(name).is_none() && let Some(r) = released.get(name).filter(|b| **b != Build::None) {
                        let had = std::fs::metadata(&here).ok().and_then(|m| m.modified().ok()).map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                        let newer = match (r, had, blob.is_empty()) {
                            (_, None, _) => true,
                            (Build::Deployed(hash), ..) => *hash != blob,
                            (Build::Released(r), Some(_), false) => r.sha256 != blob && committed.is_none_or(|c| r.committed >= c),
                            (Build::Released(r), Some(when), true) => r.committed > when,
                            (Build::None, ..) => false,
                        };
                        let deployed = matches!(r, Build::Deployed(_));
                        let lately = failed.lock().unwrap().get(name).is_some_and(|t| t.elapsed() < Duration::from_secs(60));
                        if newer && !lately && fetching.lock().unwrap().insert(name.clone()) {
                            // Set up for the first time: shown while it happens. Updated: nothing shows.
                            if had.is_none() {
                                say(name, Now { doing: Some("Setting up"), problem: None });
                            }
                            let (onet, said, address, r, name) = (onet.clone(), said.clone(), wanted.network.address.clone(), r.clone(), name.clone());
                            let (fetching, failed) = (fetching.clone(), failed.clone());
                            std::thread::spawn(move || {
                                match put(&onet, &address, &name, &r) {
                                    Ok(()) => {
                                        failed.lock().unwrap().remove(&name);
                                    }
                                    Err(err) => {
                                        tracing::warn!("fetching {name}'s program: {err:#}");
                                        failed.lock().unwrap().insert(name.clone(), Instant::now());
                                        // A build its deploy names that fails its check: it runs none, and says why.
                                        if had.is_none() || deployed {
                                            said.lock().unwrap().say(&name, Now { doing: None, problem: Some(format!("Couldn’t set it up: {err:#}")) });
                                        }
                                    }
                                }
                                fetching.lock().unwrap().remove(&name);
                            });
                        }
                    }
                    if !named && let Some((child, _)) = running.remove(name) {
                        stop(vec![(name.to_string(), child)]);
                    }
                    // Its app declares what it may reach, looked at with the builds (and while it's unknown) for one switched
                    // on: a declaration that changed starts it again in its sandbox.
                    if on && declared.get(name).is_none_or(|d| d.is_none()) {
                        let d = onet.json("POST", "/api/services/declared", Some(&json!({ "name": name }))).map_err(|err| tracing::debug!("{name}'s declaration: {err:#}")).ok().flatten();
                        declared.insert(name.clone(), d);
                    }
                    let declares = declared.get(name).cloned().flatten();
                    let resandbox = said.lock().unwrap().held.get(name).is_some_and(|h| h.sandbox.is_some() && ::services::sandbox::unsandboxed(declares.as_ref(), device).is_none());
                    if running.contains_key(name) && (resandbox || started_with.get(name).is_some_and(|d| *d != declares)) && let Some((child, _)) = running.remove(name) {
                        tracing::info!("{name}'s app declares something else of it, or this machine can sandbox it now: oned starts it again");
                        stop(vec![(name.to_string(), child)]);
                    }
                    // Unknown, it waits a while for its app to run here; a build its deploy names never runs without one.
                    let deployed = matches!(released.get(name), Some(Build::Deployed(_)));
                    let patient = began.elapsed() < Duration::from_secs(120) && !wanted.off.contains(::scope::scopes::APPS_PART);
                    let declaring = declares.is_none() && (deployed || patient);
                    if on && declares.is_none() && deployed {
                        say(name, Now { doing: None, problem: Some(UNDECLARED.into()) });
                    }
                    if on && released.get(name) == Some(&Build::None) {
                        say(name, Now { doing: None, problem: Some("Its app’s latest deploy has no build of it for this device’s platform.".into()) });
                    }
                    if on && named && !declaring && !running.contains_key(name) && here.exists() && !fetching.lock().unwrap().contains(name) {
                        started_with.insert(name.clone(), declares.clone());
                        up.remove(name);
                        match start(&wanted.network, name, declares.as_ref(), &runtime) {
                            Ok(child) => {
                                tracing::info!("oned started {name}");
                                let held = Held { sandbox: ::services::sandbox::unsandboxed(declares.as_ref(), device), user: declares.as_ref().is_some_and(|d| d.user) };
                                said.lock().unwrap().held.insert(name.clone(), held);
                                running.insert(name.clone(), (child, Instant::now()));
                                // It runs now, so that its app said nothing yet no longer holds.
                                let problem = said.lock().unwrap().now(name).problem.filter(|p| p != UNDECLARED);
                                say(name, Now { doing: problem.is_none().then_some("Starting"), problem });
                            }
                            Err(err) => say(name, Now { doing: None, problem: Some(format!("Couldn’t start it: {err:#}")) }),
                        }
                    } else if !on && let Some((child, _)) = running.remove(name) {
                        stop(vec![(name.to_string(), child)]);
                    }
                    if !on {
                        said.lock().unwrap().held.remove(name);
                        stops.remove(name);
                        switched.remove(name);
                        failed.lock().unwrap().remove(name);
                        say(name, Now::default());
                    }
                }
            }
            running.retain(|name, (child, _)| match child.try_wait() {
                Ok(None) => true,
                done => {
                    tracing::warn!("{name} stopped ({done:?}): oned starts it again");
                    let lately = stops.entry(name.clone()).or_default();
                    lately.retain(|t| t.elapsed() < Duration::from_secs(300));
                    lately.push(Instant::now());
                    if lately.len() >= 3 {
                        say(name, Now { doing: None, problem: Some("It keeps stopping. Its device’s log says why.".into()) });
                    }
                    false
                }
            });
            // Started, it's on once it's registered with its device's app runtime, asked until it is, even once it said what's wrong; one
            // that doesn't in a minute says so. One that stayed up a minute no longer keeps stopping. One that can't
            // start in its sandbox says why, and one running without one says so.
            for (name, (_, started)) in &running {
                let now = said.lock().unwrap().now(name);
                if let Ok(why) = std::fs::read_to_string(::services::folder(name).join(part::PROBLEM)) {
                    say(name, Now { doing: None, problem: Some(why) });
                    continue;
                }
                if !up.contains(name) && onet.json::<Value>("POST", "/api/services/up", Some(&json!({ "name": name }))).is_ok_and(|a| a["up"] == true) {
                    up.insert(name.clone());
                }
                if up.contains(name) {
                    if started.elapsed() > Duration::from_secs(60) {
                        stops.remove(name);
                    }
                    if now != Now::default() && stops.get(name).is_none_or(|s| s.len() < 3) {
                        say(name, Now::default());
                    }
                } else if now.doing == Some("Starting") && started.elapsed() > Duration::from_secs(60) {
                    say(name, Now { doing: None, problem: Some("It started, but doesn’t answer. Its device’s log says why.".into()) });
                }
            }
            for name in running.keys() {
                let Ok(asked) = std::fs::read_to_string(::services::folder(name).join("ask")) else { continue };
                if listed.get(name).is_some_and(|(a, at)| *a == asked && at.elapsed() < Duration::from_secs(60)) {
                    continue;
                }
                listed.insert(name.clone(), (asked.clone(), Instant::now()));
                let listing = serde_json::from_str::<Value>(&asked).map_err(anyhow::Error::from).and_then(|ask| onet.ask("POST", "/api/services/list", Some(&json!({ "name": name, "key": ask["key"], "seal": ask["seal"] }))));
                if let Err(err) = listing {
                    tracing::warn!("listing {name} as this device's: {err:#}");
                }
            }
            // What they're doing, told when it changes, and now and then.
            if let Some(wanted) = &wanted {
                let parts = json!({ "parts": said.lock().unwrap().parts(wanted) });
                if told.as_ref().is_none_or(|(was, at)| *was != parts || at.elapsed() >= TELL_EVERY) {
                    match onet.ask("POST", "/api/services", Some(&parts)) {
                        Ok(_) => told = Some((parts, Instant::now())),
                        Err(err) => tracing::debug!("telling onet what the services are doing: {err:#}"),
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Starts a part, given the network it's in, what its app declares it may reach, and its device's app runtime's address.
fn start(network: &::services::Network, name: &str, declared: Option<&::services::sandbox::Declared>, runtime: &str) -> anyhow::Result<std::process::Child> {
    let folder = ::services::folder(name);
    network.give(&folder)?;
    let _ = std::fs::remove_file(folder.join(part::PROBLEM));
    match declared {
        Some(d) => std::fs::write(folder.join(part::DECLARED), serde_json::to_vec(d)?)?,
        None => drop(std::fs::remove_file(folder.join(part::DECLARED))),
    }
    // Its stdin is a pipe only this keeps the end of: the part ends when this does, however (`part::run`).
    Ok(command(&[name, &format!("--runtime={runtime}")])?.stdin(std::process::Stdio::piped()).spawn()?)
}

/// Stops parts, all at once: each asked, then made to, if it hasn't in ten seconds.
fn stop(mut parts: Vec<(String, std::process::Child)>) {
    for (_, child) in &mut parts {
        platform::os::ask_to_stop(child);
    }
    for _ in 0..100 {
        parts.retain_mut(|(name, child)| {
            let running = matches!(child.try_wait(), Ok(None));
            if !running {
                tracing::info!("oned stopped {name}");
            }
            running
        });
        if parts.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for (name, mut child) in parts {
        let _ = child.kill();
        let _ = child.wait();
        tracing::info!("oned stopped {name}");
    }
}

/// Puts the usual tool folders on PATH: launchd starts the Mac's app runtime with almost none, and the shell, agents and
/// services it starts need Homebrew's and the user's own tools.
fn usual_path() {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let usual = [
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        "/opt/homebrew/bin".into(),
        "/opt/homebrew/sbin".into(),
        "/usr/local/bin".into(),
        "/usr/local/sbin".into(),
        "/usr/bin".into(),
        "/bin".into(),
        "/usr/sbin".into(),
        "/sbin".into(),
    ];
    // Before what it has, as a login shell puts them (Homebrew's git before the system's).
    let missing: Vec<PathBuf> = usual.into_iter().filter(|d| d.is_dir() && !path.contains(d)).collect();
    path.splice(0..0, missing);
    if let Ok(joined) = std::env::join_paths(path) {
        // Before the runtime starts any thread.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

/// What oned says it is, for `--help` and for an argument it doesn't take.
fn usage() -> String {
    format!("{} runs this device, or with a service's name, that service, as its oned starts it; --version says which build", ::custom_build::programs::oned())
}

/// oned: with no part named, this device, for good (`serve`); with one, that service (`part::run`). It takes only the arguments its own starts give it: any other (`--version` typed to see
/// the build) never starts it, which would replace the one running.
pub fn run(args: &[String]) -> Result<()> {
    let flags: Vec<&str> = args.iter().filter(|a| a.starts_with('-')).map(String::as_str).collect();
    let words: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let flag = |prefix: &str| flags.iter().find_map(|f| f.strip_prefix(prefix));
    match (words.as_slice(), flags.as_slice()) {
        ([], ["--version"]) => println!("{}", ::oned_update::release::Version::this().json()),
        ([], ["--help"]) => println!("{}", usage()),
        #[cfg(target_os = "linux")]
        ([], [_]) if let Some(dir) = flag("--sandboxed=") => return ::services::sandbox::launch(std::path::Path::new(dir)),
        // A device's app runtime from before oned started onet starts the build it installed to take over from it, so: this
        // exits at once, and that app runtime, which can't hand over, exits for its supervisor to start this build afresh.
        ([], [_]) if flag("--handoff=").is_some() => {}
        ([], []) => {
            usual_path();
            return serve();
        }
        ([name], [_]) if let Some(runtime) = flag("--runtime=") => return part::run(name, runtime),
        _ => bail!("{}", usage()),
    }
    Ok(())
}

/// What a service whose app hasn’t said what it may reach shows, until it does.
const UNDECLARED: &str = "Its app doesn’t say what it may reach, so it doesn’t run.";

/// One of the programs oned keeps running (onet and the app runtime): started again whenever it stops.
struct Kept {
    /// What it is, as oned's log says it, and how `<cli> logs` reads its log.
    what: &'static str,
    log: &'static str,
    /// What starts it, given its arguments.
    program: fn() -> Result<std::process::Command>,
    args: Vec<String>,
    /// The socket it serves, which oned keeps and hands it each time (`os::spawn_listening`), so its address stays its own.
    listening: Option<std::net::TcpListener>,
    child: Option<std::process::Child>,
    stops: Vec<Instant>,
}

impl Kept {
    fn new(what: &'static str, log: &'static str, program: fn() -> Result<std::process::Command>, args: Vec<String>, listening: Option<std::net::TcpListener>) -> Kept {
        Kept { what, log, program, args, listening, child: None, stops: Vec::new() }
    }

    /// Started if it isn't running; one that stopped is said in oned's log, and once it keeps stopping, said so.
    fn keep(&mut self) {
        if let Some(child) = &mut self.child
            && !matches!(child.try_wait(), Ok(None))
        {
            tracing::warn!("{} stopped ({:?}): oned starts it again", self.what, child.try_wait());
            self.child = None;
            self.stops.retain(|t| t.elapsed() < Duration::from_secs(300));
            self.stops.push(Instant::now());
            if self.stops.len() == 3 {
                tracing::error!("{} keeps stopping: its log says why ({CLI} logs <this device>{})", self.what, self.log);
            }
        }
        if self.child.is_none() {
            // On Windows, closing its stdin is how it's asked to stop (`os::ask_to_stop`).
            let stdin = if cfg!(windows) { std::process::Stdio::piped() } else { std::process::Stdio::null() };
            let spawn = |c: &mut std::process::Command| match &self.listening {
                Some(listener) => platform::os::spawn_listening(c, listener),
                None => c.spawn(),
            };
            match (self.program)().and_then(|mut c| Ok(spawn(c.args(&self.args).stdin(stdin))?)) {
                Ok(child) => {
                    tracing::info!("oned started {}", self.what);
                    self.child = Some(child);
                }
                Err(err) => tracing::warn!("couldn't start {}: {err:#}", self.what),
            }
        }
    }

    /// Stops each of `kept` that runs, all at once.
    fn stop(kept: &mut [Kept]) {
        stop(kept.iter_mut().filter_map(|k| Some((k.what.to_string(), k.child.take()?))).collect());
    }
}

/// The app runtime's program (`<cli>-apps`, app-runtime/) of this build: the one built beside this program (`program`),
/// else the one the record at the network's address publishes with this build (`service/apps/<target>`), fetched.
fn runtime() -> Result<std::process::Command> {
    use ::scope::scopes::APPS_PART;
    let program = program(APPS_PART);
    let this = |commit: &str| ::oned_update::latest::same_build(commit, ::custom_build::config::BUILD);
    if !::oned_update::release::Version::of(&program).is_some_and(|v| this(&v.commit)) {
        let home = platform::home();
        let address = ::oned_update::update::address(&home).context("this device doesn't know its network's address yet, to fetch its app runtime from")?;
        // Asked at most once a minute: oned starts it again every few seconds, and the address may be out of reach.
        static ASKED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        if ASKED.lock().unwrap().is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
            bail!("no app runtime of this build ({}) is here yet", ::custom_build::config::BUILD);
        }
        *ASKED.lock().unwrap() = Some(Instant::now());
        let text = ::oned_update::latest::fetch(&address)?.with_context(|| format!("nothing has been published at {address} yet"))?;
        let record = ::oned_update::latest::Latest::read(platform::mode::here().release_key(), &text).with_context(|| format!("the record at {address}"))?;
        let name = ::oned_update::latest::Thing::service_name(APPS_PART, ::custom_build::config::TARGET);
        let thing = record.get(&name).with_context(|| format!("the record at {address} has no {name}"))?;
        anyhow::ensure!(this(&thing.commit), "the app runtime published at {address} is {}, not this build's ({}): {CLI} update takes the newest build, and its app runtime with it", thing.commit, ::custom_build::config::BUILD);
        put(&Onet::new(home), &address, APPS_PART, &Build::Released(thing.clone()))?;
    }
    Ok(std::process::Command::new(program))
}

/// oned on a computer: keeps the computer's onet running, which holds its key and answers its programs, and its app
/// runtime, which onet asks on a loopback socket oned keeps for as long as it runs.
#[tokio::main(flavor = "current_thread")]
async fn serve() -> Result<()> {
    #[cfg(windows)]
    {
        platform::os::children_end_with_me()?;
        platform::os::remove_replaced(&platform::this_program()?);
    }
    ::device_logs::keep("oned");
    tracing_subscriber::fmt()
        .with_writer(::device_logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    let dir = platform::home();
    std::fs::create_dir_all(&dir).with_context(|| format!("make {}", dir.display()))?;
    // Held until it ends: a second one would keep the ports and the running build an update meant to replace.
    let _alone = platform::os::alone(&dir, "oned.lock")?.with_context(|| format!("another {} runs for {} already: stop it first, as only one keeps this device's onet and app runtime", ::custom_build::programs::oned(), dir.display()))?;
    std::fs::write(dir.join(::oned_update::update::PID), std::process::id().to_string())?;
    ::oned_update::update::note_running(&dir)?;
    with_bucket_address(&dir)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let at = listener.local_addr()?.to_string();
    let kept = std::sync::Arc::new(std::sync::Mutex::new([
        Kept::new("onet", " --service onet", || Ok(platform::os::named(&::custom_build::programs::onet())?), vec![format!("--listen={}", ::api::local::addr()), format!("--runtime={at}")], None),
        Kept::new("the app runtime", "", runtime, Vec::new(), Some(listener)),
    ]));
    keep_parts(kept.clone(), at.clone());
    let stop = platform::os::stop_asked()?;
    tokio::spawn({
        let kept = kept.clone();
        async move {
            stop.await;
            Kept::stop(&mut *kept.lock().unwrap());
            std::process::exit(0)
        }
    });
    tracing::info!("{NAME} oned {} keeping onet and its app runtime ({at}) running, from {}", ::custom_build::config::BUILD, dir.display());
    platform::os::notify("READY=1");
    loop {
        let waiting = {
            let [onet, runtime] = &mut *kept.lock().unwrap();
            onet.keep();
            // The runtime asks onet for what the device's key opens as it starts: once onet answers, which it's looked
            // for as often as a loopback is asked again (`api::loopback`).
            if runtime.child.is_some() || std::net::TcpStream::connect(::api::local::addr()).is_ok() {
                runtime.keep();
            }
            runtime.child.is_none()
        };
        tokio::time::sleep(if waiting { ::api::loopback::EVERY } else { Duration::from_secs(2) }).await;
    }
}

/// Writes the network's address beside its bucket's fields in this computer's network file, where it's S3-compatible
/// storage and they don't say it: what a computer's onet and app runtime reach the bucket through, before either has
/// read who's who, which names it. A computer that joined before its app runtime kept it there has it only in the file its
/// app runtime kept for `one update`.
fn with_bucket_address(home: &std::path::Path) -> Result<()> {
    let Some(mut network) = ::api::home::NetworkFile::read(home)? else { return Ok(()) };
    let Some(bucket) = network.bucket.as_mut().filter(|b| b.contains_key("endpoint") && !b.contains_key("address")) else { return Ok(()) };
    let Some(address) = ::oned_update::update::address(home) else { return Ok(()) };
    bucket.insert("address".into(), address);
    network.write(home)
}
