//! oned (`<cli>d`): a device's always-on program, this program run by that name; the `<cli>` command line runs, answers
//! and exits. Its key and endpoint are the device's, and everyone calls it; its parts are the app runtime (`oned apps`,
//! `apps::run`: workerd, which oned hands apps' calls to over a loopback, holding no key or endpoint of its own) and
//! each service: each a process of oned's own (`oned <name>`, `service::run`) with its own key, which oned lists as its
//! device's. Each is on while the device's latest `service` entry for it says so; with none, the app runtime is on, the
//! rest off; and each is started again when it stops, the rest left running.
//! Only the device's person writes that entry (its switch in Settings' Devices), and oned starts and stops the parts as
//! it changes. Each process part keeps its own key in its folder (`<home>/services/<name>`), so it answers only its own
//! person whoever else's calls the device runs, and one failing leaves the rest; nothing approves it by a code, since
//! its device lists it. Its program is its app's own (`apps/<name>/service/`), and oned runs only a build its app's
//! latest deploy names for this platform (`bundle::SERVICES`), which `<cli> deploy` builds where it's run, or, for one
//! whose deploy names none, as a built-in app's, the newest `<cli> update` built and published signed with the release
//! key (a `service-release` entry, which an older device ignores). oned fetches it, checks its hash or signature and puts
//! it at `~/.local/bin/<cli>-<name>` when it's switched on and again when another is named or a newer one's published;
//! a Mac app carries the built-in ones in Rust inside it, built with it. Its hello says what the device can run (`parts`) and each part's state (on, off, starting,
//! stopping, failed and why) and how it runs: in its sandbox as its app declares (`sandbox`, which oned tells each
//! part in its folder, `declared.json`), without one and why, or as the person; which is what Settings' Devices shows. Removing a device removes its parts, and can erase
//! what it keeps, which the device does itself, so only while it's reachable. On a Mac the app is oned; a phone runs
//! none, its app is the device.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::CLI;
use crate::node::{Build, Node};

/// What oned is called: its CLI's name and d.
pub fn name() -> String {
    format!("{CLI}d")
}

/// Whether this process was started as oned.
pub fn invoked() -> bool {
    std::env::args_os().next().and_then(|a| PathBuf::from(a).file_name().map(|n| n.to_string_lossy() == name())).unwrap_or(false)
}

/// This program, run as oned with `args`.
pub fn command(args: &[&str]) -> std::io::Result<std::process::Command> {
    use std::os::unix::process::CommandExt;
    // The path, not the program this process runs: an update may have replaced it.
    let exe = std::env::current_exe()?.to_string_lossy().trim_end_matches(" (deleted)").to_string();
    let mut command = std::process::Command::new(exe);
    command.arg0(name()).args(args);
    Ok(command)
}

/// The program that offers the service `name`: the one this program carries (`carried`), else the one oned fetched.
pub fn program(name: &str) -> PathBuf {
    carried(name).unwrap_or_else(|| fetched(name))
}

/// Where oned puts a service's program it fetched, and `<cli> services try` puts yours: in `~/.local/bin`, but for a
/// throwaway network's node (it has a home and an address of its own, `one throwaway`), in its home's `bin`, so what it
/// fetches never replaces the real network's program, which shares the user's home.
pub fn fetched(name: &str) -> PathBuf {
    let var = |v: &str| std::env::var_os(format!("{}_{v}", CLI.to_uppercase()));
    if let (Some(home), Some(_)) = (var("HOME"), var("ADDR")) {
        return PathBuf::from(home).join("bin").join(format!("{CLI}-{name}"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".local/bin").join(format!("{CLI}-{name}"))
}

/// A service's program built with this one and carried beside it, which oned runs as it is and fetches none of: the
/// Mac app's, inside it (apple/build.sh), and a checkout's (`one throwaway`), which Cargo builds beside it.
pub fn carried(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(format!("{CLI}-{name}"));
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

/// Every helper app's program the Mac app carries.
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
    // A helper runs unless switched off, as the Mac app's own services did.
    for program in helpers() {
        names.extend(program.file_name().and_then(|f| f.to_string_lossy().strip_prefix(&prefix).map(str::to_string)));
    }
    for entry in dirs.iter().flat_map(std::fs::read_dir).flatten().flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        let Some(name) = file.strip_prefix(&prefix).filter(|n| crate::members::valid_app_name(n)) else { continue };
        if on.contains_key(name) || crate::service::folder(name).is_dir() {
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

/// How a part runs, as its app declares: without its sandbox, saying why, or as the person.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Held {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub person: bool,
}

/// How each part running runs.
static HELD: std::sync::Mutex<BTreeMap<String, Held>> = std::sync::Mutex::new(BTreeMap::new());

/// How each part running runs, as its hello says.
pub fn held() -> BTreeMap<String, Held> {
    HELD.lock().unwrap().clone()
}

/// Says how a part runs (none: it doesn't): changed, its device tells the others at once.
fn hold(node: &Node, runtime: &tokio::runtime::Handle, name: &str, held: Option<Held>) {
    let changed = {
        let mut all = HELD.lock().unwrap();
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

/// Each part's `Now`, and the services oned could run as its processes.
static NOW: std::sync::Mutex<(BTreeMap<String, Now>, BTreeSet<String>)> = std::sync::Mutex::new((BTreeMap::new(), BTreeSet::new()));

/// The services this device could run as processes of oned's, and what each is doing now, as its hello says.
pub fn parts() -> (BTreeSet<String>, BTreeMap<String, Now>) {
    let now = NOW.lock().unwrap();
    (now.1.clone(), now.0.clone())
}

/// Says what a part is doing now: changed, its device tells the others at once.
fn say(node: &Node, runtime: &tokio::runtime::Handle, name: &str, now: Now) {
    let changed = {
        let mut all = NOW.lock().unwrap();
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
    let kept = std::fs::read_to_string(crate::service::folder(name).join("release")).unwrap_or_default();
    let (committed, blob) = kept.trim().split_once(' ').unwrap_or(("0", ""));
    (committed.parse().ok(), blob.to_string())
}

/// Puts the program of `build`, once its signature is the release key's or its hash the one its deploy names, where
/// oned runs it, in one step so it never starts half of it.
fn put(node: &Node, runtime: &tokio::runtime::Handle, name: &str, build: &Build) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (bytes, kept) = match build {
        Build::Released(r) => (runtime.block_on(node.service_program(r))?, format!("{} {}", r.committed, r.blob)),
        Build::Deployed(hash) => (runtime.block_on(node.service_build(hash))?, format!("deploy {hash}")),
        Build::None => anyhow::bail!("its deploy names no build for this platform"),
    };
    let path = fetched(name);
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let new = path.with_extension("new");
    std::fs::write(&new, bytes)?;
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&new, &path)?;
    let folder = crate::service::folder(name);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("release"), &kept)?;
    tracing::info!("oned put {name} {kept} in place");
    Ok(())
}

/// Where this device keeps what it keeps, once it was asked to erase it (`erase`).
static ERASING: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Erases what this device keeps, in its home `dir`, once the request that asked has been answered: its parts stopped,
/// its apps' runtime stopped, its home deleted, then oned stops, to start again as nobody's.
pub fn erase(dir: &std::path::Path) {
    *ERASING.lock().unwrap() = Some(dir.to_path_buf());
}

/// Keeps this device's parts that run as processes of their own running as its entries say: when one is switched on,
/// its program fetched from the newest release for this platform if it isn't here (and again when a newer one is
/// published), then started (and again if it stops); stopped when it's switched off; and listed as this device's once
/// each asks. What each is doing meanwhile, and what went wrong, its hello says (`parts`). In a thread of its own for
/// good, so what it starts is its child for as long as oned runs.
pub fn keep_parts(node: Node, runtime: tokio::runtime::Handle) {
    // Push was a part once; now every node answers it through the push Worker, so its folder (the phones it registered,
    // which register again) would only list a part that runs nothing.
    let _ = std::fs::remove_dir_all(crate::service::folder(crate::push::NAME));
    std::thread::spawn(move || {
        // Each part running, and when it started.
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
        let mut declared: BTreeMap<String, Option<crate::sandbox::Declared>> = BTreeMap::new();
        let mut started_with: BTreeMap<String, Option<crate::sandbox::Declared>> = BTreeMap::new();
        let device = node.device_name();
        // The app runtime, while it runs, and when it stopped by itself lately.
        let mut apps: Option<std::process::Child> = None;
        let mut apps_stops: Vec<Instant> = Vec::new();
        loop {
            if let Some(dir) = ERASING.lock().unwrap().clone() {
                std::thread::sleep(Duration::from_secs(1));
                for (name, (child, _)) in std::mem::take(&mut running) {
                    stop(&name, child);
                }
                if let Some(child) = apps.take() {
                    stop(crate::members::APPS_PART, child);
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
                    NOW.lock().unwrap().1 = names.clone();
                    for name in &names {
                        // A Mac service's helper the Mac app carries runs unless its person switched it off there, as
                        // the Mac app's own services did; any other, only where it's switched on.
                        let wanted = on.contains_key(name) || (helper(name).is_some() && node.removed_here(name).is_none());
                        // Switched on again while it runs fine (its record naming its scope), it goes on running.
                        let troubled = !running.contains_key(name) || NOW.lock().unwrap().0.get(name).is_some_and(|n| n.problem.is_some());
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
                                (Build::Released(r), Some(_), false) => r.blob != blob && committed.is_none_or(|c| r.committed >= c),
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
                        let resandbox = HELD.lock().unwrap().get(name).is_some_and(|h| h.sandbox.is_some() && crate::sandbox::unsandboxed(said.as_ref(), &device).is_none());
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
                                    let held = Held { sandbox: crate::sandbox::unsandboxed(said.as_ref(), &device), person: said.as_ref().is_some_and(|d| d.person) };
                                    hold(&node, &runtime, name, Some(held));
                                    running.insert(name.clone(), (child, Instant::now()));
                                    let problem = NOW.lock().unwrap().0.get(name).and_then(|n| n.problem.clone());
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
                let now = NOW.lock().unwrap().0.get(name).cloned().unwrap_or_default();
                if let Ok(why) = std::fs::read_to_string(crate::service::folder(name).join(crate::service::PROBLEM)) {
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
                let Ok(asked) = std::fs::read_to_string(crate::service::folder(name).join("ask")) else { continue };
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
    let name = crate::members::APPS_PART;
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
        (false, true) => match node.app_runtime().program().and_then(|mut c| c.stdin(std::process::Stdio::null()).spawn()) {
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
fn start(network: &crate::service::Network, name: &str, declared: Option<&crate::sandbox::Declared>) -> anyhow::Result<std::process::Child> {
    let folder = crate::service::folder(name);
    network.give(&folder)?;
    let _ = std::fs::remove_file(folder.join(crate::service::PROBLEM));
    match declared {
        Some(d) => std::fs::write(folder.join(crate::service::DECLARED), serde_json::to_vec(d)?)?,
        None => drop(std::fs::remove_file(folder.join(crate::service::DECLARED))),
    }
    Ok(command(&[name])?.stdin(std::process::Stdio::null()).spawn()?)
}

/// Stops a part: asked, then made to, if it hasn't in ten seconds.
fn stop(name: &str, mut child: std::process::Child) {
    // SAFETY: a signal to a child of this process, which it hasn't reaped.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
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

/// Lists a part as it asked: its endpoint, and whether it serves everyone.
async fn list(node: &Node, name: &str, asked: &str) -> anyhow::Result<()> {
    let asked: Value = serde_json::from_str(asked)?;
    let hexed = |k: &str| crate::keys::unhex(asked[k].as_str().unwrap_or(""));
    let endpoint = crate::keys::Endpoint { key: hexed("key")?, seal: hexed("seal")? };
    node.list_part(&endpoint, name, asked["shared"] == true).await
}
