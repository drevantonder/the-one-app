//! oned (`<cli>d`): a device's always-on program, this program run by that name; the `<cli>` command line runs, answers
//! and exits. Its key and endpoint are the device's, and everyone calls it; its parts are the app runtime (`oned apps`,
//! `apps::run`: workerd, which oned hands apps' calls to over a loopback, holding no key or endpoint of its own) and
//! each service: each a process of oned's own (`oned <name>`, `service::run`) with its own key, which oned lists as its
//! device's. Each is on while the device's latest `service` step for it says so; with none, the app runtime is on, the
//! rest off; and each is started again when it stops, the rest left running.
//! Only the device's user writes that step (its switch in Settings' Devices), and oned starts and stops the parts as
//! it changes. Each process part keeps its own key in its folder (`<home>/services/<name>`), so it answers only its own
//! user whoever else's calls the device runs, and one failing leaves the rest; nothing approves it by a code, since
//! its device lists it. Its program is its app's own (`apps/<name>/service/`), and oned runs only a build its app's
//! latest deploy names for this platform (`bundle::SERVICES`), which `<cli> deploy` builds where it's run, or, for one
//! whose deploy names none, as a built-in app's, the newest `<cli> update` published (`crate::latest`). oned fetches it, checks its hash and puts
//! it at `~/.local/bin/<cli>-<name>` when it's switched on and again when another is named or a newer one's published;
//! a Mac app carries the built-in ones in Rust inside it, built with it. Its hello says what the device can run (`parts`) and each part's state (on, off, starting,
//! stopping, failed and why) and how it runs: in its sandbox as its app declares (`sandbox`, which oned tells each
//! part in its folder, `declared.json`), without one and why, or as the user; which is what Settings' Devices shows. Removing a device removes its parts, and can erase
//! what it keeps, which the device does itself, so only while it's reachable. On a Mac the app is oned; a phone runs
//! none, its app is the device.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::config::{CLI, NAME};
use crate::node::{Build, Node};
use crate::api;

pub fn name() -> String {
    format!("{CLI}d")
}

pub fn invoked() -> bool {
    crate::os::invoked_as().is_some_and(|me| me == name())
}

pub fn command(args: &[&str]) -> std::io::Result<std::process::Command> {
    let mut command = crate::os::named(&name())?;
    command.args(args);
    Ok(command)
}

/// The program that offers the service `name`: the one this program carries (`carried`), else the one oned fetched.
pub fn program(name: &str) -> PathBuf {
    carried(name).unwrap_or_else(|| fetched(name))
}

/// Where oned puts a service's program it fetched (`Mode::bin`).
pub fn fetched(name: &str) -> PathBuf {
    crate::mode::here().bin().join(format!("{CLI}-{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A service's program built with this one and carried beside it, which oned runs as it is and fetches none of: the
/// Mac app's, inside it (client/native/mac/build.sh), and a checkout's (`one-dev throwaway`), which Cargo builds beside it.
pub fn carried(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(format!("{CLI}-{name}{}", std::env::consts::EXE_SUFFIX));
    (path.exists() && path != fetched(name)).then_some(path).or_else(|| helper(name))
}

/// A Mac service's helper app the Mac app carries (`Contents/Helpers/<Title> — <App>.app`, service/mac-helpers.sh):
/// its program, which holds its own macOS permission.
pub fn helper(name: &str) -> Option<PathBuf> {
    let program = format!("{CLI}-{name}");
    helpers().into_iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy() == program))
}

/// Where a helper app runs from: a copy of the one the Mac app carries (`helper`), out of the Mac app, in this device's
/// home (`helpers/`), copied again when the Mac app's changed. macOS records the permissions of code inside an app as
/// that app's, so only a copy outside it asks for and keeps its own, under its own name; it keeps them across builds,
/// since they're the helper's by its id and team, not where it is. Its program.
#[cfg(target_os = "macos")]
pub fn placed(carried: &std::path::Path) -> anyhow::Result<PathBuf> {
    use anyhow::Context as _;
    let bundle = carried.ancestors().nth(3).context("a helper's program is in its app's Contents/MacOS")?;
    let copy = crate::home().join("helpers").join(bundle.file_name().unwrap_or_default());
    let program = copy.join(carried.strip_prefix(bundle)?);
    let stamp = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| Ok((m.len(), m.modified()?))).ok();
    if stamp(&program).is_none() || stamp(&program) != stamp(carried) {
        let _ = std::fs::remove_dir_all(&copy);
        std::fs::create_dir_all(copy.parent().unwrap_or(&copy))?;
        let done = std::process::Command::new("/usr/bin/ditto").arg(bundle).arg(&copy).status()?;
        anyhow::ensure!(done.success(), "copying {} failed", bundle.display());
    }
    Ok(program)
}

fn helpers() -> Vec<PathBuf> {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("Helpers"))) else { return Vec::new() };
    let apps = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path().join("Contents/MacOS"));
    apps.flat_map(std::fs::read_dir).flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&format!("{CLI}-")))).collect()
}

/// The services this device could run as processes of oned's: those with a release for its platform (`released`),
/// which it fetches when one is switched on, and those whose programs are here (carried or fetched) that ran here
/// before or are switched on (`on`), not every `<cli>-` program there.
fn offered(released: &BTreeSet<String>, on: &BTreeMap<String, u64>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let dirs = [fetched("x"), std::env::current_exe().unwrap_or_default()].map(|p| p.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    let prefix = format!("{CLI}-");
    for program in helpers() {
        names.extend(program.file_name().and_then(|f| f.to_string_lossy().strip_prefix(&prefix).map(str::to_string)));
    }
    for entry in dirs.iter().flat_map(std::fs::read_dir).flatten().flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        let Some(name) = file.strip_suffix(std::env::consts::EXE_SUFFIX).and_then(|f| f.strip_prefix(&prefix)).filter(|n| crate::database::scopes::valid_app_name(n)) else { continue };
        if on.contains_key(name) || crate::services::endpoint::folder(name).is_dir() {
            names.insert(name.to_string());
        }
    }
    names.extend(released.iter().cloned());
    names
}

/// What a part that runs as a process of oned's is doing now, while it isn't simply on or off: being set up (its
/// program fetched), starting or stopping; or what went wrong.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Now {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doing: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// How a part runs, as its app declares: without its sandbox, saying why, or as the user.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Held {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user: bool,
}

/// What oned's thread for a node's parts (`keep_parts`) tells the node about them, for its hello: how each part running
/// runs, which parts could run as processes, and what each is doing; and whether it was asked to erase what the node keeps.
#[derive(Default)]
pub struct Parts {
    held: std::sync::Mutex<BTreeMap<String, Held>>,
    now: std::sync::Mutex<(BTreeMap<String, Now>, BTreeSet<String>)>,
    erasing: std::sync::atomic::AtomicBool,
}

impl Parts {
    /// How each part running runs, as its hello says.
    pub fn held(&self) -> BTreeMap<String, Held> {
        self.held.lock().unwrap().clone()
    }

    /// The services this device could run as processes of oned's, and what each is doing now, as its hello says.
    pub fn now(&self) -> (BTreeSet<String>, BTreeMap<String, Now>) {
        let now = self.now.lock().unwrap();
        (now.1.clone(), now.0.clone())
    }

    /// Asks `keep_parts` to erase what this device keeps, once the request that asked has been answered: its parts
    /// stopped, its apps' runtime stopped, its home deleted, then oned stops, to start again as nobody's.
    pub fn erase(&self) {
        self.erasing.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Says how a part runs (none: it doesn't): changed, its device tells the others at once.
fn hold(node: &Node, runtime: &tokio::runtime::Handle, name: &str, held: Option<Held>) {
    let changed = {
        let mut all = node.parts().held.lock().unwrap();
        match held {
            Some(h) => all.insert(name.to_string(), h.clone()) != Some(h),
            None => all.remove(name).is_some(),
        }
    };
    if changed {
        let _in = runtime.enter();
        node.parts_changed();
    }
}

/// Says what a part is doing now: changed, its device tells the others at once.
fn say(node: &Node, runtime: &tokio::runtime::Handle, name: &str, now: Now) {
    let changed = {
        let mut all = node.parts().now.lock().unwrap();
        let was = all.0.get(name).cloned().unwrap_or_default();
        if now == Now::default() {
            all.0.remove(name);
        } else {
            all.0.insert(name.to_string(), now.clone());
        }
        was != now
    };
    if changed {
        let _in = runtime.enter();
        node.parts_changed();
    }
}

/// What's put in a service's folder of the build its program is: its blob, and the release's commit time, or none for
/// the build a deploy named (`deploy <blob>`).
fn taken(name: &str) -> (Option<u64>, String) {
    let kept = std::fs::read_to_string(crate::services::endpoint::folder(name).join("release")).unwrap_or_default();
    let (committed, blob) = kept.trim().split_once(' ').unwrap_or(("0", ""));
    (committed.parse().ok(), blob.to_string())
}

/// Puts the program of `build`, once its signature is the release key's or its hash the one its deploy names, where
/// oned runs it, in one step so it never starts half of it.
fn put(node: &Node, runtime: &tokio::runtime::Handle, name: &str, build: &Build) -> anyhow::Result<()> {
    let (bytes, kept) = match build {
        Build::Released(r) => (runtime.block_on(node.thing_bytes(r))?, format!("{} {}", r.committed, r.sha256)),
        Build::Deployed(hash) => (runtime.block_on(node.service_build(hash))?, format!("deploy {hash}")),
        Build::None => anyhow::bail!("its deploy names no build for this platform"),
    };
    let path = fetched(name);
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let new = path.with_extension("new");
    std::fs::write(&new, bytes)?;
    crate::os::set_mode(&new, 0o755)?;
    std::fs::rename(&new, &path)?;
    let folder = crate::services::endpoint::folder(name);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("release"), &kept)?;
    tracing::info!("oned put {name} {kept} in place");
    Ok(())
}

/// Keeps this device's parts that run as processes of their own running as its steps say: when one is switched on,
/// its program fetched from the newest release for this platform if it isn't here (and again when a newer one is
/// published), then started (and again if it stops); stopped when it's switched off; and listed as this device's once
/// each asks. What each is doing meanwhile, and what went wrong, its hello says (`parts`). In a thread of its own for
/// good, so what it starts is its child for as long as oned runs.
pub fn keep_parts(node: Node, runtime: tokio::runtime::Handle) {
    std::thread::spawn(move || {
        let mut running: BTreeMap<String, (std::process::Child, Instant)> = BTreeMap::new();
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
        let mut declared: BTreeMap<String, Option<crate::services::sandbox::Declared>> = BTreeMap::new();
        let mut started_with: BTreeMap<String, Option<crate::services::sandbox::Declared>> = BTreeMap::new();
        let device = node.device_name();
        let mut apps: Option<std::process::Child> = None;
        let mut apps_stops: Vec<Instant> = Vec::new();
        loop {
            if node.parts().erasing.load(std::sync::atomic::Ordering::SeqCst) {
                let dir = node.dir().to_path_buf();
                std::thread::sleep(Duration::from_secs(1));
                for (name, (child, _)) in std::mem::take(&mut running) {
                    stop(&name, child);
                }
                if let Some(child) = apps.take() {
                    stop(crate::database::scopes::APPS_PART, child);
                }
                crate::app::runtime::stop_all();
                if let Err(err) = std::fs::remove_dir_all(&dir) {
                    tracing::warn!("erasing {}: {err:#}", dir.display());
                }
                tracing::warn!("erased {}: stopping", dir.display());
                std::process::exit(0);
            }
            // Handing over, the next oned keeps them: this one starting or listing any too would race it.
            if node.is_handing_over() {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            keep_runtime(&node, &runtime, &mut apps, &mut apps_stops);
            if looked.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
                released = runtime.block_on(node.service_builds());
                let carried: Vec<String> = helpers().iter().filter_map(|p| p.file_name()?.to_string_lossy().strip_prefix(&format!("{CLI}-")).map(str::to_string)).collect();
                runtime.block_on(node.name_services_scopes(&carried));
                declared.clear();
                looked = Some(Instant::now());
            }
            match node.parts_on() {
                Ok((network, on)) => {
                    let names = offered(&released.iter().filter(|(_, b)| **b != Build::None).map(|(n, _)| n.clone()).collect(), &on);
                    node.parts().now.lock().unwrap().1 = names.clone();
                    for name in &names {
                        // A Mac service's helper the Mac app carries runs unless its user switched it off there; any other,
                        // only where it's switched on.
                        let wanted = on.contains_key(name) || (helper(name).is_some() && node.removed_here(name).is_none());
                        // Switched on again while it runs fine (its record naming its scope), it goes on running.
                        let troubled = !running.contains_key(name) || node.parts().now.lock().unwrap().0.get(name).is_some_and(|n| n.problem.is_some());
                        if let Some(at) = on.get(name)
                            && switched.insert(name.clone(), *at).is_some_and(|was| was != *at)
                            && troubled
                        {
                            failed.lock().unwrap().remove(name);
                            stops.remove(name);
                            say(&node, &runtime, name, Now::default());
                            if let Some((child, _)) = running.remove(name) {
                                stop(name, child);
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
                        if wanted && carried(name).is_none() && let Some(r) = released.get(name).filter(|b| **b != Build::None) {
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
                                    say(&node, &runtime, name, Now { doing: Some("Setting up"), problem: None });
                                }
                                let (node, runtime, r, name) = (node.clone(), runtime.clone(), r.clone(), name.clone());
                                let (fetching, failed) = (fetching.clone(), failed.clone());
                                std::thread::spawn(move || {
                                    match put(&node, &runtime, &name, &r) {
                                        Ok(()) => {
                                            failed.lock().unwrap().remove(&name);
                                        }
                                        Err(err) => {
                                            tracing::warn!("fetching {name}'s program: {err:#}");
                                            failed.lock().unwrap().insert(name.clone(), Instant::now());
                                            // A build its deploy names that fails its check: it runs none, and says why.
                                            if had.is_none() || deployed {
                                                say(&node, &runtime, &name, Now { doing: None, problem: Some(format!("Couldn’t set it up: {err:#}")) });
                                            }
                                        }
                                    }
                                    fetching.lock().unwrap().remove(&name);
                                });
                            }
                        }
                        if !named && let Some((child, _)) = running.remove(name) {
                            stop(name, child);
                        }
                        // Its app declares what it may reach, looked at with the builds (and while it's unknown): a declaration
                        // that changed starts it again in its sandbox.
                        if declared.get(name).is_none_or(|d| d.is_none()) {
                            let d = runtime.block_on(node.service_declared(name)).map_err(|err| tracing::debug!("{name}'s declaration: {err:#}")).ok();
                            declared.insert(name.clone(), d);
                        }
                        let said = declared.get(name).cloned().flatten();
                        let resandbox = node.parts().held.lock().unwrap().get(name).is_some_and(|h| h.sandbox.is_some() && crate::services::sandbox::unsandboxed(said.as_ref(), &device).is_none());
                        if running.contains_key(name) && (resandbox || started_with.get(name).is_some_and(|d| *d != said)) && let Some((child, _)) = running.remove(name) {
                            tracing::info!("{name}'s app declares something else of it, or this machine can sandbox it now: oned starts it again");
                            stop(name, child);
                        }
                        // Unknown, it waits a while for its app to run here; a build its deploy names never runs without one.
                        let deployed = matches!(released.get(name), Some(Build::Deployed(_)));
                        let patient = crate::handoff::since_start().is_some_and(|s| s < Duration::from_secs(120)) && node.runs_apps();
                        let declaring = said.is_none() && (deployed || patient);
                        if wanted && said.is_none() && deployed {
                            say(&node, &runtime, name, Now { doing: None, problem: Some("Its app doesn’t say what it may reach, so it doesn’t run.".into()) });
                        }
                        if wanted && released.get(name) == Some(&Build::None) {
                            say(&node, &runtime, name, Now { doing: None, problem: Some("Its app’s latest deploy has no build of it for this device’s platform.".into()) });
                        }
                        if wanted && named && !declaring && !running.contains_key(name) && here.exists() && !fetching.lock().unwrap().contains(name) {
                            started_with.insert(name.clone(), said.clone());
                            match start(&network, name, said.as_ref()) {
                                Ok(child) => {
                                    tracing::info!("oned started {name}");
                                    let held = Held { sandbox: crate::services::sandbox::unsandboxed(said.as_ref(), &device), user: said.as_ref().is_some_and(|d| d.user) };
                                    hold(&node, &runtime, name, Some(held));
                                    running.insert(name.clone(), (child, Instant::now()));
                                    let problem = node.parts().now.lock().unwrap().0.get(name).and_then(|n| n.problem.clone());
                                    say(&node, &runtime, name, Now { doing: problem.is_none().then_some("Starting"), problem });
                                }
                                Err(err) => say(&node, &runtime, name, Now { doing: None, problem: Some(format!("Couldn’t start it: {err:#}")) }),
                            }
                        } else if !wanted && let Some((child, _)) = running.remove(name) {
                            say(&node, &runtime, name, Now { doing: Some("Stopping"), problem: None });
                            stop(name, child);
                            say(&node, &runtime, name, Now::default());
                        }
                        if !wanted {
                            hold(&node, &runtime, name, None);
                            stops.remove(name);
                            switched.remove(name);
                            failed.lock().unwrap().remove(name);
                            say(&node, &runtime, name, Now::default());
                        }
                    }
                }
                Err(err) => tracing::debug!("which parts are on: {err:#}"),
            }
            running.retain(|name, (child, _)| match child.try_wait() {
                Ok(None) => true,
                done => {
                    tracing::warn!("{name} stopped ({done:?}): oned starts it again");
                    let lately = stops.entry(name.clone()).or_default();
                    lately.retain(|t| t.elapsed() < Duration::from_secs(300));
                    lately.push(Instant::now());
                    if lately.len() >= 3 {
                        say(&node, &runtime, name, Now { doing: None, problem: Some("It keeps stopping. Its device’s log says why.".into()) });
                    }
                    false
                }
            });
            // Started, it's on once it answers its device, asked until it does, even once it said what's wrong; one
            // that doesn't in a minute says so. One that stayed up a minute no longer keeps stopping. One that can't
            // start in its sandbox says why, and one running without one says so.
            for (name, (_, started)) in &running {
                let now = node.parts().now.lock().unwrap().0.get(name).cloned().unwrap_or_default();
                if let Ok(why) = std::fs::read_to_string(crate::services::endpoint::folder(name).join(crate::services::endpoint::PROBLEM)) {
                    say(&node, &runtime, name, Now { doing: None, problem: Some(why) });
                    continue;
                }
                let fine = Now::default();
                if !node.part_up(name, *started) {
                    runtime.block_on(node.hello_part(name));
                }
                if node.part_up(name, *started) {
                    if started.elapsed() > Duration::from_secs(60) {
                        stops.remove(name);
                    }
                    if now != fine && stops.get(name).is_none_or(|s| s.len() < 3) {
                        say(&node, &runtime, name, fine);
                    }
                } else if now.doing == Some("Starting") && started.elapsed() > Duration::from_secs(60) {
                    say(&node, &runtime, name, Now { doing: None, problem: Some("It started, but doesn’t answer. Its device’s log says why.".into()) });
                }
            }
            for name in running.keys() {
                let Ok(asked) = std::fs::read_to_string(crate::services::endpoint::folder(name).join("ask")) else { continue };
                if listed.get(name).is_some_and(|(a, at)| *a == asked && at.elapsed() < Duration::from_secs(60)) {
                    continue;
                }
                listed.insert(name.clone(), (asked.clone(), Instant::now()));
                if let Err(err) = runtime.block_on(list(&node, name, &asked)) {
                    tracing::warn!("listing {name} as this device's: {err:#}");
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Keeps the app runtime running while its switch is on (with none, it is): started, and again whenever it stops,
/// saying so once it keeps stopping; stopped once it's switched off.
fn keep_runtime(node: &Node, runtime: &tokio::runtime::Handle, apps: &mut Option<std::process::Child>, stops: &mut Vec<Instant>) {
    let name = crate::database::scopes::APPS_PART;
    if let Some(child) = apps
        && !matches!(child.try_wait(), Ok(None))
    {
        tracing::warn!("the app runtime stopped ({:?}): oned starts it again", child.try_wait());
        *apps = None;
        stops.retain(|t| t.elapsed() < Duration::from_secs(300));
        stops.push(Instant::now());
        if stops.len() >= 3 {
            say(node, runtime, name, Now { doing: None, problem: Some("It keeps stopping. Its device’s log says why.".into()) });
        }
    }
    match (apps.is_some(), node.runs_apps()) {
        // On Windows, closing its stdin is how it's asked to stop (`os::ask_to_stop`).
        (false, true) => match node.app_runtime().program().and_then(|mut c| c.stdin(if cfg!(windows) { std::process::Stdio::piped() } else { std::process::Stdio::null() }).spawn()) {
            Ok(child) => {
                tracing::info!("oned started the app runtime");
                *apps = Some(child);
            }
            Err(err) => say(node, runtime, name, Now { doing: None, problem: Some(format!("Couldn’t start it: {err:#}")) }),
        },
        (true, false) => {
            say(node, runtime, name, Now { doing: Some("Stopping"), problem: None });
            stop(name, apps.take().expect("running"));
            stops.clear();
            say(node, runtime, name, Now::default());
        }
        (true, true) if stops.len() >= 3 && stops.last().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) => {
            stops.clear();
            say(node, runtime, name, Now::default());
        }
        _ => {}
    }
}

/// Starts a part, given the network it's in and what its app declares it may reach.
fn start(network: &crate::services::endpoint::Network, name: &str, declared: Option<&crate::services::sandbox::Declared>) -> anyhow::Result<std::process::Child> {
    let folder = crate::services::endpoint::folder(name);
    network.give(&folder)?;
    let _ = std::fs::remove_file(folder.join(crate::services::endpoint::PROBLEM));
    match declared {
        Some(d) => std::fs::write(folder.join(crate::services::endpoint::DECLARED), serde_json::to_vec(d)?)?,
        None => drop(std::fs::remove_file(folder.join(crate::services::endpoint::DECLARED))),
    }
    // Its stdin is a pipe only this keeps the end of: the part ends when this does, however (`endpoint::run`).
    Ok(command(&[name])?.stdin(std::process::Stdio::piped()).spawn()?)
}

/// Stops a part: asked, then made to, if it hasn't in ten seconds.
fn stop(name: &str, mut child: std::process::Child) {
    crate::os::ask_to_stop(&mut child);
    for _ in 0..100 {
        if !matches!(child.try_wait(), Ok(None)) {
            tracing::info!("oned stopped {name}");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    tracing::info!("oned stopped {name}");
}

/// Lists a part as it asked: its endpoint.
async fn list(node: &Node, name: &str, asked: &str) -> anyhow::Result<()> {
    let asked: Value = serde_json::from_str(asked)?;
    let hexed = |k: &str| crate::keys::unhex(asked[k].as_str().unwrap_or(""));
    let endpoint = crate::keys::Endpoint { key: hexed("key")?, seal: hexed("seal")? };
    node.list_part(&endpoint, name).await
}

/// Puts the usual tool folders on PATH: launchd starts the Mac's node with almost none, and the shell, agents and
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

/// oned: with no part named, this device, for good (`serve`); with one, that part (`service::run`).
pub fn run(args: &[String]) -> Result<()> {
    let named: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    match named.as_slice() {
        #[cfg(target_os = "linux")]
        [] if let Some(dir) = args.iter().find_map(|a| a.strip_prefix("--sandboxed=")) => crate::services::sandbox::launch(std::path::Path::new(dir)),
        [] => {
            usual_path();
            serve(args)
        }
        [name] if *name == crate::database::scopes::APPS_PART && let Some(dir) = args.iter().find_map(|a| a.strip_prefix("--runtime=")) => crate::app::runtime::run(std::path::Path::new(dir)),
        [name] => crate::services::endpoint::run(name),
        _ => bail!("{} runs this device, or with a service's name, that service, as its oned starts it", name()),
    }
}

/// How long a node stopping waits for its last insights to be written: they may be lost, and stopping may not wait.
const LAST_SAY: std::time::Duration = std::time::Duration::from_secs(3);

#[tokio::main]
async fn serve(args: &[String]) -> Result<()> {
    #[cfg(windows)]
    crate::os::children_end_with_me()?;
    // What it says is kept too, for `one logs`.
    crate::logs::keep(crate::logs::NODE);
    tracing_subscriber::fmt()
        .with_writer(crate::logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    // Asked to stop, even while it starts: its workerd first, then at once, rather than once every blocking task
    // ends, which a stuck request would hold up.
    let stop = stopped()?;
    let (ready_node, node_ready) = tokio::sync::oneshot::channel::<Node>();
    tokio::spawn(async move {
        stop.await;
        crate::insights::Ended::Signal.write(&crate::home());
        if let Ok(Ok(node)) = tokio::time::timeout(LAST_SAY, node_ready).await {
            let _ = tokio::time::timeout(LAST_SAY, node.flush_insights()).await;
        }
        crate::app::runtime::stop_all();
        std::process::exit(0)
    });
    let dir = crate::home();
    // A new network's first start makes its home.
    std::fs::create_dir_all(&dir).with_context(|| format!("make {}", dir.display()))?;
    // Taking over from the node before, an update's, with its socket: it answers until this one is warm.
    let handed = crate::handoff::handed(args)?;
    let listener = match &handed {
        Some(h) => tokio::net::TcpListener::from_std(h.listener.try_clone()?)?,
        None => listen(&dir).await?,
    };
    // Its app runtime is a program of its own, which oned starts as its parts.
    let node = Node::start(&dir, true).await?;
    let _ = ready_node.send(node.clone());
    node.node_started(crate::insights::Ended::read(&dir));
    // Its parts that run as processes of their own, as its steps switch them.
    crate::oned::keep_parts(node.clone(), tokio::runtime::Handle::current());
    // Git here reaches the network's repos at their address, as on every computer of its user's, and the harnesses
    // installed here reach this node.
    let mode = crate::mode::here();
    if matches!(mode, crate::mode::Mode::Own) {
        tokio::spawn(crate::harnesses::keep(node.clone(), dir.clone(), api::addr()));
    }
    if matches!(mode, crate::mode::Mode::Own) && let Ok(address) = node.address() {
        std::thread::spawn(move || {
            if let Err(err) = crate::git_wire::wire(&address) {
                tracing::warn!("{err:#}");
            }
        });
    }
    if handed.is_some() {
        node.warm().await;
    }
    std::fs::write(dir.join("pid"), std::process::id().to_string())?;
    tracing::info!("{NAME} node {} serving {} from {}", &crate::keys::hex(&node.id())[..8], api::addr(), dir.display());
    tokio::spawn(api::serve_guide(node.clone()));
    let mut routes = api::guarded(api::routes(), node.clone());
    if let crate::mode::Mode::Throwaway(network) = mode {
        routes = network.serve_beside(routes);
    }
    // Asked to restart (an update, or SIGHUP): the program now at this one's path takes over, and this one stops
    // taking connections, finishes what it's answering, and exits.
    let fd = raw_fd(&listener);
    let (drained, drain) = tokio::sync::oneshot::channel::<()>();
    let restarting = {
        let (node, args) = (node.clone(), args.to_vec());
        async move {
            loop {
                crate::handoff::wanted().await;
                tracing::info!("handing over to {}", crate::this_program().unwrap_or_default().display());
                crate::insights::Ended::Update.write(&crate::home());
                let _ = tokio::time::timeout(LAST_SAY, node.flush_insights()).await;
                node.handing_over(true).await;
                match crate::handoff::hand_over(fd, &args).await {
                    Ok(()) => return node.handed_over(),
                    Err(err) if crate::handoff::supervised() => {
                        // Its supervisor starts the new build.
                        tracing::warn!("couldn't hand over ({err:#}): exiting, to be started again");
                        std::process::exit(0);
                    }
                    Err(err) => {
                        tracing::warn!("couldn't hand over, so this build keeps running: {err:#}");
                        node.handing_over(false).await;
                    }
                }
            }
        }
    };
    #[cfg(unix)]
    tokio::spawn(async {
        let Ok(mut hup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) else { return };
        while hup.recv().await.is_some() {
            let _ = crate::handoff::restart();
        }
    });
    tokio::spawn(async move {
        restarting.await;
        let _ = drained.send(());
    });
    let http = api::serve_until(listener, routes, async {
        let _ = drain.await;
    });
    crate::handoff::able();
    match handed {
        Some(h) => h.ready(),
        None => crate::handoff::notify("READY=1"),
    }
    tracing::info!("{NAME} node {} answering", &crate::keys::hex(&node.id())[..8]);
    crate::handoff::watch();
    let served = http.await;
    crate::app::runtime::stop_all();
    if crate::handoff::stopped() {
        std::process::exit(0);
    }
    Ok(served?)
}

#[cfg(unix)]
fn raw_fd(listener: &tokio::net::TcpListener) -> i32 {
    std::os::fd::AsRawFd::as_raw_fd(listener)
}

#[cfg(not(unix))]
fn raw_fd(_listener: &tokio::net::TcpListener) -> i32 {
    -1
}

/// Asks a node this one didn't start to stop: on Windows, which can't ask a process it didn't start, ends it.
fn stray_stop(pid: i32) {
    #[cfg(unix)]
    // SAFETY: only a signal, to a process of this program's (`ours`).
    unsafe {
        libc::kill(pid, libc::SIGTERM)
    };
    #[cfg(windows)]
    crate::os::kill(pid as u32);
}

/// When the node is asked to stop: Ctrl-C, or SIGTERM from its supervisor, an update or a node replacing it, each
/// listened for from when it's called.
fn stopped() -> Result<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (mut term, mut interrupt) = (signal(SignalKind::terminate())?, signal(SignalKind::interrupt())?);
        Ok(Box::pin(async move {
            tokio::select! {
                _ = term.recv() => {}
                _ = interrupt.recv() => {}
            }
        }))
    }
    #[cfg(not(unix))]
    Ok(Box::pin(async {
        let _ = tokio::signal::ctrl_c().await;
    }))
}

/// The node's address, listened on. A node of this network already there (one left running outside launchd or systemd
/// by an earlier build, or started by hand) is stopped first: the one starting is the one to keep, so its supervisor's
/// node never waits on a stray, and an update's node replaces whatever ran before it.
async fn listen(dir: &std::path::Path) -> Result<tokio::net::TcpListener> {
    let bind = || tokio::net::TcpListener::bind(api::addr());
    match bind().await {
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {}
        bound => return bound.with_context(|| format!("listen on {}", api::addr())),
    }
    // Who listens there: the pid this home's node wrote.
    let mut pids: Vec<i32> = std::fs::read_to_string(dir.join("pid")).ok().and_then(|p| p.trim().parse().ok()).into_iter().collect();
    // Only a process of this program, never whatever else took the port.
    let me = std::env::current_exe().ok().and_then(|e| e.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    let ours = |pid: i32| crate::os::is_program(pid as u32, &me);
    pids.sort();
    pids.dedup();
    pids.retain(|&pid| pid as u32 != std::process::id() && ours(pid));
    anyhow::ensure!(!pids.is_empty(), "{} is taken, and not by {NAME}'s node", api::addr());
    for &pid in &pids {
        tracing::warn!("a node of this network was already running (pid {pid}): stopping it, to run this one");
        stray_stop(pid);
    }
    for tries in 0..100 {
        if let Ok(listener) = bind().await {
            return Ok(listener);
        }
        if tries == 50 {
            for &pid in &pids {
                crate::os::kill(pid as u32);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bind().await.with_context(|| format!("listen on {}: the node there didn't stop", api::addr()))
}
