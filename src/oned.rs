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
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg0(name()).args(args);
    Ok(command)
}

/// The program in another language that offers the service `name`, where `services/install.sh` puts it.
pub fn program(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".local/bin").join(format!("{CLI}-{name}"))
}

/// The services this device could run as processes of oned's: this program's own, and those whose programs are here.
fn offered() -> BTreeSet<String> {
    let mut names: BTreeSet<String> = crate::services::hosted().iter().map(|h| h.name.to_string()).collect();
    let dir = program("x").parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let prefix = format!("{CLI}-");
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if let Some(name) = file.strip_prefix(&prefix).filter(|n| crate::members::valid_app_name(n)) {
            names.insert(name.to_string());
        }
    }
    names
}

/// Keeps this device's parts that run as processes of their own running as its entries say: started when one is
/// switched on (and again if it stops), stopped when it's switched off, and listed as this device's once each asks.
/// In a thread of its own for good, so what it starts is its child for as long as oned runs.
pub fn keep_parts(node: Node, runtime: tokio::runtime::Handle) {
    std::thread::spawn(move || {
        let mut running: BTreeMap<String, std::process::Child> = BTreeMap::new();
        // What each part last asked, and when it was listed for it: asked again later, it's listed again.
        let mut listed: BTreeMap<String, (String, Instant)> = BTreeMap::new();
        loop {
            match node.parts_on() {
                Ok((install, on)) => {
                    for name in offered() {
                        let wanted = on.contains(&name);
                        if wanted && !running.contains_key(&name) {
                            match start(&install, &name) {
                                Ok(child) => {
                                    tracing::info!("oned started {name}");
                                    running.insert(name, child);
                                }
                                Err(err) => tracing::warn!("starting {name}: {err:#}"),
                            }
                        } else if !wanted && let Some(child) = running.remove(&name) {
                            stop(&name, child);
                        }
                    }
                }
                Err(err) => tracing::debug!("which parts are on: {err:#}"),
            }
            running.retain(|name, child| match child.try_wait() {
                Ok(None) => true,
                done => {
                    tracing::warn!("{name} stopped ({done:?}): oned starts it again");
                    false
                }
            });
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
