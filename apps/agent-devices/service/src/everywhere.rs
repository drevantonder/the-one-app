//! This service reaches the same service on the user's other devices through its own node's local API, as any app
//! would: list and backups show every device's agent devices, and move restores one on another. Calls passed on carry
//! here: true, so they go no further.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};

use super::pool::Loan;
use super::{Devices, Input, NAME, time};

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct Listed {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub on: String,
    #[serde(default)]
    pub kind: String,
    /// The lasting device it's a throwaway copy of.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub of: String,
    /// running, paused or stopped.
    #[serde(default)]
    pub state: String,
    #[serde(with = "time::maybe", default, skip_serializing_if = "Option::is_none")]
    pub started: Option<jiff::Timestamp>,
    #[serde(default)]
    pub size: String,
    /// At most; none: no limit.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cpus: f64,
    /// Bytes.
    #[serde(default, skip_serializing_if = "is_zero_i")]
    pub memory_limit: i64,
    /// Bytes in use now.
    #[serde(default, skip_serializing_if = "is_zero_i")]
    pub memory: i64,
    /// Percent of one CPU, now.
    #[serde(default)]
    pub cpu: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borrowed: Option<Loan>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spare: bool,
    #[serde(with = "time::maybe", default, skip_serializing_if = "Option::is_none")]
    pub last_call: Option<jiff::Timestamp>,
    /// The machine it moved to: it's kept stopped here until deleted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub moved_to: String,
}

fn is_zero(n: &f64) -> bool {
    *n == 0.0
}

fn is_zero_i(n: &i64) -> bool {
    *n == 0
}

impl Devices {
    /// Runs `function` on the device `on` (by name; "" for wherever the node picks): its answer.
    pub(super) fn call_node(&self, on: &str, function: &str, input: &Value) -> Result<Value> {
        let mut req = json!({ "app": NAME, "service": NAME, "function": function, "input": input });
        if !on.is_empty() {
            req["device"] = json!(on);
        }
        Ok(self.host.post("/api/call", &req)?["ok"].clone())
    }

    fn call_node_within(&self, on: &str, function: &str, input: &Value, limit: Duration) -> Result<Value> {
        let req = json!({ "app": NAME, "service": NAME, "function": function, "input": input, "device": on });
        Ok(self.host.post_within("/api/call", &req, limit)?["ok"].clone())
    }

    pub(super) fn call_app(&self, tool: &str, input: &Value) -> Result<Value> {
        Ok(self.host.post("/api/call", &json!({ "app": NAME, "tool": tool, "input": input }))?["ok"].clone())
    }

    /// This device's name and the user's other online devices that run agent devices, as the node says.
    pub(super) fn hosts(&self) -> Result<(String, Vec<String>)> {
        let (me, all) = self.ordered()?;
        let others = all.into_iter().filter(|s| *s != me).collect();
        Ok((me, others))
    }

    /// This device's name and every online device that runs agent devices, this one too, in where things run first
    /// (the network's order).
    pub(super) fn ordered(&self) -> Result<(String, Vec<String>)> {
        let st = self.host.get("/api/state")?;
        let mut devices: Vec<&Value> = st["network"]["devices"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        devices.sort_by_key(|d| d["order"].as_i64().unwrap_or(0));
        let all = devices
            .iter()
            .filter(|d| d["here"] == true || d["online"] == true && d["services"].as_array().is_some_and(|s| s.iter().any(|s| s == NAME)))
            .filter_map(|d| d["name"].as_str().map(str::to_string))
            .collect();
        Ok((st["me"]["name"].as_str().unwrap_or_default().to_string(), all))
    }

    /// Runs `function` on every other device that runs agent devices, at once: each one's answer, or why it couldn't.
    pub(super) fn everywhere(&self, function: &str, input: Option<&Input>) -> (HashMap<String, Value>, BTreeMap<String, String>) {
        let mut got = HashMap::new();
        let mut failed = BTreeMap::new();
        let others = match self.hosts() {
            Ok((_, others)) => others,
            Err(err) => {
                failed.insert("this device".into(), format!("{err:#}"));
                return (got, failed);
            }
        };
        let mut pass = input.cloned().unwrap_or_default();
        pass.insert("here".into(), json!(true));
        let pass = Value::Object(pass);
        let answers: Vec<(String, Result<Value>)> = std::thread::scope(|s| {
            let asked: Vec<_> = others.iter().map(|on| (on.clone(), s.spawn(|| self.call_node_within(on, function, &pass, Duration::from_secs(20))))).collect();
            asked.into_iter().map(|(on, t)| (on, t.join().unwrap_or_else(|_| Err(anyhow!("failed"))))).collect()
        });
        for (on, r) in answers {
            match r {
                Ok(v) => {
                    got.insert(on, v);
                }
                Err(err) => {
                    failed.insert(on, format!("{err:#}"));
                }
            }
        }
        (got, failed)
    }

    pub(super) fn list_here(&self) -> Result<Vec<Listed>> {
        let me = self.hosts()?.0;
        let all = self.list()?;
        let limits = self.limits(&all)?;
        let usage = self.stats();
        let lent = self.loans();
        let used = self.used();
        let moved = self.moves();
        let mut out: Vec<Listed> = all
            .iter()
            .map(|d| {
                let l = limits.get(&d.name).copied().unwrap_or_default();
                let u = usage.get(&d.name).copied().unwrap_or_default();
                Listed {
                    name: d.name.clone(),
                    on: me.clone(),
                    kind: super::kinds::LINUX.into(),
                    state: d.state.clone(),
                    started: d.started,
                    size: l.size(),
                    cpus: l.cpus,
                    memory_limit: l.memory,
                    memory: u.memory,
                    cpu: u.cpu,
                    spare: d.pool && !lent.contains_key(&d.name),
                    borrowed: lent.get(&d.name).cloned(),
                    last_call: used.get(&d.name).copied(),
                    moved_to: moved.get(&d.name).map(|m| m.to.clone()).unwrap_or_default(),
                    ..Default::default()
                }
            })
            .collect();
        out.extend(self.list_vms(&me, &lent));
        Ok(out)
    }

    /// The agent devices on every device, this one's first: with keep, as each of them last knew it.
    pub(super) fn list_all(&self, keep: bool) -> Result<Value> {
        let mut all: Vec<Value> = self.list_here()?.into_iter().map(|l| serde_json::to_value(l).unwrap_or_default()).collect();
        let (got, mut failed) = self.everywhere("list", Some(&serde_json::Map::from_iter([("kept".to_string(), json!(keep))])));
        let mut names: Vec<&String> = got.keys().collect();
        names.sort();
        for on in names {
            match got[on]["devices"].as_array() {
                Some(theirs) => all.extend(theirs.iter().cloned()),
                None => {
                    failed.insert(on.clone(), "it gave no list".into());
                }
            }
        }
        let empty = all.is_empty();
        let mut out = json!({ "devices": all });
        if empty {
            out["note"] = json!("No devices. Borrow one with borrow, or make a lasting one with create.");
        }
        if !failed.is_empty() {
            out["unreachable"] = json!(failed.iter().map(|(s, e)| format!("{s}: {e}")).collect::<Vec<_>>().join("; "));
        }
        Ok(out)
    }

    /// The other device that has the agent device named, or "".
    pub(super) fn located(&self, name: &str) -> String {
        let (got, _) = self.everywhere("list", None);
        for (on, r) in got {
            if r["devices"].as_array().into_iter().flatten().any(|d| d["name"] == name && d["moved_to"].as_str().unwrap_or_default().is_empty()) {
                return on;
            }
        }
        String::new()
    }

    /// Runs `function` (create, borrow or sign_in) of a kind on the first device that runs it and has room, in where things run
    /// first; for a borrow, one with a lasting device of the kind nobody has borrowed comes before those. None when it's
    /// this device's turn, for it to run `function` itself, or when no other device runs the kind.
    pub(super) fn first(&self, function: &str, kind: &str, name: &str, input: &Input) -> Option<Result<Value>> {
        let (me, all) = self.ordered().ok()?;
        let (mut got, _) = self.everywhere("kinds", None);
        got.insert(me.clone(), json!({ "kinds": self.kinds_here() }));
        let lend = function == "borrow" && input.get("copy") != Some(&json!(true));
        // How ready each device that runs the kind is: 0 a lasting one to lend, 1 room for one, 2 neither.
        let mut ready: HashMap<String, u8> = HashMap::new();
        for (on, r) in &got {
            for k in r["kinds"].as_array().into_iter().flatten() {
                if k["kind"] != kind || k["unavailable"].as_str().is_some_and(|u| !u.is_empty()) {
                    continue;
                }
                let n = if lend && k["unborrowed"].as_i64().unwrap_or(0) > 0 {
                    0
                } else if k["room"].as_bool().unwrap_or(true) {
                    1 // none: a device too old to say
                } else {
                    2
                };
                ready.insert(on.clone(), n);
            }
        }
        let mut can: Vec<String> = all.into_iter().filter(|s| ready.contains_key(s)).collect();
        can.sort_by_key(|s| ready[s]);
        let mut pass = input.clone();
        pass.insert("here".into(), json!(true));
        if !name.is_empty() {
            pass.insert("device".into(), json!(name));
        }
        let pass = Value::Object(pass);
        let mut tried = Vec::new();
        let mut all_refused = true;
        for on in can {
            if on == me {
                return None;
            }
            match self.call_node(&on, function, &pass) {
                Ok(r) => return Some(Ok(r)),
                Err(err) => {
                    tracing::info!("{function} on {on}: {err:#}");
                    all_refused &= err.is::<one_service::Refused>();
                    tried.push(format!("{on}: {err:#}"));
                }
            }
        }
        if tried.is_empty() {
            return None;
        }
        let said = tried.join("; ");
        Some(Err(if all_refused { one_service::refused(said) } else { anyhow!("{said}") }))
    }

    /// The other device with an agent device signing in to the profile, or "".
    pub(super) fn signing_in_elsewhere(&self, item: &str) -> String {
        let (got, _) = self.everywhere("signing_in", None);
        for (on, r) in got {
            if r["signing_in"].as_array().into_iter().flatten().any(|l| l["item"] == item) {
                return on;
            }
        }
        String::new()
    }
}
