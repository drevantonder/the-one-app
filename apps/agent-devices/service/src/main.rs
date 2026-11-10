//! Agent Devices' service: devices for agents, keeping what it keeps in its folder (`<home>/agent-devices`). Linux ones (kind linux) are each a container running Cua's desktop
//! image (github.com/trycua/cua) with Google Chrome, which keeps its sign-ins (linux.rs); the server inside each device
//! speaks MCP (mcp.rs), and its browser tools become this service's functions. Chrome ones (kind chrome, chrome.rs)
//! are this machine's own Chrome, whose browser tools this service answers itself (browse.rs). Windows, Android and
//! iPhones are each a virtual machine or an emulator (kinds.rs).
//!
//! A program in an agent device reaches nothing of this machine's 127.0.0.1, which is for its own programs: Windows is
//! on passt, Android on Linux in a network of its own (pasta), and `reach` opens one port the asker chose, never the
//! node's, adb's or another device's. On a Mac nothing can do that: an Android phone's apps reach the Mac's 127.0.0.1
//! at 10.0.2.2, adb's server among it, and an iPhone simulator's share the Mac's network and files, its node's token
//! too.
//!
//! What it keeps in its folder, in the files the Go service before it kept: devices/ (each device of a kind other than
//! linux), lent.json, used.json, running.json, moved.json, backups.json, tools.json, chrome.json, and each Windows
//! device's and Android phone's files.

mod android;
mod backup;
mod browse;
mod chrome;
mod cookies;
mod everywhere;
mod iphone;
mod kinds;
mod latest;
mod lifecycle;
mod linux;
mod mcp;
mod picture;
mod pool;
mod profiles;
mod viewer;
mod windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use one_service::{Answer, Call, Host, Service, refuse};
use latest::{Latest, Pictures};
use linux::{Containers, Device};
use mcp::{Conn, Tool};
use pool::Loan;

pub const NAME: &str = "agent-devices";

type Input = Map<String, Value>;

/// Who a call is from: the device it came from, and its user's id.
#[derive(Clone, Default)]
pub struct Caller {
    pub name: String,
    pub user: String,
}

impl Caller {
    fn name(&self) -> String {
        or(&self.name, SOMEONE)
    }
}

/// Who borrowed what a caller with no name did, which never carries on another's loan.
const SOMEONE: &str = "someone";

/// This service's state, for as long as it runs.
pub struct Devices {
    host: Host,
    dir: PathBuf,
    /// Linux devices' container runtime, or none where this machine has none: then it runs only devices of other kinds.
    rt: Option<Box<dyn Containers>>,
    /// The devices' tools, as a running Linux device gave them, kept for when none is running.
    tools: Mutex<Vec<Tool>>,
    /// Each Linux device's MCP session, made the first time it's needed.
    conns: Mutex<HashMap<String, Arc<Conn>>>,
    pool: Mutex<pool::Pool>,
    /// One borrow, or spare made, at a time.
    take: Mutex<()>,
    kick: (Mutex<bool>, std::sync::Condvar),
    started: jiff::Timestamp,
    /// Two devices being made at once mustn't take the same port.
    creating: Mutex<()>,
    vm_files: Mutex<()>,
    life: Mutex<()>,
    backups: Mutex<()>,
    building: Mutex<()>,
    latest_here: Arc<Latest>,
    latest_all: Arc<Latest>,
    pics: Pictures,
    /// Each Android phone's bytes through adb at the last look.
    traffic: Mutex<HashMap<String, i64>>,
}

static DEVICES: OnceLock<Arc<Devices>> = OnceLock::new();

fn devices(host: &Host) -> Arc<Devices> {
    DEVICES
        .get_or_init(|| {
            let _ = std::fs::create_dir_all(&host.dir);
            let rt = linux::runtime(&host.dir).map_err(|err| tracing::info!("no Linux devices here: {err:#}")).ok();
            Arc::new(Devices::new(host.clone(), rt))
        })
        .clone()
}

impl Devices {
    fn new(host: Host, rt: Option<Box<dyn Containers>>) -> Devices {
        let dir = host.dir.clone();
        let tools = std::fs::read(dir.join("tools.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let ds = Devices {
            host,
            dir,
            rt,
            tools: Mutex::new(tools),
            conns: Mutex::new(HashMap::new()),
            pool: Mutex::new(pool::Pool::default()),
            take: Mutex::new(()),
            kick: (Mutex::new(false), std::sync::Condvar::new()),
            started: jiff::Timestamp::now(),
            creating: Mutex::new(()),
            vm_files: Mutex::new(()),
            life: Mutex::new(()),
            backups: Mutex::new(()),
            building: Mutex::new(()),
            latest_here: Arc::new(Latest::new(Duration::from_secs(2), Duration::from_secs(24 * 3600))),
            latest_all: Arc::new(Latest::new(Duration::from_secs(2), Duration::from_secs(24 * 3600))),
            pics: Pictures::default(),
            traffic: Mutex::new(HashMap::new()),
        };
        ds.load_loans();
        ds
    }

    /// One in `dir`, with no node and no Linux devices, for tests.
    #[cfg(test)]
    pub(crate) fn for_test(dir: &std::path::Path) -> Devices {
        Devices::new(Host::for_test(dir), None)
    }
}

/// Serves, or given `delete-all <folder>` (`one-dev throwaway stop`), deletes the Linux devices a folder of its kept.
fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(filter).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [command, dir] = args.as_slice()
        && command == "delete-all"
    {
        return linux::delete_all(std::path::Path::new(dir));
    }
    one_service::serve(Service { name: NAME, prefer_here: &[], call, start: Some(start) })
}

fn start(host: &Host) {
    let ds = devices(host);
    let d = ds.clone();
    std::thread::spawn(move || {
        let _ = d.refresh();
    });
    let d = ds.clone();
    std::thread::spawn(move || d.keep());
    std::thread::spawn(move || ds.serve_viewer());
}

fn call(host: &Host, call: &Call) -> Result<Answer> {
    let ds = devices(host);
    let caller = Caller { name: call.caller.name.clone(), user: call.caller.user.clone() };
    let input = match &call.input {
        Value::Null => Map::new(),
        Value::Object(o) => o.clone(),
        _ => refuse!("bad input: an object"),
    };
    ds.run(&caller, &call.function, input)
}

/// The device's browser tools, which this service's functions are; the rest are reached through `use`.
const BROWSING: [&str; 8] = ["get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_pointer", "browser_dialog"];

/// The functions that act on this device even when given a device it doesn't have.
const LOCAL: [&str; 12] = ["list", "kinds", "chrome", "set_chrome", "adopt", "create", "borrow", "sign_in", "signing_in", "backups", "restore", "refresh_tools"];

/// The functions after which the list isn't what it was.
const CHANGES: [&str; 12] = ["adopt", "borrow", "give_back", "sign_in", "save_profile", "create", "resize", "start", "stop", "delete", "restore", "move"];

fn text(input: &Input, key: &str) -> String {
    input.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

fn number(input: &Input, key: &str) -> Option<f64> {
    input.get(key).and_then(Value::as_f64)
}

fn flag(input: &Input, key: &str) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn or(a: &str, b: &str) -> String {
    if a.is_empty() { b.to_string() } else { a.to_string() }
}

/// Minutes given as `minutes`, at least 1 and at most `most`, or `default`.
fn minutes(input: &Input, default: i64, most: i64) -> i64 {
    number(input, "minutes").map(|m| (m as i64).clamp(1, most)).unwrap_or(default)
}

/// A picture's width given as `width`, from 160 to 1920, or 640.
fn width(input: &Input) -> u32 {
    number(input, "width").map(|w| (w as i64).clamp(160, 1920) as u32).unwrap_or(640)
}

fn now() -> jiff::Timestamp {
    jiff::Timestamp::now()
}

fn since(t: jiff::Timestamp) -> Duration {
    Duration::try_from(now().duration_since(t)).unwrap_or_default()
}

/// A time as this machine's clock reads it, in `format` (strftime's).
fn local(t: jiff::Timestamp, format: &str) -> String {
    t.to_zoned(jiff::tz::TimeZone::system()).strftime(format).to_string()
}

fn bytes_text(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GB", n as f64 / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.0} MB", n as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", n >> 10)
    }
}

fn random_hex(n: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    data_encoding::HEXLOWER.encode(&b)
}

/// A string in single quotes for a POSIX shell.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Times as Go's encoding/json writes and reads them, RFC 3339, so what the Go service kept reads the same. Go's zero
/// time (0001-01-01) is a time not given.
mod time {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn zero() -> jiff::Timestamp {
        "0001-01-01T00:00:00Z".parse().unwrap()
    }

    pub fn is_zero(t: &jiff::Timestamp) -> bool {
        *t <= zero()
    }

    pub fn serialize<S: Serializer>(t: &jiff::Timestamp, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<jiff::Timestamp, D::Error> {
        let text = String::deserialize(d)?;
        Ok(text.parse().unwrap_or_else(|_| zero()))
    }

    /// A time that may not be given: none for Go's zero time.
    pub mod maybe {
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(t: &Option<jiff::Timestamp>, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&t.unwrap_or_else(super::zero).to_string())
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<jiff::Timestamp>, D::Error> {
            let text = Option::<String>::deserialize(d)?;
            Ok(text.and_then(|t| t.parse().ok()).filter(|t| !super::is_zero(t)))
        }
    }
}

/// The content of a tool's answer, as MCP gives it, in the shape the node passes on.
fn contents(items: Vec<Value>) -> Value {
    json!({ "content": items })
}

fn picture_answer(jpeg: &[u8], said: String) -> Value {
    contents(vec![json!({ "type": "image", "mimeType": "image/jpeg", "data": data_encoding::BASE64.encode(jpeg) }), json!({ "type": "text", "text": said })])
}

impl Devices {
    fn run(&self, caller: &Caller, function: &str, mut input: Input) -> Result<Answer> {
        let name = text(&input, "device");
        input.remove("device");
        let here = flag(&input, "here");
        input.remove("here");
        if !name.is_empty() && !here && (!LOCAL.contains(&function) || function == "borrow") && self.vm_named(&name).is_none() {
            // An agent device on another of the user's devices: the call goes on to it. One moved from here is
            // there, but for deleting it here.
            let mut pass = |on: &str| {
                input.insert("device".into(), json!(name));
                input.insert("here".into(), json!(true));
                self.call_node(on, function, &Value::Object(input.clone())).map(Answer::new)
            };
            if let Some(m) = self.moves().get(&name)
                && m.done
                && function != "delete"
            {
                let to = m.to.clone();
                return pass(&to);
            }
            if self.find(&name).is_err() {
                let on = self.located(&name);
                if !on.is_empty() {
                    return pass(&on);
                }
            }
        }
        // What the page asks for, kept: it asks again soon, and has a fresh one then.
        let keep = flag(&input, "kept");
        input.remove("kept");
        if CHANGES.contains(&function) {
            self.latest_here.forget();
            self.latest_all.forget();
            if !name.is_empty() {
                self.pics.forget(&name);
            }
        }
        if function == "list" && keep {
            return self.list_latest(here).map(Answer::new);
        }
        if function == "screen" && keep && !name.is_empty() {
            let have = text(&input, "have");
            let w = width(&input);
            return self.screen_latest(&name, w, &have).map(Answer::new);
        }
        if function == "chrome" || function == "set_chrome" {
            return self.run_chrome(caller, function, here, &input).map(Answer::new);
        }
        if let Some(r) = self.run_kinds(caller, function, &name, here, &mut input) {
            return r.map(Answer::new);
        }
        self.run_linux_function(caller, function, &name, here, input)
    }

    /// The functions on Linux devices, and those of every kind, once `run_kinds` has passed on them.
    fn run_linux_function(&self, caller: &Caller, function: &str, name: &str, here: bool, input: Input) -> Result<Answer> {
        let ok = |v: Value| Ok(Answer::new(v));
        match function {
            "list" => {
                if here {
                    let all = self.list_here()?;
                    return ok(json!({ "devices": all }));
                }
                ok(self.list_all(false)?)
            }
            "create" => {
                let size = linux::size_named(&text(&input, "size"))?;
                if !here && let Some(r) = self.first(function, kinds::LINUX, name, &input) {
                    return r.map(Answer::new);
                }
                if self.vm_named(name).is_some() {
                    refuse!("there's already a device named {name}");
                }
                let d = self.create(name, size, false, true)?;
                self.touch(&d.name);
                self.refresh_if_empty();
                ok(json!(format!("Created device {}, {}. Its browser is yours to sign in to sites with: open it with view.", d.name, size)))
            }
            "resize" => {
                let size = linux::size_named(&text(&input, "size"))?;
                let d = self.find(name)?;
                self.rt()?.resize(&d, size)?;
                ok(json!(format!("Device {} is now {}.", d.name, size)))
            }
            "start" => {
                let d = self.find(name)?;
                self.may_start(&d.name)?;
                if d.state != "stopped" {
                    if d.state == "paused" {
                        self.running(&d.name)?;
                    }
                    return ok(json!("It's running."));
                }
                self.drop_conn(&d.name);
                self.start_device(&d)?;
                self.touch(&d.name);
                self.refresh_if_empty();
                ok(json!("Started."))
            }
            "stop" => {
                let d = self.find(name)?;
                self.drop_conn(&d.name);
                self.stop_device(&d)?;
                ok(json!("Stopped. Its files and sign-ins stay."))
            }
            "delete" => {
                let d = self.find(name)?;
                if self.loans().contains_key(&d.name) {
                    // Given back, which saves a sign-in's profile first.
                    self.give_back(&d.name, &format!("deleted by {}", caller.name()))?;
                    return ok(json!(format!("Deleted device {} and everything in it.", d.name)));
                }
                self.drop_conn(&d.name);
                self.remove_device(&d)?;
                self.set_move(&d.name, None)?;
                ok(json!(format!("Deleted device {} and everything in it.", d.name)))
            }
            "view" => {
                let d = self.running(name)?;
                if self.loans().get(&d.name).is_some_and(|l| l.no_screen) {
                    refuse!("device {} was made without a screen: there's nothing to watch", d.name);
                }
                let minutes = minutes(&input, 60, 1440);
                let url = self.view_url(&d, minutes)?;
                ok(json!(format!("{url}\nThe link works for {minutes} minutes, on the user's own devices.")))
            }
            "screen" => ok(self.screen_now(name, width(&input))?),
            "tools" => ok(self.list_tools(name, &text(&input, "name"))?),
            "refresh_tools" => {
                let n = self.refresh()?;
                ok(json!(format!("Read {n} tools from the devices.")))
            }
            "use" => {
                let tool = text(&input, "tool");
                let args = input.get("args").and_then(Value::as_object).cloned().unwrap_or_default();
                ok(self.use_tool(name, &tool, args)?)
            }
            "borrow" => {
                let why = text(&input, "for");
                if why.is_empty() {
                    refuse!("say what it's for, in a few words: the user sees it");
                }
                let p = profiles::profile_in(&input)?;
                if !here && let Some(r) = self.first(function, kinds::LINUX, "", &input) {
                    return r.map(Answer::new);
                }
                let loan = Loan { by: or(&text(&input, "by"), &caller.name()), r#for: why, no_screen: input.get("screen") == Some(&json!(false)), ..Default::default() };
                let d = self.borrow(loan, p.clone())?;
                let me = self.me();
                let mut out = json!({ "device": d.name, "on": me });
                let mut msg = format!("Borrowed device {}: pass device {:?} to the other tools.", d.name, d.name);
                if let Some(p) = &p {
                    out["profile"] = json!(p.name);
                    msg += &format!(" Its browser starts with a copy of the {} profile: browser_prepare with allow_launch true and profile {{\"mode\":\"isolated_named\",\"name\":{:?}}}.", p.name, p.name);
                }
                out["text"] = json!(msg + " Give it back with give_back when done; it's given back for you after 15 minutes without a call.");
                ok(out)
            }
            "give_back" => {
                if name.is_empty() {
                    refuse!("which device? pass the one you borrowed");
                }
                let l = self.loans().get(name).cloned();
                if let Some(l) = l.as_ref().filter(|l| l.waits_for_key()) {
                    // The app saves it, with the key it gives: a service's call of its app can't call back.
                    return ok(json!({ "save": l.item }));
                }
                self.give_back(name, &format!("given back by {}", caller.name()))?;
                if let Some(l) = l.filter(|l| l.signing_in) {
                    return ok(json!(format!("Saved the {} profile and closed device {name}.", l.profile)));
                }
                ok(json!(format!("Gave back device {name}.")))
            }
            "sign_in" => {
                let p = profiles::profile_in(&input)?.ok_or_else(|| one_service::refused("which browser profile? the app gives it"))?;
                if !here && !self.signing_in_here(&p.item) && let Some(r) = self.first(function, kinds::LINUX, "", &input) {
                    return r.map(Answer::new);
                }
                let minutes = minutes(&input, pool::SIGN_IN_FOR.as_secs() as i64 / 60, 240);
                let (d, until) = self.sign_in(&p, &text(&input, "url"), minutes, &or(&text(&input, "by"), &caller.name()))?;
                let url = self.view_url(&d, minutes)?;
                let msg = format!("Sign in to sites in the {} profile here:\n{url}\nIt's saved in {minutes} minutes, or as soon as it's saved (save_profile).", p.name);
                ok(json!({ "device": d.name, "on": self.me(), "profile": p.name, "url": url, "until": until.to_string(), "text": msg }))
            }
            "save_profile" => self.save_profile_function(name, here, input).map(Answer::new),
            "signing_in" => {
                // Which profiles are being signed in to here: their items, with the agent device and the device it's on.
                let me = self.me();
                let out: Vec<Value> = self
                    .loans()
                    .iter()
                    .filter(|(_, l)| l.signing_in)
                    .map(|(n, l)| json!({ "item": l.item, "profile": l.profile, "device": n, "on": me, "until": l.until.unwrap_or_else(time::zero).to_string() }))
                    .collect();
                ok(json!({ "signing_in": out }))
            }
            "backup" => {
                let d = self.running(name)?;
                let rec = self.backup(&d)?;
                let said = format!("Backed up device {} ({}). Restore it on any of your devices with restore.", d.name, bytes_text(rec.bytes));
                Ok(Answer { ok: json!({ "backup": rec, "text": said }), blobs: vec![rec.blob.clone()] })
            }
            "backups" => {
                if here {
                    return ok(json!({ "backups": self.backups_here() }));
                }
                let (all, failed) = self.backups_all(name);
                let mut out = json!({ "backups": all });
                if !failed.is_empty() {
                    out["unreachable"] = json!(failed);
                }
                ok(out)
            }
            "restore" => {
                let rec = self.find_backup(name, &text(&input, "blob"))?;
                let r#as = or(&text(&input, "as"), &or(name, &rec.device));
                let size = linux::size_named(&or(&text(&input, "size"), &rec.size)).unwrap_or(linux::SIZES[0]);
                let d = self.restore(&r#as, &rec, size)?;
                self.touch(&d.name);
                ok(json!(format!("Restored device {} from its backup of {}.", d.name, local(rec.at, "%Y-%m-%d %H:%M:%S"))))
            }
            "move" => self.move_device(name, &text(&input, "to")).map(Answer::new),
            function if BROWSING.contains(&function) => ok(self.use_tool(name, function, input)?),
            function => bail!("no function {function}"),
        }
    }

    fn save_profile_function(&self, name: &str, here: bool, mut input: Input) -> Result<Value> {
        // The device signing in to the profile, or the one named, with the profile the app gives (and its key).
        let mut p = profiles::profile_in(&input)?;
        let mut name = name.to_string();
        if name.is_empty()
            && let Some(p) = &p
        {
            for (n, l) in self.loans() {
                if l.signing_in && l.item == p.item {
                    name = n;
                }
            }
            if name.is_empty() && !here {
                let on = self.signing_in_elsewhere(&p.item);
                if !on.is_empty() {
                    input.insert("here".into(), json!(true));
                    return self.call_node(&on, "save_profile", &Value::Object(input));
                }
            }
            if name.is_empty() {
                refuse!("nobody is signing in to the {} profile: there's nothing to save", p.name);
            }
        }
        let loan = self.loans().get(&name).cloned();
        match (&mut p, &loan) {
            (None, Some(l)) => p = l.key.clone(),
            (Some(p), Some(l)) => {
                if let Some(k) = l.key.as_ref().filter(|k| k.item == p.item) {
                    p.name = k.name.clone(); // the folder it was opened with, if it was renamed since
                }
            }
            _ => {}
        }
        let mut p = p.ok_or_else(|| one_service::refused("which browser profile? the app gives it"))?;
        if loan.as_ref().is_some_and(|l| l.signing_in && l.item == p.item) {
            // Given back, which starts it if it's stopped.
            self.hold_key(&name, &p);
            self.give_back(&name, "signed in")?;
            return Ok(json!(format!("Saved the {} profile and closed device {name}.", p.name)));
        }
        let d = self.running(&name)?;
        self.save_profile(&d, Some(&mut p))?;
        Ok(json!(format!("Saved device {}'s browser as the {} profile: devices borrowed with it start signed in to the same sites.", d.name, p.name)))
    }

    fn move_device(&self, name: &str, to: &str) -> Result<Value> {
        if to.is_empty() {
            refuse!("move it to which of your devices? pass to");
        }
        let d = self.running(name)?;
        if self.loans().contains_key(&d.name) {
            refuse!("a borrowed device can't be moved: borrow one on the device you'd move it to");
        }
        self.close_browsers(&d, true);
        // Backed up through the node, so the entry naming the backup's blob is written before the other device asks
        // for it.
        let me = self.hosts()?.0;
        let backed = self.call_node(&me, "backup", &json!({ "device": d.name, "here": true }))?;
        let rec: backup::Record = serde_json::from_value(backed["backup"].clone()).ok().filter(|r: &backup::Record| !r.blob.is_empty()).ok_or_else(|| anyhow!("backing it up: {backed}"))?;
        // Kept before it's restored there, so a stop anywhere from here on leaves it stopped here, asking there.
        let mut m = lifecycle::Move { to: to.to_string(), at: now(), done: false };
        self.set_move(&d.name, Some(&m)).map_err(|err| anyhow!("not moved: {err:#}"))?;
        if let Err(err) = self.call_node(to, "restore", &json!({ "device": d.name, "blob": rec.blob, "size": rec.size })) {
            // Its answer may be what was lost: it's settled by asking there.
            match self.moved_away(&d.name) {
                Err(_) => {
                    self.drop_conn(&d.name);
                    let _ = self.stop_device(&d);
                    bail!("restoring it on {to} failed, but it may be there: it's kept stopped here until {to} says ({err:#})");
                }
                Ok(got) if !got.done => bail!("not moved: restoring it on {to}: {err:#}"),
                Ok(_) => {}
            }
        } else {
            m.done = true;
            if let Err(err) = self.set_move(&d.name, Some(&m)) {
                tracing::warn!("keeping that {} moved: {err:#}", d.name);
            }
        }
        self.drop_conn(&d.name);
        self.stop_device(&d).map_err(|err| anyhow!("moved to {to}, but stopping it here failed: it's stopped again shortly: {err:#}"))?;
        Ok(json!(format!("Moved device {} to {to}, where it's running. It's stopped here, and won't start here again; delete it here (delete) once it's fine there.", d.name)))
    }

    fn use_tool(&self, name: &str, tool: &str, args: Input) -> Result<Value> {
        if tool.is_empty() {
            refuse!("which tool? computer_tools lists them");
        }
        let d = self.running(name)?;
        let r = self.tool_call(&d, tool, args)?;
        r.content()
    }

    /// The device's tools other than the browser's, or one in full.
    fn list_tools(&self, device: &str, name: &str) -> Result<Value> {
        let mut tools = self.tools.lock().unwrap().clone();
        if tools.is_empty() {
            self.running(device)?;
            let _ = self.refresh();
            tools = self.tools.lock().unwrap().clone();
        }
        if !name.is_empty() {
            if let Some(t) = tools.iter().find(|t| t.name == name) {
                return Ok(json!(format!("{}\n\nInput: {}", t.description, t.input_schema)));
            }
            let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            refuse!("no tool {name}: there's {}", names.join(", "));
        }
        let lines: Vec<String> = tools.iter().filter(|t| !BROWSING.contains(&t.name.as_str())).map(|t| format!("{}: {}", t.name, gist(&t.description))).collect();
        Ok(json!(lines.join("\n").trim()))
    }

    fn refresh_if_empty(&self) {
        if self.tools.lock().unwrap().is_empty() {
            let _ = self.refresh();
        }
    }

    /// Reads the tools from a running Linux device: every device runs the same image, so any one will do. How many
    /// there are.
    fn refresh(&self) -> Result<usize> {
        let all = self.list().inspect_err(|err| tracing::warn!("{err:#}"))?;
        let mut last = anyhow!("no device is running to read them from");
        for d in all.iter().filter(|d| d.state == "running") {
            match self.conn(d).and_then(|c| c.tools()) {
                Ok(tools) => {
                    let n = tools.len();
                    if let Err(err) = std::fs::write(self.dir.join("tools.json"), serde_json::to_vec(&tools)?) {
                        tracing::warn!("{err}");
                    }
                    *self.tools.lock().unwrap() = tools;
                    return Ok(n);
                }
                Err(err) => {
                    tracing::info!("reading {}'s tools: {err:#}", d.name);
                    last = err;
                }
            }
        }
        Err(last)
    }

    /// A picture of the device named, of any kind, taken now.
    fn screen_now(&self, name: &str, width: u32) -> Result<Value> {
        if let Some(mut v) = self.vm_named(name) {
            if !v.kind().running(self, &v) {
                refuse!("{} is stopped: start it first", v.name);
            }
            self.awake(&mut v)?;
            return self.screen_vm(&v, width);
        }
        let d = self.running(name)?;
        self.screen(&d, width)
    }

    fn screen(&self, d: &Device, width: u32) -> Result<Value> {
        let r = self.tool_call(d, "get_desktop_state", Map::from_iter([("max_image_dimension".to_string(), json!(width))]))?;
        for c in &r.content {
            let data = c["data"].as_str().unwrap_or_default();
            if c["type"] != "image" || data.is_empty() {
                continue;
            }
            let raw = data_encoding::BASE64.decode(data.as_bytes())?;
            let img = picture::decode(&raw)?;
            let jpeg = picture::jpeg(&img, 70)?;
            return Ok(picture_answer(&jpeg, format!("Device {} now.", d.name)));
        }
        bail!("the device sent no picture")
    }

    /// The name of this device, as its node says; "" if it can't say now.
    fn me(&self) -> String {
        self.hosts().map(|h| h.0).unwrap_or_default()
    }

    fn poke(&self) {
        *self.kick.0.lock().unwrap() = true;
        self.kick.1.notify_all();
    }

    /// Gives back borrowed devices left idle or whose sign-in time is up, keeps one spare, pauses idle devices and
    /// starts again those that should be running, for as long as this runs.
    fn keep(&self) {
        self.revive();
        loop {
            self.tend();
            let mut kicked = self.kick.0.lock().unwrap();
            if !*kicked {
                kicked = self.kick.1.wait_timeout(kicked, Duration::from_secs(30)).unwrap().0;
            }
            *kicked = false;
        }
    }
}

/// A description's first sentence.
fn gist(s: &str) -> String {
    let s = s.split('\n').next().unwrap_or_default();
    match s.find(". ") {
        Some(i) => s[..=i].to_string(),
        None => s.to_string(),
    }
}

/// Waits for `done` up to `limit`, every `every`: whether it was.
fn wait_for(limit: Duration, every: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(every);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the Go service kept reads the same: its times, with an offset or none given.
    #[test]
    fn go_times_read_the_same() {
        let l: Loan = serde_json::from_str(r#"{"by":"a","for":"b","since":"2026-11-05T10:01:02.123456789+10:00","until":"0001-01-01T00:00:00Z","state":"preparing"}"#).unwrap();
        assert_eq!(l.since.to_string(), "2026-11-05T00:01:02.123456789Z");
        assert!(l.until.is_none());
        let back = serde_json::to_value(&l).unwrap();
        assert_eq!(back["since"], "2026-11-05T00:01:02.123456789Z");
        assert!(back.get("until").is_none());
        assert_eq!(back["state"], "preparing");
    }
}
