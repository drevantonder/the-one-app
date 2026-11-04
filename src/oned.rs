//! oned (`<cli>d`): a device's always-on program, this program run by that name; the `<cli>` command line runs, answers
//! and exits. Its key is the device's, and its parts are the app service (the node: running calls) and each service:
//! those built into the node, answering at the device's endpoint, and the rest, each a process of oned's own
//! (`oned <name>`, `service::run`) with its own key, which oned lists as its device's. Each is on while the device's
//! latest `service` entry for it says so; with none, the app service and the built-in services are on, the rest off.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::CLI;
use crate::node::Node;

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

/// The program in another language that offers the service `name`, where `services/install.sh` puts it.
pub fn program(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".local/bin").join(format!("{CLI}-{name}"))
}

/// The services this device could run as processes of oned's: this program's own, those whose programs are here, and
/// those with a release for its platform (`released`), which it fetches when one is switched on.
fn offered(released: &BTreeSet<String>) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = crate::services::hosted().iter().map(|h| h.name.to_string()).collect();
    let dir = program("x").parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let prefix = format!("{CLI}-");
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if let Some(name) = file.strip_prefix(&prefix).filter(|n| crate::members::valid_app_name(n)) {
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

/// What's put in a service's folder of the release its program came from: its commit's time and its blob.
fn taken(name: &str) -> (u64, String) {
    let kept = std::fs::read_to_string(crate::service::folder(name).join("release")).unwrap_or_default();
    let (committed, blob) = kept.trim().split_once(' ').unwrap_or(("0", ""));
    (committed.parse().unwrap_or(0), blob.to_string())
}

/// Puts `r`'s program, signed with the release key, where oned runs it, in one step so it never starts half of it.
fn put(node: &Node, runtime: &tokio::runtime::Handle, name: &str, r: &crate::release::Release) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let bytes = runtime.block_on(node.service_program(r))?;
    let path = program(name);
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let new = path.with_extension("new");
    std::fs::write(&new, bytes)?;
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&new, &path)?;
    let folder = crate::service::folder(name);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("release"), format!("{} {}", r.committed, r.blob))?;
    tracing::info!("oned put {name} {} in place", r.commit);
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
    std::thread::spawn(move || {
        // Each part running, and when it started.
        let mut running: BTreeMap<String, (std::process::Child, Instant)> = BTreeMap::new();
        // What each part last asked, and when it was listed for it: asked again later, it's listed again.
        let mut listed: BTreeMap<String, (String, Instant)> = BTreeMap::new();
        // The newest release of each service's program for this platform, looked at now and then.
        let mut released: BTreeMap<String, crate::release::Release> = BTreeMap::new();
        let mut looked: Option<Instant> = None;
        // Programs being fetched, and when fetching each last failed, to try again a minute on.
        let fetching = std::sync::Arc::new(std::sync::Mutex::new(BTreeSet::<String>::new()));
        let failed = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::<String, Instant>::new()));
        // When each part stopped by itself lately: one that keeps stopping says so.
        let mut stops: BTreeMap<String, Vec<Instant>> = BTreeMap::new();
        // When each part on was switched on: switched on again (Try Again), it starts afresh.
        let mut switched: BTreeMap<String, u64> = BTreeMap::new();
        loop {
            if let Some(dir) = ERASING.lock().unwrap().clone() {
                std::thread::sleep(Duration::from_secs(1));
                for (name, (child, _)) in std::mem::take(&mut running) {
                    stop(&name, child);
                }
                crate::apps::stop_all();
                if let Err(err) = std::fs::remove_dir_all(&dir) {
                    tracing::warn!("erasing {}: {err:#}", dir.display());
                }
                tracing::warn!("erased {}: stopping", dir.display());
                std::process::exit(0);
            }
            if looked.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
                released = node.service_releases();
                looked = Some(Instant::now());
            }
            match node.parts_on() {
                Ok((install, on)) => {
                    let names = offered(&released.keys().cloned().collect());
                    NOW.lock().unwrap().1 = names.clone();
                    for name in &names {
                        let wanted = on.contains_key(name);
                        if let Some(at) = on.get(name)
                            && switched.insert(name.clone(), *at).is_some_and(|was| was != *at)
                        {
                            failed.lock().unwrap().remove(name);
                            stops.remove(name);
                            say(&node, &runtime, name, Now::default());
                            if let Some((child, _)) = running.remove(name) {
                                stop(name, child);
                            }
                        }
                        let hosted = crate::services::hosted().iter().any(|h| h.name == name.as_str());
                        let here = program(name);
                        // Its program, fetched: none is here, or a newer release is out than the one here (or than
                        // one put here by hand, by when it was).
                        if wanted && !hosted && let Some(r) = released.get(name) {
                            let (committed, blob) = taken(name);
                            let had = std::fs::metadata(&here).ok().and_then(|m| m.modified().ok()).map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                            let newer = match (had, blob.is_empty()) {
                                (None, _) => true,
                                (Some(_), false) => r.blob != blob && r.committed >= committed,
                                (Some(when), true) => r.committed > when,
                            };
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
                                            if had.is_none() {
                                                say(&node, &runtime, &name, Now { doing: None, problem: Some(format!("Couldn’t set it up: {err:#}")) });
                                            }
                                        }
                                    }
                                    fetching.lock().unwrap().remove(&name);
                                });
                            }
                        }
                        if wanted && !running.contains_key(name) && (hosted || here.exists()) && !fetching.lock().unwrap().contains(name) {
                            match start(&install, name) {
                                Ok(child) => {
                                    tracing::info!("oned started {name}");
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
            // Started, it's on once it answers its device; one that doesn't in a minute says so.
            for (name, (_, started)) in &running {
                let now = NOW.lock().unwrap().0.get(name).cloned().unwrap_or_default();
                if now.doing == Some("Starting") && !node.part_up(name, *started) {
                    runtime.block_on(node.hello_part(name));
                }
                if node.part_up(name, *started) {
                    if now != Now::default() && stops.get(name).is_none_or(|s| s.len() < 3) {
                        say(&node, &runtime, name, Now::default());
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

/// Starts a part, given the install it's in.
fn start(install: &crate::service::Install, name: &str) -> anyhow::Result<std::process::Child> {
    install.give(&crate::service::folder(name))?;
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
