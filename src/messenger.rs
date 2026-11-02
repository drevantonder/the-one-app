//! Messenger: Facebook Messenger, read and sent through the browser of a device borrowed from Agent Devices, with a copy
//! of a browser profile signed in to messenger.com: one per account. Messenger has no API for personal accounts and its
//! chats are end-to-end encrypted, so messenger.com in a browser is the way in. A hosted sidecar
//! (`sidecars::Hosted`); its folder keeps profiles.json, the profiles it's linked to.

mod parse;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::sidecars::{Call, Host};
use parse::{Chat, MENU, NoChat, Node, pick};

pub const NAME: &str = "messenger";

const HOME: &str = "https://www.messenger.com/";

/// How long a borrowed device is kept without a call.
const IDLE: Duration = Duration::from_secs(5 * 60);

pub fn describe() -> Value {
    let chat = json!({ "type": "string", "description": "the chat's name, or a part of it only one recent chat has" });
    let account = json!({ "type": "string", "description": "which account: the browser profile, as messenger_accounts lists them; default the first" });
    let with_account = |mut props: Value, required: &[&str]| {
        props["account"] = account.clone();
        let mut o = json!({ "type": "object", "properties": props });
        if !required.is_empty() {
            o["required"] = json!(required);
        }
        o
    };
    json!({
        "title": "Messenger", "description": "Reads and sends your Messenger chats in an agent device's browser",
        "icon": "messenger-logo", "color": "#0866ff", "domain": "messenger.com",
        "instructions": "Messenger is the person's Facebook Messenger, read through messenger.com in an agent device's browser. Each account is a browser profile signed in to it (Personal, PTC), as messenger_accounts lists them; the others take account, the first by default. messenger_chats lists recent chats, messenger_read reads one, messenger_search finds messages. Each takes some seconds: a browser does the reading. If Messenger isn't signed in, messenger_sign_in gives the person a link to sign in; never sign in for them. messenger_send sends a real message to a real person: only when the person asked, with the exact text they approved.",
        "ops": [
            { "name": "accounts", "reads": true, "description": "The browser profiles Messenger reads, the default first: each { account, ok }, with why when ok is false and it needs signing in again.",
              "input": { "type": "object", "properties": {} } },
            { "name": "chats", "reads": true, "description": "Recent Messenger chats, newest first: who, when, their last message, and whether unread.",
              "input": with_account(json!({ "unread": { "type": "boolean", "description": "only unread chats" } }), &[]) },
            { "name": "read", "description": "A recent chat's latest messages. Opening a chat marks it read on Messenger, so one that was unread is marked unread again afterwards; the sender may still see it as seen.",
              "input": with_account(json!({ "chat": chat }), &["chat"]) },
            { "name": "search", "reads": true, "description": "Messages containing some words, across every chat: which chats have them, and the message when a chat has one.",
              "input": with_account(json!({ "query": { "type": "string", "description": "words to find" } }), &["query"]) },
            { "name": "send", "confirm": true, "description": "Send a real message, as the person, to a recent chat or to someone by their exact name. It can't be taken back: only when the person asked, with the text they approved.",
              "input": with_account(json!({ "chat": { "type": "string", "description": "the chat's name, or a part of it only one recent chat has; anyone else by their exact name" }, "text": { "type": "string", "description": "the message" } }), &["chat", "text"]) },
            { "name": "sign_in", "description": "For the person: links Messenger to a browser profile (made if new) and gives a link to messenger.com in it, to sign in there themselves. The default profile's again when none is given.",
              "input": { "type": "object", "properties": { "profile": { "type": "string", "description": "the browser profile, like Personal or PTC; agent-devices_profiles lists them" } } } },
            { "name": "sign_out", "description": "Stop reading a browser profile's Messenger. The profile, and its sign-ins to other sites, stay.",
              "input": { "type": "object", "required": ["account"], "properties": { "account": account } } },
        ],
        "screens": [{ "name": "settings", "title": "Messenger" }],
        "widgets": [],
    })
}

/// A browser profile Messenger reads isn't signed in to it.
#[derive(Debug)]
struct NotSignedIn;

impl std::fmt::Display for NotSignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Messenger isn't signed in on that browser profile. Ask the person to sign in again: messenger_sign_in with the profile gives them a link, where they sign in at messenger.com (with Keep me signed in), then save the profile")
    }
}

impl std::error::Error for NotSignedIn {}

/// The profiles it's linked to, kept in its folder, and a browser for each.
struct Messenger {
    path: PathBuf,
    /// The profiles, the default first.
    linked: Vec<String>,
    /// Why a profile needs signing in again, as its last read found.
    out: BTreeMap<String, String>,
    browsers: HashMap<String, Arc<Mutex<Browser>>>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    linked: Option<Vec<String>>,
    #[serde(rename = "signed_out", default, skip_serializing_if = "BTreeMap::is_empty")]
    out: BTreeMap<String, String>,
}

impl Messenger {
    /// Which profiles are linked; with none saved yet, the browser profile called default is the first account.
    fn load(host: &Host) -> Result<Messenger> {
        let path = host.dir.join("profiles.json");
        let saved = match std::fs::read(&path) {
            Ok(raw) => serde_json::from_slice(&raw).map_err(|err| anyhow!("{}: {err}", path.display()))?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Saved { linked: Some(vec!["default".into()]), ..Default::default() },
            Err(err) => return Err(err.into()),
        };
        Ok(Messenger { path, linked: saved.linked.unwrap_or_default(), out: saved.out, browsers: HashMap::new() })
    }

    fn save(&self) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let raw = serde_json::to_vec_pretty(&Saved { linked: Some(self.linked.clone()), out: self.out.clone() })?;
        let tmp = self.path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?.write_all(&raw)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// The profile's browser, or the default's when none is named.
    fn browser(&mut self, account: &str) -> Result<Arc<Mutex<Browser>>> {
        let Some(first) = self.linked.first() else { bail!("Messenger isn't linked to a browser profile yet: messenger_sign_in links one") };
        let profile = match account {
            "" => first.clone(),
            account => self.linked.iter().find(|p| p.eq_ignore_ascii_case(account)).cloned().ok_or_else(|| anyhow!("Messenger isn't linked to a profile {account:?}; it's linked to {}", self.linked.join(", ")))?,
        };
        Ok(self.browsers.entry(profile.clone()).or_insert_with(|| Arc::new(Mutex::new(Browser::new(&profile)))).clone())
    }

    /// Keeps whether a read found the profile signed in.
    fn found(&mut self, profile: &str, result: &Result<Value>) {
        let was = self.out.get(profile).cloned();
        match result {
            Err(err) if err.downcast_ref::<NotSignedIn>().is_some() => {
                self.out.insert(profile.into(), "Messenger signed this profile out: sign in again".into());
            }
            Ok(_) => {
                self.out.remove(profile);
            }
            Err(_) => {}
        }
        if was.as_ref() != self.out.get(profile) {
            let _ = self.save();
        }
    }
}

static MESSENGER: Mutex<Option<Messenger>> = Mutex::new(None);

/// Runs `f` on what it keeps, read from its folder the first time, when it starts giving back idle devices too.
fn with<T>(host: &Host, f: impl FnOnce(&mut Messenger) -> Result<T>) -> Result<T> {
    let mut m = MESSENGER.lock().unwrap();
    if m.is_none() {
        *m = Some(Messenger::load(host)?);
        let host = host.clone();
        std::thread::spawn(move || keep_giving_back(&host));
    }
    f(m.as_mut().unwrap())
}

fn keep_giving_back(host: &Host) {
    loop {
        std::thread::sleep(Duration::from_secs(60));
        let all: Vec<_> = MESSENGER.lock().unwrap().as_ref().map(|m| m.browsers.values().cloned().collect()).unwrap_or_default();
        for b in all {
            b.lock().unwrap().give_back(host);
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    account: String,
    profile: String,
    name: String,
    unread: bool,
    chat: String,
    text: String,
    query: String,
}

pub fn call(host: &Host, call: &Call) -> Result<Value> {
    let input: Input = match &call.input {
        Value::Null => Input::default(),
        input => serde_json::from_value(input.clone()).map_err(|err| anyhow!("bad input: {err}"))?,
    };
    match call.op.as_str() {
        "_screen" if input.name == "settings" => return Ok(json!(include_str!("messenger/settings.html"))),
        "_screen" => bail!("Messenger has no screen {:?}", input.name),
        "accounts" => {
            return with(host, |m| {
                Ok(m.linked.iter().map(|p| match m.out.get(p) {
                    Some(why) => json!({ "account": p, "ok": false, "why": why }),
                    None => json!({ "account": p, "ok": true }),
                }).collect())
            });
        }
        "sign_in" => return sign_in(host, if input.profile.is_empty() { input.account.trim() } else { input.profile.trim() }),
        "sign_out" => return sign_out(host, &input.account),
        // The agent devices' profiles, for the settings screen to pick one.
        "_profiles" => return devices(host, "profiles", json!({})),
        // The settings screen's Done: saves the profile signed in.
        "_save" => {
            devices(host, "save_profile", json!({ "profile": input.profile }))?;
            with(host, |m| {
                m.found(&input.profile, &Ok(Value::Null));
                Ok(())
            })?;
            return Ok(json!(format!("Saved the {} profile.", input.profile)));
        }
        _ => {}
    }
    let browser = with(host, |m| m.browser(&input.account))?;
    let mut b = browser.lock().unwrap();
    let result = match call.op.as_str() {
        "chats" => b.chats(host, input.unread),
        "read" => b.read(host, &input.chat),
        "search" => b.search(host, &input.query),
        "send" => b.send(host, &input.chat, &input.text),
        op => Err(anyhow!("no operation {op}")),
    };
    let profile = b.profile.clone();
    drop(b);
    if call.op != "send" || result.is_ok() {
        with(host, |m| {
            m.found(&profile, &result);
            Ok(())
        })?;
    }
    result
}

/// Links Messenger to a browser profile (made if new) and opens messenger.com in it, for the person to sign in.
fn sign_in(host: &Host, profile: &str) -> Result<Value> {
    let profile = match profile {
        "" => with(host, |m| Ok(m.linked.first().cloned().unwrap_or_else(|| "default".into())))?,
        profile => profile.to_string(),
    };
    let ask = || devices(host, "sign_in", json!({ "profile": profile, "url": HOME }));
    let got = match ask() {
        // A new one: added to the Vault first.
        Err(err) if err.to_string().contains("No browser profile") => {
            devices(host, "add_profile", json!({ "profile": profile }))?;
            ask()?
        }
        got => got?,
    };
    let url = got["url"].as_str().unwrap_or_default();
    with(host, |m| {
        if !m.linked.contains(&profile) {
            m.linked.push(profile.clone());
        }
        m.out.remove(&profile);
        m.save()
    })?;
    Ok(json!({
        "profile": profile, "url": url, "device": got["device"], "on": got["on"],
        "text": format!("Give the person this link: {url}\nThey sign in to Messenger there, in the {profile} browser profile (tick Keep me signed in), then press Done on Messenger's settings, or Done Signing In on the profile in the Vault; it's saved by itself after 30 minutes otherwise."),
    }))
}

/// Unlinks a profile: Messenger stops reading it. The profile itself, and its sign-ins to other sites, stay.
fn sign_out(host: &Host, account: &str) -> Result<Value> {
    with(host, |m| {
        let i = m.linked.iter().position(|p| p.eq_ignore_ascii_case(account)).ok_or_else(|| anyhow!("Messenger isn't linked to a profile {account:?}"))?;
        let profile = m.linked.remove(i);
        m.out.remove(&profile);
        if let Some(b) = m.browsers.remove(&profile) {
            let b = b.lock().unwrap();
            if !b.device.is_empty() {
                let _ = devices(host, "give_back", json!({ "device": b.device }));
            }
        }
        m.save()?;
        Ok(json!(format!("Messenger no longer reads the {profile} profile. The profile and its other sign-ins stay, on the Agent Devices page.")))
    })
}

/// Runs one of Agent Devices' operations: those with a browser profile (whose Vault item only the app's calls may use)
/// the app's, the rest its sidecar's, wherever it runs.
fn devices(host: &Host, op: &str, input: Value) -> Result<Value> {
    let mut call = json!({ "app": "agent-devices", "op": op, "input": input });
    if !["borrow", "give_back", "profiles", "sign_in", "save_profile", "add_profile"].contains(&op) {
        call["sidecar"] = json!("agent-devices");
    }
    let reply = host.post_within("/v1/call", &call, Duration::from_secs(180)).map_err(|err| anyhow!("agent devices {op}: {err:#}"))?;
    Ok(reply["ok"].clone())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Ref {
    #[serde(rename = "ref")]
    id: String,
    role: String,
    name: String,
}

#[derive(Default)]
struct Page {
    url: String,
    nodes: Vec<Node>,
    /// The page comes in parts; a row cut between two shows in both.
    parts: Vec<Vec<Node>>,
    refs: Vec<Ref>,
}

impl Page {
    fn find(&self, role: &str, name: &str) -> Option<&str> {
        self.refs.iter().find(|r| r.role == role && r.name == name).map(|r| r.id.as_str())
    }

    /// Whether the page is Messenger's sign-in page.
    fn signed_out(&self) -> bool {
        self.find("button", "Log In").is_some() || self.find("textbox", "Password").is_some() || self.url.contains("/login")
    }

    fn chats(&self) -> Vec<Chat> {
        parse::chats_in(&self.parts)
    }
}

/// The browser in a device borrowed from Agent Devices, through this device's node, starting with a copy of a browser
/// profile signed in to messenger.com.
struct Browser {
    profile: String,
    /// The borrowed device; given back when idle.
    device: String,
    /// The device's tools keep a browser in a session.
    session: String,
    target: String,
    tab: String,
    used: Option<Instant>,
    /// Off the chat list: a conversation, or search results.
    dirty: bool,
}

impl Browser {
    fn new(profile: &str) -> Browser {
        Browser { profile: profile.into(), device: String::new(), session: String::new(), target: String::new(), tab: String::new(), used: None, dirty: false }
    }

    /// Takes a device with the profile, unless one is borrowed: one with no screen, since nobody watches it read.
    fn borrow(&mut self, host: &Host) -> Result<()> {
        if !self.device.is_empty() {
            return Ok(());
        }
        let got = devices(host, "borrow", json!({ "for": "Reading Messenger", "by": "messenger", "profile": self.profile, "screen": false }))?;
        self.device = got["device"].as_str().unwrap_or_default().into();
        (self.target, self.tab) = (String::new(), String::new());
        self.session = format!("messenger-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        if self.device.is_empty() {
            bail!("couldn't borrow a device");
        }
        Ok(())
    }

    /// Gives the device back once it's been idle a while.
    fn give_back(&mut self, host: &Host) {
        if self.device.is_empty() || self.used.is_some_and(|u| u.elapsed() < IDLE) {
            return;
        }
        let _ = devices(host, "give_back", json!({ "device": self.device }));
        self.device.clear();
    }

    /// Runs one of the device's tools and returns its structured answer, borrowing a device again if the last one was
    /// given back.
    fn tool(&mut self, host: &Host, name: &str, input: Value) -> Result<Value> {
        self.used = Some(Instant::now());
        match self.try_tool(host, name, input.clone()) {
            // The device was given back, as after 15 minutes without a call.
            Err(err) if !self.device.is_empty() && format!("{err:#}").contains(&self.device) => {
                self.device.clear();
                if name != "browser_prepare" {
                    self.open(host)?;
                }
                self.try_tool(host, name, input)
            }
            out => out,
        }
    }

    fn try_tool(&mut self, host: &Host, name: &str, mut input: Value) -> Result<Value> {
        self.borrow(host)?;
        input["session"] = json!(self.session);
        let got = if name == "list_windows" {
            devices(host, "use", json!({ "device": self.device, "tool": name, "args": input }))?
        } else {
            input["device"] = json!(self.device);
            if !self.target.is_empty() && name != "browser_prepare" {
                (input["target_id"], input["tab_id"]) = (json!(self.target), json!(self.tab));
            }
            devices(host, name, input)?
        };
        let data = structured(&got);
        if data["status"] == "refused" {
            bail!("{name} refused: {}", data["message"].as_str().unwrap_or_default());
        }
        Ok(data)
    }

    /// Starts the device's browser on the profile and attaches to its tab.
    fn open(&mut self, host: &Host) -> Result<()> {
        (self.target, self.tab) = (String::new(), String::new());
        let prepared = self.tool(host, "browser_prepare", json!({ "allow_launch": true, "profile": { "mode": "isolated_named", "name": self.profile } })).map_err(|err| anyhow!("starting the browser: {err:#}"))?;
        let pid = prepared["prepared_pid"].as_f64().unwrap_or(0.0) as i64;
        let windows = self.tool(host, "list_windows", json!({ "pid": pid }))?;
        let Some(window) = windows["windows"].as_array().and_then(|w| w.first()).cloned() else { bail!("the browser has no window") };
        let bound = self.tool(host, "get_browser_state", json!({ "pid": pid, "window_id": window["window_id"] }))?;
        self.target = bound["target_id"].as_str().unwrap_or_default().into();
        // The active tab, or else the first.
        for t in bound["tabs"].as_array().into_iter().flatten() {
            if t["active"] == true || self.tab.is_empty() {
                self.tab = t["tab_id"].as_str().unwrap_or_default().into();
            }
        }
        if self.target.is_empty() || self.tab.is_empty() {
            bail!("couldn't attach to the browser");
        }
        self.dirty = true;
        Ok(())
    }

    /// Reads the page, or the parts matching `query`; it attaches again once if the browser closed (it does after a few
    /// idle minutes).
    fn snapshot(&mut self, host: &Host, query: &str) -> Result<Page> {
        let mut failed = None;
        for attempt in 0..2 {
            if self.target.is_empty() || attempt > 0 {
                self.open(host)?;
            }
            let mut input = json!({ "snapshot_format": "semantic_v2" });
            if !query.is_empty() {
                input["query"] = json!(query);
            }
            let (mut page, mut outline) = (Page::default(), String::new());
            failed = None;
            for _ in 0..6 {
                let part = match self.tool(host, "get_browser_state", input.clone()) {
                    Ok(part) => part,
                    Err(err) => {
                        failed = Some(err);
                        break;
                    }
                };
                let text = part["outline"].as_str().unwrap_or_default();
                outline.push_str(text);
                outline.push('\n');
                page.parts.push(parse::outline(text));
                page.refs.extend(serde_json::from_value::<Vec<Ref>>(part["refs"].clone()).unwrap_or_default());
                page.url = part["page"]["url"].as_str().or(part["page"]["URL"].as_str()).unwrap_or_default().into();
                match part["snapshot"]["continuation"].as_str().filter(|c| !c.is_empty()) {
                    Some(more) => input["continuation"] = json!(more),
                    None => break,
                }
            }
            if failed.is_none() {
                page.nodes = parse::outline(&outline);
                return Ok(page);
            }
        }
        Err(failed.unwrap_or_else(|| anyhow!("couldn't read the page")))
    }

    /// Reads the chat list, loading it unless it's showing. Anything else is loaded over rather than read: the browser
    /// tools drop a page whose DOM nests too deeply, as some conversations' do, and are slow to say so.
    fn list(&mut self, host: &Host) -> Result<Page> {
        if !self.target.is_empty() && !self.dirty
            && let Ok(p) = self.snapshot(host, "")
            && p.url == HOME
            && !p.chats().is_empty()
        {
            return Ok(p);
        }
        // If the list doesn't show, load it once more.
        for _ in 0..2 {
            self.navigate(host, HOME)?;
            self.dirty = false;
            // A browser that just started can take a while.
            for _ in 0..30 {
                std::thread::sleep(Duration::from_millis(500));
                let p = self.snapshot(host, "")?;
                if p.signed_out() {
                    // Its copy of the profile stays signed out: borrow afresh next time, after the person signed in.
                    let _ = devices(host, "give_back", json!({ "device": self.device }));
                    self.device.clear();
                    return Err(NotSignedIn.into());
                }
                if !p.chats().is_empty() {
                    return Ok(p);
                }
            }
        }
        bail!("Messenger's chat list didn't show")
    }

    /// Loads `url`, attaching again once if the browser closed.
    fn navigate(&mut self, host: &Host, url: &str) -> Result<()> {
        if self.target.is_empty() {
            self.open(host)?;
        }
        if self.tool(host, "browser_navigate", json!({ "url": url })).is_ok() {
            return Ok(());
        }
        self.open(host)?;
        self.tool(host, "browser_navigate", json!({ "url": url }))?;
        Ok(())
    }

    fn click(&mut self, host: &Host, p: &Page, role: &str, name: &str) -> Result<()> {
        let r = p.find(role, name).ok_or_else(|| anyhow!("{role} {name:?} isn't on the page"))?.to_string();
        self.tool(host, "browser_click", json!({ "ref": r, "input_route": "dom_event" }))?;
        Ok(())
    }

    /// Marks a chat unread again from its menu in the chat list.
    fn mark_unread(&mut self, host: &Host, chat: &str) -> Result<()> {
        let menu = format!("{MENU}{chat}");
        let p = self.snapshot(host, &menu)?;
        self.click(host, &p, "button", &menu)?;
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(300));
            let p = self.snapshot(host, "Mark as unread")?;
            if p.find("menuitem", "Mark as unread").is_some() {
                return self.click(host, &p, "menuitem", "Mark as unread");
            }
        }
        bail!("its menu had no Mark as unread")
    }

    fn chats(&mut self, host: &Host, unread: bool) -> Result<Value> {
        let p = self.list(host)?;
        let lines: Vec<String> = p.chats().iter().filter(|c| !unread || c.unread).map(|c| format!("{}{}, {}: {}", c.name, if c.unread { " (unread)" } else { "" }, c.time, c.last)).collect();
        Ok(json!(if lines.is_empty() { "No unread chats.".into() } else { lines.join("\n") }))
    }

    fn read(&mut self, host: &Host, chat: &str) -> Result<Value> {
        if chat.trim().is_empty() {
            bail!("which chat?");
        }
        let p = self.list(host)?;
        let c = pick(&p.chats(), chat)?;
        self.dirty = true;
        self.click(host, &p, "link", &c.link)?;
        let mut msgs = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while msgs.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            msgs = parse::messages(&self.snapshot(host, "Message sent")?.nodes);
        }
        let mut note = String::new();
        if c.unread {
            note = match self.list(host).and_then(|_| self.mark_unread(host, &c.name)) {
                Ok(()) => "\n(Marked unread again.)".into(),
                Err(err) => format!("\n(Couldn't mark it unread again: {err:#})"),
            };
        }
        if msgs.is_empty() {
            bail!("no messages showed in {}{note}", c.name);
        }
        let lines: Vec<String> = msgs.iter().map(|m| format!("[{}] {}: {}", m.time, m.sender, m.text)).collect();
        Ok(json!(lines.join("\n") + &note))
    }

    fn search(&mut self, host: &Host, query: &str) -> Result<Value> {
        if query.trim().is_empty() {
            bail!("what to search for?");
        }
        let p = self.list(host)?;
        // The list shows results until it's loaded again.
        self.dirty = true;
        let box_ = p.find("combobox", "Search Messenger").ok_or_else(|| anyhow!("no search box on the page"))?.to_string();
        self.tool(host, "browser_type", json!({ "ref": box_, "text": query, "replace": true }))?;
        let mut link = None;
        for _ in 0..10 {
            let p = self.snapshot(host, "Search messages for")?;
            link = p.find("link", &format!("Search messages for {query}")).map(str::to_string);
            if link.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        let link = link.ok_or_else(|| anyhow!("Messenger didn't offer to search messages"))?;
        self.tool(host, "browser_click", json!({ "ref": link, "input_route": "dom_event" }))?;
        let mut found = Vec::new();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(500));
            found = parse::matches(&self.snapshot(host, "")?.nodes);
            if !found.is_empty() {
                break;
            }
        }
        if found.is_empty() {
            return Ok(json!("Nothing found."));
        }
        let lines: Vec<String> = found.iter().map(|m| if m.matches > 0 { format!("{}: {} matching messages", m.chat, m.matches) } else { format!("{}, {}: {}", m.chat, m.time, m.text) }).collect();
        Ok(json!(lines.join("\n")))
    }

    fn send(&mut self, host: &Host, chat: &str, text: &str) -> Result<Value> {
        if text.trim().is_empty() || chat.trim().is_empty() {
            bail!("chat and text are required");
        }
        let name = self.open_chat(host, chat)?;
        let mut box_ = None;
        // The conversation loads; on a narrow page its details may cover it.
        for _ in 0..16 {
            let p = self.snapshot(host, "Write to")?;
            box_ = p.refs.iter().rfind(|r| r.role == "textbox" && r.name.starts_with("Write to")).map(|r| r.id.clone());
            if box_.is_some() {
                break;
            }
            if p.find("button", "Back").is_some() {
                let _ = self.click(host, &p, "button", "Back");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let box_ = box_.ok_or_else(|| anyhow!("no message box in the chat with {name}"))?;
        self.tool(host, "browser_type", json!({ "ref": box_, "text": text }))?;
        self.tool(host, "browser_type", json!({ "ref": box_, "text": "\n", "mode": "keystrokes" }))?;
        // Confirm it from the chat list, which is quick to read where the conversation may not be.
        let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let want = format!("You: {}", words(text));
        std::thread::sleep(Duration::from_secs(1));
        for _ in 0..10 {
            let p = self.list(host)?;
            if let Ok(c) = pick(&p.chats(), &name)
                && words(&c.last) == want
            {
                return Ok(json!(format!("Sent to {}.", c.name)));
            }
            std::thread::sleep(Duration::from_millis(500));
            self.dirty = true;
        }
        bail!("typed the message to {name}, but it didn't show as sent: look at the Agent Devices page")
    }

    /// Opens a recent chat, or else the one person Messenger suggests whose name is exactly `want`, and says its name.
    /// Anything less certain is refused, so a message never goes to the wrong person.
    fn open_chat(&mut self, host: &Host, want: &str) -> Result<String> {
        let p = self.list(host)?;
        self.dirty = true;
        match pick(&p.chats(), want) {
            Ok(c) => {
                self.click(host, &p, "link", &c.link)?;
                return Ok(c.name);
            }
            Err(err) if err.downcast_ref::<NoChat>().is_none() => return Err(err),
            Err(_) => {}
        }
        let box_ = p.find("combobox", "Search Messenger").ok_or_else(|| anyhow!("no search box on the page"))?.to_string();
        self.tool(host, "browser_type", json!({ "ref": box_, "text": want, "replace": true }))?;
        let mut found = Vec::new();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(500));
            let p = self.snapshot(host, want)?;
            let people: Vec<&Ref> = p.refs.iter().filter(|r| r.role == "link" && !r.name.starts_with("Search messages for")).collect();
            found = people.iter().map(|r| r.name.clone()).collect();
            let exact: Vec<&&Ref> = people.iter().filter(|r| r.name.to_lowercase() == want.trim().to_lowercase()).collect();
            match exact.as_slice() {
                [one] => {
                    self.tool(host, "browser_click", json!({ "ref": one.id, "input_route": "dom_event" }))?;
                    return Ok(want.trim().to_string());
                }
                [] => {}
                _ => bail!("several people are called {want:?}: send from the agent device yourself"),
            }
        }
        if found.is_empty() {
            bail!("no chat or person called {want:?}");
        }
        bail!("no chat or person called exactly {want:?}; Messenger suggests: {}", found.join("; "))
    }
}

/// A tool's structured answer: Agent Devices gives it as its content's last text.
fn structured(got: &Value) -> Value {
    got["content"].as_array().into_iter().flatten().rev().find_map(|item| serde_json::from_str::<Value>(item["text"].as_str()?).ok().filter(Value::is_object)).unwrap_or_else(|| json!({}))
}
