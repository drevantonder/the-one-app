//! Device services on the command line: what each device runs, and switching them on and off.

use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;

use ::api::Client;
use ::api::cli::{Command, Failed, ago};
use ::custom_build::config::{CLI, NAME};

pub fn commands() -> Command {
    Command {
        help: vec![
            ("services".into(), "what each of your devices can run, each on or off, and what it's doing".into()),
            ("services on|off <name> [--device <name>]".into(), "switch a service on or off on a device of yours (here by default), which starts or stops it, fetching its program as it's published; apps is the app runtime".into()),
            ("services sandbox | sudo sh".into(), "let this machine run services in their sandbox, each as its own user (Linux; the installer does it)".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["services"] => {
                    let Some(network) = g.node.state()?.network else { return Ok(true) };
                    for s in &network.devices {
                        let (installed, off) = (&s.now.installed_services, &s.off_services);
                        let away = Some(::api::cli::presence(s.now.online, s.now.seen)).filter(|p| !s.now.online && !p.is_empty()).map(|p| format!(" ({p})")).unwrap_or_default();
                        if let Some(parts) = &s.now.parts {
                            println!("{}{away}", s.name);
                            for p in parts {
                                let version = installed.iter().find(|d| d.name == p.name).and_then(|d| d.version.as_deref()).map(|v| format!(" {}", &v[..v.len().min(12)])).unwrap_or_default();
                                let doing = p.doing.as_ref().map(|d| format!(", {}", d.to_lowercase())).unwrap_or_default();
                                let problem = p.problem.as_ref().map(|w| format!(": {w}")).unwrap_or_default();
                                let held = match (&p.sandbox, p.user) {
                                    (Some(why), _) => format!(", without its sandbox: {why}"),
                                    (None, true) => ", as you".into(),
                                    _ => String::new(),
                                };
                                println!("  {}{version}  {}{doing}{held}{problem}", p.name, if p.on { "on" } else { "off" });
                            }
                            continue;
                        }
                        if installed.is_empty() && off.is_empty() {
                            continue;
                        }
                        println!("{}{away}", s.name);
                        for d in installed {
                            let by = match (&d.installed_by, d.built_in) {
                                (_, true) => format!("built into {NAME}"),
                                (Some(by), _) => format!("installed by {by}"),
                                _ => String::new(),
                            };
                            let version = d.version.as_deref().map(|v| format!(" {}", &v[..v.len().min(12)])).unwrap_or_default();
                            println!("  {}{version}  {} ({by})", d.name, d.title);
                        }
                        for d in off {
                            println!("  {}  switched off {}", d.name, ago(d.time));
                        }
                    }
                }
                // What lets this machine's user start services in their sandbox, for root to run once: `| sudo sh`.
                ["services", "sandbox"] => print!("{}", crate::sandbox::setup_script()?),
                ["services", how @ ("on" | "off"), name] => {
                    let names = known(g.node)?;
                    if !names.contains(*name) {
                        return Err(Failed::Typed(format!("no service called {name}: {}", names.into_iter().collect::<Vec<_>>().join(", "))).into());
                    }
                    let device = g.flag("device");
                    g.node.post("/api/service", json!({ "name": name, "device": device, "on": *how == "on" }))?;
                    let there = device.map(|s| format!("on {s}")).unwrap_or_else(|| "here".into());
                    match (*how, *name == scope::scopes::APPS_PART) {
                        _ if device.is_some_and(|d| d.eq_ignore_ascii_case(scope::rank::CLOUDFLARE)) => println!("{name} is switched {how} on the Cloudflare node."),
                        ("off", true) => println!("Apps' calls no longer run {there}."),
                        (_, true) => println!("Apps' calls run {there} again."),
                        ("off", _) => println!("{name} is switched off {there}."),
                        _ => println!("{name} is switched on {there}: {CLI} services says when it's running."),
                    }
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}

/// Every service there is: those an app uses (an app's service may not be on a device yet, and is switched on there
/// to put it on), and the parts devices list.
fn known(node: &Client) -> Result<BTreeSet<String>> {
    let mut names: BTreeSet<String> = node.apps()?.iter().flat_map(|a| a["uses"]["services"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>()).collect();
    names.insert(scope::scopes::APPS_PART.into());
    if let Some(network) = node.state()?.network {
        for s in &network.devices {
            names.extend(s.now.parts.iter().flatten().map(|p| p.name.clone()));
            names.extend(s.now.installed_services.iter().map(|d| d.name.clone()));
            names.extend(s.off_services.iter().map(|d| d.name.clone()));
        }
    }
    Ok(names)
}
