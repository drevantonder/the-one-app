//! iPhones and iPads in Xcode's Simulator on a Mac, each a simulator of its own (simctl), with no window unless the
//! person opens Simulator. Simulator takes no clicks or typing from outside it, so agents get pictures, URLs, apps and
//! commands, and the person uses it in Simulator on the Mac. A copy is a clone of a stopped lasting device, or with the
//! lasting one running, a new simulator of the same model.

use anyhow::{Result, bail};
use serde_json::Value;

use super::Devices;
use super::kinds::{Action, Kind, Vm, run};

pub struct Iphone;

fn simctl(args: &[&str]) -> Result<String> {
    let mut all = vec!["simctl"];
    all.extend(args);
    run("xcrun", &all)
}

/// A simulator: its state, its device type, and the runtime it's of.
fn simulator_of(udid: &str) -> Option<(Value, String)> {
    let all: Value = serde_json::from_str(&simctl(&["list", "--json", "devices"]).ok()?).ok()?;
    for (runtime, sims) in all["devices"].as_object()? {
        if let Some(s) = sims.as_array()?.iter().find(|s| s["udid"] == udid) {
            return Some((s.clone(), runtime.clone()));
        }
    }
    None
}

impl Kind for Iphone {
    fn name(&self) -> &'static str {
        "iphone"
    }

    fn title(&self) -> &'static str {
        "iPhone"
    }

    fn limits(&self) -> &'static str {
        "no clicks, typing or keys: Simulator takes none from agents. Pictures, open_url, install_app (an .app built for the simulator) and run_command (a program in the simulator) work; the person uses it in Simulator on the Mac"
    }

    fn input(&self) -> bool {
        false
    }

    fn port(&self) -> (u16, u16) {
        (0, 0)
    }

    fn can(&self) -> String {
        if !cfg!(target_os = "macos") {
            return "iPhones run on Macs with Xcode".into();
        }
        if simctl(&["help"]).is_err() {
            return "iPhones need Xcode's simctl here".into();
        }
        String::new()
    }

    fn adopt(&self, _: &Devices, v: &mut Vm) -> Result<()> {
        if simulator_of(&v.id).is_none() {
            bail!("give id (its simulator's UDID)");
        }
        Ok(())
    }

    fn make(&self, _: &Devices, v: &mut Vm, from: Option<&Vm>) -> Result<()> {
        let name = format!("Agent Devices {}", v.name);
        v.gb = 2;
        if let Some(from) = from {
            let Some((src, runtime)) = simulator_of(&from.id) else { bail!("{}'s simulator is gone", from.name) };
            let out = if src["state"] == "Shutdown" {
                simctl(&["clone", &from.id, &name])?
            } else {
                simctl(&["create", &name, src["deviceTypeIdentifier"].as_str().unwrap_or_default(), &runtime])?
            };
            v.id = out.trim().to_string();
            return Ok(());
        }
        // The newest iPhone Pro with the newest iOS.
        let got: Value = serde_json::from_str(&simctl(&["list", "--json", "runtimes", "devicetypes"])?)?;
        let runtime = got["runtimes"].as_array().into_iter().flatten().filter(|r| r["platform"] == "iOS" && r["isAvailable"] == true).filter_map(|r| r["identifier"].as_str()).next_back();
        let kind = got["devicetypes"].as_array().into_iter().flatten().rev().find(|d| d["name"].as_str().is_some_and(|n| n.starts_with("iPhone") && n.ends_with(" Pro"))).and_then(|d| d["identifier"].as_str());
        let (Some(runtime), Some(kind)) = (runtime, kind) else { bail!("Xcode here has no iOS runtime or iPhone to make one with") };
        v.id = simctl(&["create", &name, kind, runtime])?.trim().to_string();
        Ok(())
    }

    fn running(&self, _: &Devices, v: &Vm) -> bool {
        simulator_of(&v.id).is_some_and(|(s, _)| s["state"] == "Booted")
    }

    fn start(&self, _: &Devices, v: &Vm) -> Result<()> {
        if let Err(err) = simctl(&["boot", &v.id])
            && !format!("{err:#}").contains("current state: Booted")
        {
            return Err(err);
        }
        simctl(&["bootstatus", &v.id, "-b"]).map(|_| ())
    }

    fn stop(&self, _: &Devices, v: &Vm) -> Result<()> {
        match simctl(&["shutdown", &v.id]) {
            Err(err) if !format!("{err:#}").contains("current state: Shutdown") => Err(err),
            _ => Ok(()),
        }
    }

    /// Shuts it down: Simulator can't pause one.
    fn pause(&self, ds: &Devices, v: &Vm) -> Result<()> {
        self.stop(ds, v)
    }

    fn resume(&self, ds: &Devices, v: &Vm) -> Result<()> {
        self.start(ds, v)
    }

    fn remove(&self, ds: &Devices, v: &Vm) -> Result<()> {
        let _ = self.stop(ds, v);
        simctl(&["delete", &v.id]).map(|_| ())
    }

    fn screen(&self, _: &Devices, v: &Vm) -> Result<image::DynamicImage> {
        let file = std::env::temp_dir().join(format!("agent-devices-screen-{}.png", super::random_hex(6)));
        let shot = simctl(&["io", &v.id, "screenshot", "--type=png", &file.to_string_lossy()]).and_then(|_| super::picture::decode_file(&file));
        let _ = std::fs::remove_file(&file);
        shot
    }

    fn act(&self, _: &Devices, _: &Vm, _: &Action) -> Result<()> {
        bail!("Simulator takes no clicks, typing or keys from agents: open_url, install_app and run_command work, and the person uses it in Simulator on the Mac")
    }

    fn open(&self, _: &Devices, v: &Vm, url: &str) -> Result<()> {
        simctl(&["openurl", &v.id, url]).map(|_| ())
    }

    fn install(&self, _: &Devices, v: &Vm, app: &str) -> Result<String> {
        if !app.trim_end_matches('/').ends_with(".app") {
            bail!("an iPhone simulator installs an .app built for the simulator, given as its folder on the Mac");
        }
        simctl(&["install", &v.id, app])?;
        Ok(format!("Installed {}.", std::path::Path::new(app.trim_end_matches('/')).file_name().unwrap_or_default().to_string_lossy()))
    }

    fn reach(&self, _: &Devices, v: &Vm, port: u16) -> Result<String> {
        Ok(format!("{} shares this Mac's network, so it reaches 127.0.0.1:{port} already.", v.name))
    }

    fn run(&self, _: &Devices, v: &Vm, command: &str) -> Result<String> {
        // The simulator has no shell: the command's first word is a program in it, the rest its arguments.
        let args: Vec<&str> = command.split_whitespace().collect();
        if args.is_empty() {
            bail!("run what? a program in the simulator, like launchctl list");
        }
        let mut all = vec!["spawn", v.id.as_str()];
        all.extend(args);
        simctl(&all)
    }
}
