//! What this machine keeps about each device's lifecycle, beside the runtimes' own state, in this service's folder:
//!
//! - running.json: whether each device should be running, of every kind, set only once a start or stop worked. A
//!   restart of this service, or of the machine, starts again those that should run and have room (revive).
//! - moved.json: devices moved to another machine. Moving one says where it's going before the other machine restores
//!   it, and that it went once it did; until it's deleted here, it's kept stopped and refuses to start, saying where it
//!   went.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, anyhow};
use one_service::refuse;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

use super::linux::Device;
use super::{Devices, time};

/// A device moved, or being moved, from here to another machine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Move {
    pub to: String,
    #[serde(with = "time")]
    pub at: jiff::Timestamp,
    /// The other machine restored it; until then it's being moved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
}

impl Devices {
    fn read_life<T: DeserializeOwned + Default>(&self, file: &str) -> T {
        match std::fs::read(self.dir.join(file)) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|err| {
                tracing::warn!("{file}: {err}");
                T::default()
            }),
            Err(_) => T::default(),
        }
    }

    fn write_life<T: Serialize>(&self, file: &str, v: &T) -> Result<()> {
        let part = self.dir.join(format!("{file}.part"));
        std::fs::write(&part, serde_json::to_vec_pretty(v)?)?;
        std::fs::rename(part, self.dir.join(file))?;
        Ok(())
    }

    fn wanted(&self) -> HashMap<String, bool> {
        let _life = self.life.lock().unwrap();
        self.read_life("running.json")
    }

    pub(super) fn want(&self, name: &str, running: bool) {
        let _life = self.life.lock().unwrap();
        let mut w: HashMap<String, bool> = self.read_life("running.json");
        if w.get(name) == Some(&running) {
            return;
        }
        w.insert(name.into(), running);
        if let Err(err) = self.write_life("running.json", &w) {
            tracing::warn!("keeping whether {name} should run: {err:#}");
        }
    }

    pub(super) fn unwant(&self, name: &str) {
        let _life = self.life.lock().unwrap();
        let mut w: HashMap<String, bool> = self.read_life("running.json");
        if w.remove(name).is_some()
            && let Err(err) = self.write_life("running.json", &w)
        {
            tracing::warn!("{err:#}");
        }
    }

    pub(super) fn moves(&self) -> HashMap<String, Move> {
        let _life = self.life.lock().unwrap();
        self.read_life("moved.json")
    }

    pub(super) fn set_move(&self, name: &str, m: Option<&Move>) -> Result<()> {
        let _life = self.life.lock().unwrap();
        let mut all: HashMap<String, Move> = self.read_life("moved.json");
        match m {
            None => {
                if all.remove(name).is_none() {
                    return Ok(());
                }
            }
            Some(m) => {
                all.insert(name.into(), m.clone());
            }
        }
        self.write_life("moved.json", &all)
    }

    /// Where the device named here went, once it's settled whether it did: a move that never finished asks the machine
    /// it was going to whether it has it.
    pub(super) fn moved_away(&self, name: &str) -> Result<Move> {
        let Some(mut m) = self.moves().get(name).cloned() else { return Ok(Move { to: String::new(), at: time::zero(), done: false }) };
        if m.done {
            return Ok(m);
        }
        let has = self.has_there(&m.to, name).map_err(|err| {
            anyhow!("device {name} was being moved to {}, which didn't answer whether it has it: it stays stopped here until it does ({err:#})", m.to)
        })?;
        if has {
            m.done = true;
            self.set_move(name, Some(&m))?;
            return Ok(m);
        }
        self.set_move(name, None)?;
        Ok(Move { to: String::new(), at: time::zero(), done: false })
    }

    fn has_there(&self, machine: &str, name: &str) -> Result<bool> {
        let req = json!({ "app": super::NAME, "service": super::NAME, "function": "list", "input": { "here": true }, "device": machine });
        let theirs = self.host.post_within("/api/call", &req, Duration::from_secs(20))?["ok"].clone();
        Ok(theirs["devices"].as_array().into_iter().flatten().any(|d| d["name"] == name && d["moved_to"].as_str().unwrap_or_default().is_empty()))
    }

    pub(super) fn may_start(&self, name: &str) -> Result<()> {
        let m = self.moved_away(name)?;
        if m.done {
            refuse!("device {name} moved to {} on {}: use it there; it's kept stopped here until you delete it here", m.to, super::local(m.at, "%-d %b %H:%M"));
        }
        Ok(())
    }

    pub(super) fn tend_moved(&self, all: &[Device]) {
        for name in self.moves().into_keys() {
            let Some(d) = all.iter().find(|d| d.name == name) else {
                let _ = self.set_move(&name, None);
                continue;
            };
            if d.state == "stopped" || !self.moved_away(&name).is_ok_and(|m| m.done) {
                continue;
            }
            self.drop_conn(&name);
            if let Err(err) = self.stop_device(d) {
                tracing::warn!("stopping {name}, moved away: {err:#}");
            }
        }
    }

    /// A device not yet in running.json runs as its runtime kept it: Podman's restart policy, a Mac's stopped marker.
    pub(super) fn revive(&self) {
        let mut all = Vec::new();
        if let Some(rt) = &self.rt {
            for i in 0.. {
                match rt.list() {
                    Ok(got) => {
                        all = got;
                        break;
                    }
                    Err(err) if i == 60 => {
                        tracing::warn!("not starting devices: {err:#}");
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_secs(5)),
                }
            }
        }
        let limits = self.limits(&all).inspect_err(|err| tracing::warn!("{err:#}")).unwrap_or_default();
        let w = self.wanted();
        let moved = self.moves();
        for d in &all {
            let mut should = match w.get(&d.name) {
                Some(s) => *s,
                None => {
                    let s = limits.get(&d.name).is_some_and(|l| l.restart);
                    self.want(&d.name, s);
                    s
                }
            };
            if moved.contains_key(&d.name) {
                should = false;
            }
            if should && d.state == "stopped" {
                let gb = (limits.get(&d.name).map(|l| l.memory).unwrap_or(0) >> 30).max(1) as u64;
                if !super::kinds::room_for(gb) {
                    tracing::info!("not starting {}, which should be running: no memory to spare", d.name);
                    continue;
                }
                tracing::info!("starting {}, which should be running", d.name);
                if let Some(rt) = &self.rt
                    && let Err(err) = rt.start(d)
                {
                    tracing::warn!("{err:#}");
                }
            } else if !should && d.state != "stopped" {
                tracing::info!("stopping {}, which should be stopped", d.name);
                if let Err(err) = self.stop_device(d) {
                    tracing::warn!("{err:#}");
                }
            }
        }
        let lent = self.loans();
        for mut v in self.vms() {
            let k = v.kind();
            if !w.get(&v.name).copied().unwrap_or(false) || !k.can().is_empty() || v.kind == super::chrome::NAME || k.running(self, &v) {
                continue;
            }
            if !lent.contains_key(&v.name) && v.throwaway() {
                continue;
            }
            if !super::kinds::room_for(super::kinds::gb_of(Some(&v))) {
                tracing::info!("not starting {}, which should be running: no memory to spare", v.name);
                continue;
            }
            tracing::info!("starting {}, which should be running", v.name);
            v.paused = false;
            self.remember(&v);
            if let Err(err) = k.start(self, &v) {
                tracing::warn!("starting {}: {err:#}", v.name);
            }
        }
    }
}
