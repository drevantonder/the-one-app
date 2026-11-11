//! oned (`<cli>d`): a computer's always-on program, this program run by that name; the `<cli>` command line runs, answers
//! and exits. It holds no key. It keeps two programs of its own running, starting each again when it stops: the
//! computer's onet (`<cli>t`), which holds the device's key and answers its programs, and its app runtime (`oned apps
//! --listen=`, `runtime`), an endpoint of its own key that onet asks on a loopback address oned gives both. And it takes
//! the device's updates from the signed record by itself (`update::pull`), then starts again on the new build.
//!
//! The app runtime runs the device's parts that are processes of their own (`keep_parts`): each service, a process of
//! oned's own (`oned <name>`, `service::run`) with its own key, which it lists as its device's. Each is on while the
//! device's latest `service` step for it says so; with none, off; and each is started again when it stops, the rest
//! left running. Only the device's user writes that step (its switch in Settings' Devices), and the parts start and stop
//! as it changes. Each part keeps its own key in its folder (`<home>/services/<name>`), so it answers only its own
//! user whoever else's calls the device runs, and one failing leaves the rest; nothing approves it by a code, since
//! its device lists it. Its program is its app's own (`apps/<name>/service/`), and only a build its app's
//! latest deploy names for this platform runs (`bundle::SERVICES`), which `<cli> deploy` builds where it's run, or, for one
//! whose deploy names none, as a built-in app's, the newest `<cli> update` published (`crate::latest`). It's fetched, its
//! hash checked and put at `~/.local/bin/<cli>-<name>` when it's switched on and again when another is named or a newer
//! one's published; a Mac app carries the built-in ones in Rust inside it, built with it. The runtime's hello says what
//! the device can run (`parts`) and each part's state (on, off, starting, stopping, failed and why) and how it runs: in
//! its sandbox as its app declares (`sandbox`, told to each part in its folder, `declared.json`), without one and why,
//! or as the user; which is what Settings' Devices shows. Removing a device removes its parts, and can erase what it
//! keeps, which the device does itself, so only while it's reachable. On a Mac the app carries oned; a phone runs none,
//! its app is the device.

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
/// each asks. What each is doing meanwhile, and what went wrong, its hello says (`parts`). Run by the app runtime, in
/// a thread of its own for good, so what it starts is its child for as long as the runtime runs.
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
        loop {
            if node.parts().erasing.load(std::sync::atomic::Ordering::SeqCst) {
                let dir = node.dir().to_path_buf();
                let oned = std::fs::read_to_string(dir.join(crate::update::PID)).ok().and_then(|p| p.trim().parse::<i32>().ok());
                std::thread::sleep(Duration::from_secs(1));
                for (name, (child, _)) in std::mem::take(&mut running) {
                    stop(&name, child);
                }
                crate::app::runtime::stop_all();
                if let Err(err) = std::fs::remove_dir_all(&dir) {
                    tracing::warn!("erasing {}: {err:#}", dir.display());
                }
                tracing::warn!("erased {}: stopping", dir.display());
                // Its oned too, with onet and all it started (`serve`), to start again as nobody's.
                if let Some(pid) = oned {
                    crate::os::ask_pid_to_stop(pid);
                }
                std::process::exit(0);
            }
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
                        let patient = crate::watchdog::since_start().is_some_and(|s| s < Duration::from_secs(120)) && node.runs_apps();
                        let declaring = said.is_none() && (deployed || patient);
                        if wanted && said.is_none() && deployed {
                            say(&node, &runtime, name, Now { doing: None, problem: Some(UNDECLARED.into()) });
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
                                    // It runs now, so that its app said nothing yet no longer holds.
                                    let problem = node.parts().now.lock().unwrap().0.get(name).and_then(|n| n.problem.clone()).filter(|p| p != UNDECLARED);
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

/// What oned says it is, for `--help` and for an argument it doesn't take.
fn usage() -> String {
    format!("{} runs this device, or with a service's name, that service, as its oned starts it; --version says which build", name())
}

/// The name onet runs by on a computer (`<cli>t`), which oned starts it as.
pub fn onet() -> String {
    format!("{CLI}t")
}

/// oned: with no part named, this device, for good (`serve`); with one, that part (`service::run`, or `apps
/// --listen=`, its app runtime). It takes only the arguments its own starts give it: any other (`--version` typed to see
/// the build) never starts it, which would replace the one running.
pub fn run(args: &[String]) -> Result<()> {
    let flags: Vec<&str> = args.iter().filter(|a| a.starts_with('-')).map(String::as_str).collect();
    let words: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let flag = |prefix: &str| flags.iter().find_map(|f| f.strip_prefix(prefix));
    match (words.as_slice(), flags.as_slice()) {
        ([], ["--version"]) => println!("{}", crate::release::Version::this().json()),
        ([], ["--help"]) => println!("{}", usage()),
        #[cfg(target_os = "linux")]
        ([], [_]) if let Some(dir) = flag("--sandboxed=") => return crate::services::sandbox::launch(std::path::Path::new(dir)),
        // A device's node from before oned started onet starts the build it installed to take over from it, so: this
        // exits at once, and that node, which can't hand over, exits for its supervisor to start this build afresh.
        ([], [_]) if flag("--handoff=").is_some() => {}
        ([], []) => {
            usual_path();
            return serve();
        }
        ([crate::database::scopes::APPS_PART], [_]) if let Some(listen) = flag("--listen=") => return runtime(listen),
        ([name], []) if crate::database::scopes::valid_app_name(name) => return crate::services::endpoint::run(name),
        _ => bail!("{}", usage()),
    }
    Ok(())
}

/// `oned apps --listen=<address>`: this device's app runtime, an endpoint of its own key (`Node::runtime`), answering on
/// `address` only its device's own programs, with the token in its folder, and asking its device's key, held by the
/// computer's onet at the API's address, for what that opens and signs. It runs the device's parts that are processes
/// of their own (`keep_parts`), and its own watchdog.
#[tokio::main]
async fn runtime(listen: &str) -> Result<()> {
    crate::logs::keep(crate::logs::NODE);
    tracing_subscriber::fmt()
        .with_writer(crate::logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    crate::watchdog::started();
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
    let token = api::local_token(&dir).with_context(|| format!("{} has no token for this device's own programs: its onet makes it as it starts", dir.display()))?;
    let node = Node::runtime(&dir, crate::node::Holder { at: api::addr(), token }).await?;
    let _ = ready_node.send(node.clone());
    node.node_started(crate::insights::Ended::read(&dir));
    keep_parts(node.clone(), tokio::runtime::Handle::current());
    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("listen on {listen}"))?;
    tracing::info!("{NAME} app runtime {} answering {listen} from {}", crate::keys::short(&node.id()), dir.display());
    tokio::spawn(api::serve_guide(node.clone()));
    crate::watchdog::watch();
    let served = api::serve(listener, api::guarded(api::routes(), node)).await;
    crate::app::runtime::stop_all();
    Ok(served?)
}

/// What a computer's oned is checking the record at its network's address every so often for (`update::pull`).
const PULL_EVERY: Duration = Duration::from_secs(30);

/// What a node stopping waits at most for its last insights to be written: they may be lost, and stopping may not wait.
const LAST_SAY: Duration = Duration::from_secs(3);

/// What a service whose app hasn’t said what it may reach shows, until it does.
const UNDECLARED: &str = "Its app doesn’t say what it may reach, so it doesn’t run.";

/// One of the programs oned keeps running (onet and the app runtime): started again whenever it stops.
struct Kept {
    /// The name it runs by, and what it is, as oned's log says it.
    name: String,
    what: &'static str,
    args: Vec<String>,
    child: Option<std::process::Child>,
    stops: Vec<Instant>,
}

impl Kept {
    fn new(name: String, what: &'static str, args: Vec<String>) -> Kept {
        Kept { name, what, args, child: None, stops: Vec::new() }
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
                tracing::error!("{} keeps stopping: its log says why ({CLI} logs <this device>{})", self.what, self.log());
            }
        }
        if self.child.is_none() {
            // On Windows, closing its stdin is how it's asked to stop (`os::ask_to_stop`).
            let stdin = if cfg!(windows) { std::process::Stdio::piped() } else { std::process::Stdio::null() };
            match crate::os::named(&self.name).and_then(|mut c| c.args(&self.args).stdin(stdin).spawn()) {
                Ok(child) => {
                    tracing::info!("oned started {}", self.what);
                    self.child = Some(child);
                }
                Err(err) => tracing::warn!("couldn't start {}: {err}", self.what),
            }
        }
    }

    /// How its log is read: onet's is its own, and the app runtime's the node's (`logs::NODE`).
    fn log(&self) -> &str {
        if self.name == onet() { " --service onet" } else { "" }
    }

    fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            stop(self.what, child);
        }
    }
}

/// oned on a computer: keeps the computer's onet running, which holds its key and answers its programs, and its app
/// runtime, which onet asks on a loopback address chosen here; and now and then takes the newest build the record at
/// the network's address names (`update::pull`), needing nothing else of the network, then starts again on it.
#[tokio::main(flavor = "current_thread")]
async fn serve() -> Result<()> {
    #[cfg(windows)]
    {
        crate::os::children_end_with_me()?;
        crate::os::remove_replaced(&crate::this_program()?);
    }
    crate::logs::keep("oned");
    tracing_subscriber::fmt()
        .with_writer(crate::logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    let dir = crate::home();
    std::fs::create_dir_all(&dir).with_context(|| format!("make {}", dir.display()))?;
    std::fs::write(dir.join(crate::update::PID), std::process::id().to_string())?;
    crate::update::note_running(&dir)?;
    with_bucket_address(&dir)?;
    let at = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
    let kept = std::sync::Arc::new(std::sync::Mutex::new([
        Kept::new(onet(), "onet", vec![format!("--listen={}", api::addr()), format!("--runtime={at}")]),
        Kept::new(name(), "the app runtime", vec![crate::database::scopes::APPS_PART.into(), format!("--listen={at}")]),
    ]));
    let stop = stopped()?;
    tokio::spawn({
        let kept = kept.clone();
        async move {
            stop.await;
            kept.lock().unwrap().iter_mut().for_each(Kept::stop);
            std::process::exit(0)
        }
    });
    tracing::info!("{NAME} oned {} keeping onet and its app runtime ({at}) running, from {}", crate::config::BUILD, dir.display());
    crate::watchdog::notify("READY=1");
    let mut pulled: Option<Instant> = None;
    loop {
        {
            let [onet, runtime] = &mut *kept.lock().unwrap();
            onet.keep();
            // The runtime asks onet for what the device's key opens as it starts: once onet answers.
            if runtime.child.is_some() || std::net::TcpStream::connect(api::addr()).is_ok() {
                runtime.keep();
            }
        }
        if pulled.is_none_or(|p| p.elapsed() >= PULL_EVERY) {
            pulled = Some(Instant::now());
            let home = dir.clone();
            if let Ok(Some(commit)) = tokio::task::spawn_blocking(move || crate::update::pull(&home)).await {
                tracing::info!("put {commit} in place: starting again on it");
                kept.lock().unwrap().iter_mut().for_each(Kept::stop);
                return again();
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Starts oned again, on the program now at its path: in this process, so what started it still has it. Windows can't,
/// so there it exits, for its task to start it again within the minute.
fn again() -> Result<()> {
    #[cfg(unix)]
    {
        let err = std::os::unix::process::CommandExt::exec(&mut command(&[])?);
        Err(err).context("start oned again")
    }
    #[cfg(not(unix))]
    std::process::exit(0)
}

/// Writes the network's address beside its bucket's fields in this computer's network file, where it's S3-compatible
/// storage and they don't say it: what a computer's onet and app runtime reach the bucket through, before either has
/// read who's who, which names it. A computer that joined before its node kept it there has it only in the file its
/// node kept for `one update`.
fn with_bucket_address(home: &std::path::Path) -> Result<()> {
    let Some(mut network) = crate::node::NetworkFile::read(home)? else { return Ok(()) };
    let Some(bucket) = network.bucket.as_mut().filter(|b| b.contains_key("endpoint") && !b.contains_key("address")) else { return Ok(()) };
    let Some(address) = crate::update::address(home) else { return Ok(()) };
    bucket.insert("address".into(), address);
    network.write(home)
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
