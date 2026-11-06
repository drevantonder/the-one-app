//! Device services on the command line: what each device runs, and switching them on and off.

use serde_json::json;

use crate::cli::{Command, ago};
use crate::config::{CLI, NAME};

pub fn commands() -> Command {
    Command {
        help: vec![
            ("services".into(), "what each of your devices can run, each on or off, and what it's doing".into()),
            ("services on|off <name> [--device <name>]".into(), "switch a service on or off on a device of yours (here by default), which its oned starts or stops, fetching its program as it's published (a Mac app carries its own); apps is its app service. An admin's --device cloudflare switches a service built for Cloudflare on the Cloudflare node (Models: Workers AI)".into()),
            ("services sandbox | sudo sh".into(), "let this machine run services in their sandbox, each as its own user (Linux; the installer does it)".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["services"] => {
                    let Some(network) = g.node.state()?.network else { return Ok(true) };
                    for s in &network.devices {
                        let (installed, off) = (&s.now.installed_services, &s.off_services);
                        let offline = if s.now.online { "" } else { " (offline)" };
                        if let Some(parts) = &s.now.parts {
                            println!("{}{offline}", s.name);
                            for p in parts {
                                let version = installed.iter().find(|d| d.name == p.name).and_then(|d| d.version.as_deref()).map(|v| format!(" {}", &v[..v.len().min(12)])).unwrap_or_default();
                                let doing = p.doing.as_ref().map(|d| format!(", {}", d.to_lowercase())).unwrap_or_default();
                                let problem = p.problem.as_ref().map(|w| format!(": {w}")).unwrap_or_default();
                                let held = match (&p.sandbox, p.person) {
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
                        println!("{}{offline}", s.name);
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
                    // The Cloudflare node, whose services an admin switches (`--device cloudflare`).
                    if let Some(cloudflare) = &network.cloudflare_node {
                        println!("Cloudflare");
                        for p in &cloudflare.parts {
                            println!("  {}  {}", p.name, if p.on { "on" } else { "off" });
                        }
                    }
                }
                // What lets this machine's person start services in their sandbox, for root to run once: `| sudo sh`.
                ["services", "sandbox"] => print!("{}", crate::services::sandbox::setup_script()?),
                ["services", how @ ("on" | "off"), name] => {
                    let device = g.flag("device");
                    g.node.post("/v1/service", json!({ "name": name, "device": device, "on": *how == "on" }))?;
                    let there = device.map(|s| format!("on {s}")).unwrap_or_else(|| "here".into());
                    match (*how, *name == crate::database::scopes::APPS_PART) {
                        _ if device.is_some_and(|d| d.eq_ignore_ascii_case(crate::rank::CLOUDFLARE)) => println!("{name} is switched {how} on the Cloudflare node."),
                        ("off", true) => println!("Apps' calls no longer run {there}."),
                        (_, true) => println!("Apps' calls run {there} again."),
                        ("off", _) => println!("{name} is switched off {there}: its oned stops it."),
                        _ => println!("{name} is switched on {there}: its oned sets it up and starts it; {CLI} services says when it's running."),
                    }
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
