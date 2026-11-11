//! Chrome devices: the machine's own Google Chrome, headless, one for each borrow, with a folder of its own that's
//! deleted when it's given back. No container: it starts in a second and reaches what this machine reaches, so a
//! machine runs them only while its owner allows it (the switch below). This service drives it over its DevTools pipe
//! (--remote-debugging-pipe: no port, nothing else can reach it) and answers the browser tools itself (browse.rs).
//! Chrome's own sandbox stays on.
//!
//! Whether a machine runs chrome devices is its owner's to say. Until they say, it does where the machine runs only its
//! owner's own calls, and doesn't where it runs a group's, global's or another user's too (Where Things Run): their
//! data is on it. What the owner chose is kept in chrome.json in this service's folder, and holds whatever the machine
//! comes to run.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use one_service::refuse;
use serde_json::{Value, json};

use super::kinds::{Action, Kind, Vm};
use super::pool::Loan;
use super::profiles::Profile;
use super::{Caller, Devices, Input, or, text};

pub const NAME: &str = "chrome";

/// The memory a chrome device is taken to need, in GB.
pub const GB: u64 = 1;

pub struct Chrome;

fn chrome_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        return [PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"), home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome")].into_iter().find(|p| p.exists());
    }
    let path = std::env::var_os("PATH")?;
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].iter().find_map(|name| std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file()))
}

pub fn target_of(v: &Vm) -> String {
    format!("bt-{}", v.name)
}

pub fn tab_id(target: &str) -> String {
    format!("tab-{}", target.to_lowercase())
}

impl Kind for Chrome {
    fn name(&self) -> &'static str {
        NAME
    }

    fn title(&self) -> &'static str {
        "Chrome"
    }

    fn limits(&self) -> &'static str {
        "the browser tools (computer_browser_*) read and act on its pages; clicks, typing, keys and scrolling in the page; open_url. It's only a browser: no install_app or run_command"
    }

    fn input(&self) -> bool {
        true
    }

    fn port(&self) -> (u16, u16) {
        (0, 0)
    }

    fn can(&self) -> String {
        if chrome_path().is_none() { "Google Chrome isn't installed here".into() } else { String::new() }
    }

    fn adopt(&self, _: &Devices, _: &mut Vm) -> Result<()> {
        refuse!("chrome devices are only borrowed: there's nothing to adopt")
    }

    fn make(&self, _: &Devices, v: &mut Vm, _: Option<&Vm>) -> Result<()> {
        let dir = std::env::temp_dir().join(format!("agent-devices-{}-{}", v.name, super::random_hex(6)));
        std::fs::create_dir_all(&dir)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        (v.dir, v.gb) = (dir.to_string_lossy().to_string(), GB as u16);
        Ok(())
    }

    fn start(&self, _: &Devices, v: &Vm) -> Result<()> {
        launch(v).map(|_| ())
    }

    fn stop(&self, _: &Devices, v: &Vm) -> Result<()> {
        if let Some(b) = browser_of(&v.name) {
            b.close();
        }
        Ok(())
    }

    /// Chrome can't pause: a borrowed one runs until it's given back.
    fn pause(&self, _: &Devices, _: &Vm) -> Result<()> {
        Ok(())
    }

    fn resume(&self, _: &Devices, _: &Vm) -> Result<()> {
        Ok(())
    }

    fn running(&self, _: &Devices, v: &Vm) -> bool {
        browser_of(&v.name).is_some_and(|b| b.alive())
    }

    fn remove(&self, ds: &Devices, v: &Vm) -> Result<()> {
        self.stop(ds, v)?;
        if !v.dir.is_empty() && Path::new(&v.dir).file_name().is_some_and(|n| n.to_string_lossy().contains("agent-devices-")) {
            // A Chrome this program started before it restarted.
            let _ = super::linux::cmd("pkill").args(["-f", "--", &format!("--user-data-dir={}", v.dir)]).status();
            match std::fs::remove_dir_all(&v.dir) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn screen(&self, _: &Devices, v: &Vm) -> Result<image::DynamicImage> {
        let b = live_browser(v)?;
        let p = b.current_page()?;
        let shot = b.call(&p, "Page.captureScreenshot", Some(json!({ "format": "jpeg", "quality": 80 })))?;
        let data = data_encoding::BASE64.decode(shot["data"].as_str().unwrap_or_default().as_bytes())?;
        super::picture::decode(&data)
    }

    /// The viewer's and computer_use's input, in the page's CSS pixels.
    fn act(&self, _: &Devices, v: &Vm, a: &Action) -> Result<()> {
        let b = live_browser(v)?;
        let p = b.current_page()?;
        match a.r#do.as_str() {
            "click" | "double_click" | "right_click" => {
                let button = if a.r#do == "right_click" { "right" } else { "left" };
                b.click(&p, a.x, a.y, button, if a.r#do == "double_click" { 2 } else { 1 })
            }
            "type" => b.input(&p, "Input.insertText", json!({ "text": a.text })).map(|_| ()),
            "key" => b.key(&p, &a.key).map(|_| ()),
            "scroll" => b.input(&p, "Input.dispatchMouseEvent", json!({ "type": "mouseWheel", "x": a.x, "y": a.y, "deltaX": a.x - a.to_x, "deltaY": a.y - a.to_y })).map(|_| ()),
            "swipe" => b.drag(&p, a.x, a.y, a.to_x, a.to_y),
            other => refuse!("no action {other:?}"),
        }
    }

    fn open(&self, _: &Devices, v: &Vm, url: &str) -> Result<()> {
        let b = live_browser(v)?;
        let p = b.current_page()?;
        b.navigate(&p, url)
    }

    fn install(&self, _: &Devices, _: &Vm, _: &str) -> Result<String> {
        refuse!("a chrome device is only a browser: nothing installs on it; borrow a linux one for that")
    }

    fn run(&self, _: &Devices, _: &Vm, _: &str) -> Result<String> {
        refuse!("a chrome device is only a browser: it runs no commands; borrow a linux one for that")
    }

    fn reach(&self, _: &Devices, _: &Vm, _: u16) -> Result<String> {
        refuse!("a chrome device already reaches what this machine reaches")
    }
}

/// A chrome device's Chrome, and this program's DevTools connection to it: the browser's own, with a session for each
/// page (flat sessions, Target.setAutoAttach).
pub struct Browser {
    name: String,
    pid: i32,
    /// To Chrome.
    w: Mutex<Option<std::fs::File>>,
    next: AtomicI64,
    pub now: Mutex<State>,
    changed: Condvar,
}

/// What a browser knows, changed as Chrome's messages come.
#[derive(Default)]
pub struct State {
    /// Chrome is gone.
    done: bool,
    /// Answers by id, once they come, for whoever waits for them.
    replies: HashMap<i64, std::result::Result<Value, String>>,
    /// Pages by session.
    pub pages: HashMap<String, Page>,
    /// The page the user sees and computer_use acts on, by target.
    current: String,
    /// Chrome's user agent, without "Headless".
    ua: String,
    /// Snapshots taken, for their ids.
    pub snaps: u64,
    /// Dialogs seen, for their ids.
    dialogs: u64,
}

/// A tab, as one session of the connection sees it.
#[derive(Clone, Default)]
pub struct Page {
    pub target: String,
    pub session: String,
    /// Backend DOM app runtime ids by ref, since its last navigation.
    pub refs: HashMap<String, i64>,
    /// Its last snapshot's lines, for a continuation.
    pub outline: Vec<String>,
    pub snapshot: String,
    pub dialog: Option<Dialog>,
    /// How many dialogs have opened in it, and how many times it loaded.
    opened: u64,
    loaded: u64,
}

#[derive(Clone, Debug)]
pub struct Dialog {
    pub id: String,
    pub kind: String,
    pub message: String,
    pub default: String,
}

/// Input that opened a dialog, which holds the page until it's answered.
#[derive(Debug)]
pub struct DialogOpened;

impl std::fmt::Display for DialogOpened {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a dialog opened")
    }
}

impl std::error::Error for DialogOpened {}

static BROWSERS: Mutex<Option<HashMap<String, Arc<Browser>>>> = Mutex::new(None);

fn browser_of(name: &str) -> Option<Arc<Browser>> {
    BROWSERS.lock().unwrap().get_or_insert_default().get(name).cloned()
}

pub fn live_browser(v: &Vm) -> Result<Arc<Browser>> {
    browser_of(&v.name).filter(|b| b.alive()).ok_or_else(|| anyhow!("{}'s Chrome isn't running: borrow another", v.name))
}

/// What Chrome starts with, as the Linux devices' Chrome does where it makes sense: no first-run screens, sign-ins kept
/// in the profile's own store (basic on Linux, as a profile made in a Linux device was; on a Mac, never the user's
/// keychain), and headless at the size of a Linux device's screen.
fn chrome_args(dir: &str) -> Vec<String> {
    let mut args: Vec<String> = ["--headless=new", "--remote-debugging-pipe", &format!("--user-data-dir={dir}"), "--window-size=1280,800", "--no-first-run", "--no-default-browser-check", "--disable-search-engine-choice-screen", "--hide-crash-restore-bubble"]
        .map(String::from)
        .to_vec();
    args.push(if cfg!(target_os = "macos") { "--use-mock-keychain" } else { "--password-store=basic" }.into());
    args.push("about:blank".into());
    args
}

/// A pipe's two ends, each at fd 10 or above, so they never land where the child's are put (3 and 4).
fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: pipe fills both fds, which are this process's from then on; each is moved, close-on-exec, at once.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let high = |fd: i32| -> Result<OwnedFd> {
        // SAFETY: fd is open and this process's; the copy is too, and the original is closed once copied.
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        unsafe { libc::close(fd) };
        if copy < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(copy) })
    };
    Ok((high(fds[0])?, high(fds[1])?))
}

/// Starts a chrome device's Chrome, its DevTools on fds 3 (to it) and 4 (from it).
fn launch(v: &Vm) -> Result<Arc<Browser>> {
    let path = chrome_path().ok_or_else(|| anyhow!("{}", Chrome.can()))?;
    if let Some(b) = browser_of(&v.name) {
        b.close();
    }
    let (to_r, to_w) = pipe()?;
    let (from_r, from_w) = pipe()?;
    let mut c = std::process::Command::new(&path);
    c.args(chrome_args(&v.dir)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        let (r, w) = (to_r.as_raw_fd(), from_w.as_raw_fd());
        // SAFETY: only async-signal-safe calls, between fork and exec.
        unsafe {
            c.pre_exec(move || {
                if libc::dup2(r, 3) < 0 || libc::dup2(w, 4) < 0 || libc::setpgid(0, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = c.spawn()?;
    drop((to_r, from_w));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    if let Some(mut err) = child.stderr.take() {
        let stderr = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut s = stderr.lock().unwrap();
                let room = (8usize << 10).saturating_sub(s.len());
                s.extend_from_slice(&buf[..n.min(room)]);
            }
        });
    }
    let b = Arc::new(Browser { name: v.name.clone(), pid: child.id() as i32, w: Mutex::new(Some(std::fs::File::from(to_w))), next: AtomicI64::new(0), now: Mutex::new(State::default()), changed: Condvar::new() });
    let reader = b.clone();
    std::thread::spawn(move || {
        reader.read(std::fs::File::from(from_r));
        let _ = child.wait();
        reader.now.lock().unwrap().done = true;
        reader.changed.notify_all();
    });
    BROWSERS.lock().unwrap().get_or_insert_default().insert(v.name.clone(), b.clone());
    let started = Instant::now();
    let fail = |err: anyhow::Error| -> Result<Arc<Browser>> {
        b.close();
        Err(err)
    };
    let ver = match b.call_within("", "Browser.getVersion", None, Duration::from_secs(30)) {
        Ok(ver) => ver,
        Err(err) => return fail(anyhow!("Chrome didn't start: {err:#}: {}", String::from_utf8_lossy(&stderr.lock().unwrap()).trim())),
    };
    b.now.lock().unwrap().ua = ver["userAgent"].as_str().unwrap_or_default().replacen("HeadlessChrome", "Chrome", 1);
    if let Err(err) = b.call_within("", "Target.setAutoAttach", Some(json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true })), Duration::from_secs(30)) {
        return fail(err);
    }
    if let Err(err) = b.current_page() {
        return fail(err);
    }
    tracing::info!("{}: Chrome started in {}ms", v.name, started.elapsed().as_millis());
    Ok(b)
}

/// How long a DevTools command may take, unless it says.
const CALL: Duration = Duration::from_secs(90);

impl Browser {
    pub fn alive(&self) -> bool {
        !self.now.lock().unwrap().done
    }

    pub fn close(&self) {
        if self.alive() {
            // SAFETY: a signal to Chrome's own process group, which it leads.
            unsafe { libc::kill(-self.pid, libc::SIGKILL) };
        }
        self.w.lock().unwrap().take();
        let now = self.now.lock().unwrap();
        let _ = self.changed.wait_timeout_while(now, Duration::from_secs(5), |s| !s.done);
        let mut all = BROWSERS.lock().unwrap();
        let all = all.get_or_insert_default();
        if all.get(&self.name).is_some_and(|b| std::ptr::eq(b.as_ref(), self)) {
            all.remove(&self.name);
        }
    }

    /// Takes Chrome's messages, each JSON ended by a NUL: answers go to whoever waits for them, events are handled.
    fn read(self: &Arc<Self>, from: std::fs::File) {
        let mut r = BufReader::with_capacity(1 << 20, from);
        loop {
            let mut msg = Vec::new();
            match r.read_until(0, &mut msg) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            msg.pop();
            let Ok(m) = serde_json::from_slice::<Value>(&msg) else { continue };
            if let Some(id) = m["id"].as_i64().filter(|id| *id != 0) {
                let reply = match m.get("error").filter(|e| !e.is_null()) {
                    Some(e) => Err(e["message"].as_str().unwrap_or("failed").to_string()),
                    None => Ok(m["result"].clone()),
                };
                self.now.lock().unwrap().replies.insert(id, reply);
                self.changed.notify_all();
                continue;
            }
            self.event(m["method"].as_str().unwrap_or_default(), m["sessionId"].as_str().unwrap_or_default(), &m["params"]);
            self.changed.notify_all();
        }
    }

    fn event(self: &Arc<Self>, method: &str, session: &str, params: &Value) {
        let mut s = self.now.lock().unwrap();
        match method {
            "Target.attachedToTarget" => {
                let session = params["sessionId"].as_str().unwrap_or_default().to_string();
                let target = params["targetInfo"]["targetId"].as_str().unwrap_or_default().to_string();
                let is_page = params["targetInfo"]["type"] == "page";
                if is_page {
                    s.pages.insert(session.clone(), Page { target: target.clone(), session: session.clone(), ..Default::default() });
                    if s.current.is_empty() {
                        s.current = target;
                    }
                }
                let me = self.clone();
                std::thread::spawn(move || me.ready(&session, is_page));
            }
            "Target.detachedFromTarget" => {
                let session = params["sessionId"].as_str().unwrap_or_default();
                if let Some(p) = s.pages.remove(session)
                    && p.target == s.current
                {
                    s.current.clear();
                }
            }
            "Page.javascriptDialogOpening" => {
                s.dialogs += 1;
                let id = format!("dialog-{}", s.dialogs);
                if let Some(p) = s.pages.get_mut(session) {
                    p.dialog = Some(Dialog {
                        id,
                        kind: params["type"].as_str().unwrap_or_default().into(),
                        message: params["message"].as_str().unwrap_or_default().into(),
                        default: params["defaultPrompt"].as_str().unwrap_or_default().into(),
                    });
                    p.opened += 1;
                }
            }
            "Page.javascriptDialogClosed" => {
                if let Some(p) = s.pages.get_mut(session) {
                    p.dialog = None;
                }
            }
            "Page.loadEventFired" => {
                if let Some(p) = s.pages.get_mut(session) {
                    p.loaded += 1;
                }
            }
            "Page.frameNavigated" => {
                if params["frame"]["parentId"].as_str().unwrap_or_default().is_empty()
                    && let Some(p) = s.pages.get_mut(session)
                {
                    (p.refs, p.outline) = (HashMap::new(), Vec::new());
                }
            }
            _ => {}
        }
    }

    /// Sets up a target Chrome attached, which waits for it: a page's events, and its user agent without "Headless",
    /// which sites refuse.
    fn ready(&self, session: &str, is_page: bool) {
        let limit = Duration::from_secs(10);
        if is_page {
            let _ = self.call_within(session, "Page.enable", None, limit);
            let ua = self.now.lock().unwrap().ua.clone();
            if !ua.is_empty() {
                let _ = self.call_within(session, "Network.setUserAgentOverride", Some(json!({ "userAgent": ua })), limit);
            }
        }
        let _ = self.call_within(session, "Runtime.runIfWaitingForDebugger", None, limit);
    }

    /// Sends a DevTools command, to a page's session or ("") the browser: its id.
    fn send(&self, session: &str, method: &str, params: Option<Value>) -> Result<i64> {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut msg = json!({ "id": id, "method": method });
        if let Some(p) = params {
            msg["params"] = p;
        }
        if !session.is_empty() {
            msg["sessionId"] = json!(session);
        }
        let mut bytes = msg.to_string().into_bytes();
        bytes.push(0);
        let mut w = self.w.lock().unwrap();
        let w = w.as_mut().ok_or_else(|| anyhow!("Chrome closed"))?;
        w.write_all(&bytes).map_err(|err| anyhow!("Chrome closed: {err}"))?;
        Ok(id)
    }

    /// Waits for what `done` finds in the state, up to `limit`: none if it ran out.
    fn wait<T>(&self, limit: Duration, mut done: impl FnMut(&mut MutexGuard<State>) -> Option<T>) -> Option<T> {
        let until = Instant::now() + limit;
        let mut s = self.now.lock().unwrap();
        loop {
            if let Some(v) = done(&mut s) {
                return Some(v);
            }
            let left = until.checked_duration_since(Instant::now())?;
            s = self.changed.wait_timeout(s, left).unwrap().0;
        }
    }

    fn answer(&self, id: i64, method: &str, limit: Duration) -> Result<Value> {
        let got = self.wait(limit, |s| s.replies.remove(&id).or_else(|| s.done.then(|| Err("Chrome closed".to_string()))));
        match got {
            Some(Ok(v)) => Ok(v),
            Some(Err(err)) => bail!("{err}"),
            None => {
                self.now.lock().unwrap().replies.remove(&id);
                bail!("{method}: it didn't answer in {}s", limit.as_secs())
            }
        }
    }

    pub fn call_within(&self, session: &str, method: &str, params: Option<Value>, limit: Duration) -> Result<Value> {
        let id = self.send(session, method, params)?;
        self.answer(id, method, limit)
    }

    pub fn call(&self, session: &str, method: &str, params: Option<Value>) -> Result<Value> {
        self.call_within(session, method, params, CALL)
    }

    pub fn page(&self, session: &str) -> Option<Page> {
        self.now.lock().unwrap().pages.get(session).cloned()
    }

    /// Sends input to a page and waits for it to be taken, or for a dialog it opened: a dialog holds the input until
    /// it's answered (browser_dialog).
    pub fn input(&self, session: &str, method: &str, params: Value) -> Result<Value> {
        let opened = self.page(session).map(|p| p.opened).unwrap_or(0);
        let id = self.send(session, method, Some(params))?;
        let got = self.wait(Duration::from_secs(20), |s| {
            if let Some(r) = s.replies.remove(&id) {
                return Some(r.map_err(|e| anyhow!("{e}")));
            }
            if s.done {
                return Some(Err(anyhow!("Chrome closed")));
            }
            s.pages.get(session).filter(|p| p.opened > opened).map(|_| Err(DialogOpened.into()))
        });
        got.unwrap_or_else(|| bail!("{method}: the page didn't take it in 20 seconds"))
    }

    pub fn click(&self, session: &str, x: f64, y: f64, button: &str, count: u32) -> Result<()> {
        self.input(session, "Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y }))?;
        for n in 1..=count {
            for kind in ["mousePressed", "mouseReleased"] {
                self.input(session, "Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": n }))?;
            }
        }
        Ok(())
    }

    pub fn drag(&self, session: &str, x: f64, y: f64, to_x: f64, to_y: f64) -> Result<()> {
        let mut steps = vec![json!({ "type": "mouseMoved", "x": x, "y": y }), json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 })];
        for i in 1..=10 {
            let f = i as f64 / 10.0;
            steps.push(json!({ "type": "mouseMoved", "x": x + (to_x - x) * f, "y": y + (to_y - y) * f, "button": "left", "buttons": 1 }));
        }
        steps.push(json!({ "type": "mouseReleased", "x": to_x, "y": to_y, "button": "left", "clickCount": 1 }));
        for s in steps {
            self.input(session, "Input.dispatchMouseEvent", s)?;
        }
        Ok(())
    }

    /// Presses a key, or a combination like ctrl-a.
    pub fn key(&self, session: &str, combo: &str) -> Result<Value> {
        let lower = combo.to_lowercase();
        let parts: Vec<&str> = lower.split(['-', '+']).filter(|p| !p.is_empty()).collect();
        let Some((name, mods_named)) = parts.split_last() else { refuse!("press which key?") };
        let mut mods = 0;
        for m in mods_named {
            mods |= match *m {
                "alt" | "option" => 1,
                "ctrl" | "control" => 2,
                "cmd" | "meta" | "win" | "super" => 4,
                "shift" => 8,
                m => refuse!("no modifier {m:?}: there's ctrl, alt, shift and cmd"),
            };
        }
        let (key, code, vk, text): (String, String, i64, String) = match *name {
            "enter" | "return" => ("Enter".into(), "Enter".into(), 13, "\r".into()),
            "tab" => ("Tab".into(), "Tab".into(), 9, String::new()),
            "escape" | "esc" => ("Escape".into(), "Escape".into(), 27, String::new()),
            "backspace" => ("Backspace".into(), "Backspace".into(), 8, String::new()),
            "delete" => ("Delete".into(), "Delete".into(), 46, String::new()),
            "space" => (" ".into(), "Space".into(), 32, " ".into()),
            "up" => ("ArrowUp".into(), "ArrowUp".into(), 38, String::new()),
            "down" => ("ArrowDown".into(), "ArrowDown".into(), 40, String::new()),
            "left" => ("ArrowLeft".into(), "ArrowLeft".into(), 37, String::new()),
            "right" => ("ArrowRight".into(), "ArrowRight".into(), 39, String::new()),
            "home" => ("Home".into(), "Home".into(), 36, String::new()),
            "end" => ("End".into(), "End".into(), 35, String::new()),
            "pageup" => ("PageUp".into(), "PageUp".into(), 33, String::new()),
            "pagedown" => ("PageDown".into(), "PageDown".into(), 34, String::new()),
            name => {
                let mut chars = name.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else { refuse!("no key {name:?}") };
                if c.is_ascii_lowercase() {
                    let up = c.to_ascii_uppercase();
                    (name.into(), format!("Key{up}"), up as i64, name.into())
                } else if c.is_ascii_digit() {
                    (name.into(), format!("Digit{name}"), c as i64, name.into())
                } else {
                    (name.into(), String::new(), c as i64, name.into())
                }
            }
        };
        let mut down = json!({ "type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk, "modifiers": mods });
        if !text.is_empty() && mods & (2 | 4) == 0 {
            down["text"] = json!(text);
        } else {
            down["type"] = json!("rawKeyDown");
        }
        if mods & (2 | 4) != 0 && *name == "a" {
            down["commands"] = json!(["selectAll"]);
        }
        self.input(session, "Input.dispatchKeyEvent", down)?;
        self.input(session, "Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "modifiers": mods }))
    }

    /// Opens url in a page and waits for it to load, up to 20 seconds.
    pub fn navigate(&self, session: &str, url: &str) -> Result<()> {
        let before = self.page(session).map(|p| (p.loaded, p.opened)).unwrap_or_default();
        let nav = match self.input(session, "Page.navigate", json!({ "url": url })) {
            Err(err) if err.downcast_ref::<DialogOpened>().is_some() => return Ok(()),
            r => r?,
        };
        let error = nav["errorText"].as_str().unwrap_or_default();
        if !error.is_empty() {
            bail!("couldn't open {url}: {error}");
        }
        if nav["loaderId"].as_str().unwrap_or_default().is_empty() {
            return Ok(()); // the same document, at another #fragment
        }
        self.wait(Duration::from_secs(20), |s| s.pages.get(session).filter(|p| p.loaded > before.0 || p.opened > before.1).map(|_| ()));
        Ok(())
    }

    /// The browser's pages as the browser tools list them, the current one active.
    pub fn tabs(&self) -> Result<Vec<Value>> {
        let got = self.call("", "Target.getTargets", None)?;
        let current = self.now.lock().unwrap().current.clone();
        Ok(got["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["type"] == "page")
            .map(|t| {
                let target = t["targetId"].as_str().unwrap_or_default();
                json!({ "active": target == current, "tab_id": tab_id(target), "title": t["title"], "url": t["url"] })
            })
            .collect())
    }

    /// The page with a tab id, which becomes the current one: its session.
    pub fn page_of(&self, id: &str) -> Result<String> {
        if id.is_empty() {
            return self.current_page();
        }
        let mut s = self.now.lock().unwrap();
        let Some((session, target)) = s.pages.values().find(|p| tab_id(&p.target) == id.to_lowercase()).map(|p| (p.session.clone(), p.target.clone())) else {
            refuse!("no tab {id}: it closed, or is another device's; get_browser_state lists the tabs")
        };
        s.current = target;
        Ok(session)
    }

    /// The page the user sees, a new one if every tab closed: its session. Chrome attaches each page as it opens.
    pub fn current_page(&self) -> Result<String> {
        for _ in 0..50 {
            {
                let mut s = self.now.lock().unwrap();
                if let Some(p) = s.pages.values().find(|p| p.target == s.current) {
                    return Ok(p.session.clone());
                }
                if let Some((session, target)) = s.pages.values().min_by(|a, b| a.target.cmp(&b.target)).map(|p| (p.session.clone(), p.target.clone())) {
                    s.current = target;
                    return Ok(session);
                }
                if s.done {
                    bail!("Chrome closed");
                }
            }
            // None attached yet: a new tab if none is open.
            if self.tabs().is_ok_and(|t| t.is_empty()) {
                let _ = self.call("", "Target.createTarget", Some(json!({ "url": "about:blank" })));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("Chrome has no tab to use")
    }

    /// Hands Chrome the cookies `take_cookies` opened: all at once, or, if one of them is refused, each alone.
    fn give_cookies(&self, cookies: &[Value]) {
        if cookies.is_empty() || self.call("", "Storage.setCookies", Some(json!({ "cookies": cookies }))).is_ok() {
            return;
        }
        for c in cookies {
            let _ = self.call("", "Storage.setCookies", Some(json!({ "cookies": [c] })));
        }
    }
}

/// The files and folders of a gzipped tarball (GNU's or POSIX's, as tar in a Linux device writes it), each put in dir:
/// nothing outside it, and no links.
fn untar(gz: &[u8], dir: &Path) -> Result<()> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut tar)?;
    let field = |h: &[u8], at: usize, len: usize| -> String { String::from_utf8_lossy(&h[at..at + len]).trim_end_matches('\0').to_string() };
    let octal = |h: &[u8], at: usize, len: usize| -> u64 {
        let f = &h[at..at + len];
        if f[0] & 0x80 != 0 {
            return f[1..].iter().fold(0u64, |n, b| n << 8 | *b as u64); // base-256, for a large file
        }
        u64::from_str_radix(String::from_utf8_lossy(f).trim_matches(|c: char| c == '\0' || c == ' '), 8).unwrap_or(0)
    };
    let (mut at, mut long_name) = (0usize, None::<String>);
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let size = octal(h, 124, 12) as usize;
        let kind = h[156];
        let body = tar.get(at + 512..at + 512 + size).ok_or_else(|| anyhow!("the profile's tarball is cut short"))?;
        at += 512 + size.div_ceil(512) * 512;
        match kind {
            // GNU's long name, for the next entry.
            b'L' => {
                long_name = Some(String::from_utf8_lossy(body).trim_end_matches('\0').to_string());
                continue;
            }
            // POSIX's extended header: its path, for the next entry.
            b'x' => {
                let text = String::from_utf8_lossy(body).to_string();
                long_name = text.lines().find_map(|l| l.split_once(' ').and_then(|(_, kv)| kv.strip_prefix("path=")).map(str::to_string));
                continue;
            }
            _ => {}
        }
        let name = long_name.take().unwrap_or_else(|| {
            let (prefix, name) = (field(h, 345, 155), field(h, 0, 100));
            // POSIX's prefix field; GNU's has times there instead.
            if prefix.is_empty() || &h[257..263] != b"ustar\0" { name } else { format!("{prefix}/{name}") }
        });
        let rel: PathBuf = Path::new(&name).components().filter(|c| matches!(c, std::path::Component::Normal(_))).collect();
        if rel.as_os_str().is_empty() || Path::new(&name).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            continue;
        }
        let to = dir.join(rel);
        match kind {
            b'5' => std::fs::create_dir_all(&to)?,
            b'0' | 0 => {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&to, body)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether a machine runs chrome devices: as its owner chose, or by default only where it runs no one else's calls.
fn allowed(chosen: Option<bool>, shared: bool) -> bool {
    chosen.unwrap_or(!shared)
}

/// Whether a machine whose owner is owner runs calls of scopes other than theirs, as runs_for lists them (global,
/// group:<id>, user:<id>).
fn runs_shared(owner: &str, runs_for: &[String]) -> bool {
    runs_for.iter().any(|s| *s != format!("user:{owner}"))
}

/// Whether caller (a user's id) may switch chrome devices on or off on a machine owned by owner.
fn may_choose(caller: &str, owner: &str, machine: &str) -> Result<()> {
    if owner.is_empty() {
        bail!("whose {machine} is isn't known here yet: try again in a moment");
    }
    if caller != owner {
        refuse!("only {machine}'s owner may allow Chrome on it");
    }
    Ok(())
}

/// One of the network's devices, as its app runtime's /api/state says.
#[derive(Clone, Default)]
struct Machine {
    name: String,
    user: String,
    /// The scopes whose calls it runs; none where its row doesn't say.
    runs_for: Option<Vec<String>>,
}

impl Machine {
    fn of(d: &Value) -> Machine {
        Machine {
            name: d["name"].as_str().unwrap_or_default().into(),
            user: d["user"].as_str().unwrap_or_default().into(),
            runs_for: d["runs_for"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()),
        }
    }
}

fn machines_in(st: &Value) -> (Machine, Vec<Machine>) {
    let all: Vec<&Value> = st["network"]["devices"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let mut here = all.iter().find(|d| d["here"] == true).map(|d| Machine::of(d)).unwrap_or_default();
    here.name = or(st["me"]["name"].as_str().unwrap_or_default(), &or(st["device"]["name"].as_str().unwrap_or_default(), &here.name));
    here.user = or(&here.user, st["user"]["id"].as_str().unwrap_or_default());
    // A service's rows don't say what their devices run: its own device's row does.
    if here.runs_for.is_none() {
        here.runs_for = Machine::of(&st["device"]).runs_for;
    }
    (here, all.iter().map(|d| Machine::of(d)).collect())
}

impl Devices {
    /// What this machine's owner chose, if they did.
    fn chrome_choice(&self) -> Option<bool> {
        let b = std::fs::read(self.dir.join("chrome.json")).ok()?;
        serde_json::from_slice::<Value>(&b).ok()?["allowed"].as_bool()
    }

    fn machines(&self) -> Result<(Machine, Vec<Machine>)> {
        Ok(machines_in(&self.host.get("/api/state")?))
    }

    /// This machine's row of chrome: whether it runs chrome devices, and why.
    fn chrome_row(&self, me: &Machine) -> Value {
        let chosen = self.chrome_choice();
        let shared = runs_shared(&me.user, me.runs_for.as_deref().unwrap_or_default());
        json!({ "on": me.name, "allowed": allowed(chosen, shared), "shared": shared, "chosen": chosen.is_some(), "can": Chrome.can() })
    }

    /// Why this machine runs no chrome devices now, or "".
    pub(super) fn chrome_why_not(&self) -> String {
        let why = Chrome.can();
        if !why.is_empty() {
            return why;
        }
        let row = match self.machines() {
            Ok((me, _)) => self.chrome_row(&me),
            Err(err) => return format!("whether its owner allows Chrome here isn't known: {err:#}"),
        };
        if row["allowed"] == true {
            return String::new();
        }
        if row["chosen"] == true {
            return format!("{}'s owner switched Chrome off here", row["on"].as_str().unwrap_or_default());
        }
        format!("Chrome is off on {}, which runs others' calls too, until its owner allows it", row["on"].as_str().unwrap_or_default())
    }

    pub(super) fn run_chrome(&self, caller: &Caller, function: &str, here: bool, input: &Input) -> Result<Value> {
        let (me, all) = self.machines()?;
        let mine = self.chrome_row(&me);
        if function == "chrome" {
            if here {
                return Ok(mine);
            }
            let mut rows = vec![mine];
            let (got, failed) = self.everywhere("chrome", None);
            let mut names: Vec<&String> = got.keys().collect();
            names.sort();
            rows.extend(names.into_iter().map(|s| got[s].clone()).filter(|row| !row["on"].is_null()));
            let mut out = json!({ "devices": rows });
            if !failed.is_empty() {
                out["unreachable"] = json!(failed);
            }
            return Ok(out);
        }
        let mut on = or(&text(input, "on"), &me.name);
        let Some(allowed) = input.get("allowed").and_then(Value::as_bool) else { refuse!("allowed: true or false") };
        let mut owner = String::new();
        for d in &all {
            if d.name.eq_ignore_ascii_case(&on) {
                (on, owner) = (d.name.clone(), d.user.clone());
            }
        }
        if on == me.name {
            owner = me.user.clone();
        }
        // Who asked must own the machine: here, and again there when it's passed on, where it's asked as this machine's
        // user, so it's passed on only for the user whose machines both are.
        may_choose(&caller.user, &owner, &on)?;
        if on != me.name {
            if here {
                bail!("set_chrome for {on} reached {}", me.name);
            }
            return self.call_node(&on, "set_chrome", &json!({ "on": on, "allowed": allowed, "here": true }));
        }
        std::fs::write(self.dir.join("chrome.json"), serde_json::to_vec(&json!({ "allowed": allowed }))?)?;
        let mut row = self.chrome_row(&me);
        let n = self.chrome_off("Chrome switched off here");
        if n > 0 {
            row["given_back"] = json!(n);
        }
        Ok(row)
    }

    /// Gives back every chrome device here when chrome devices may not run here now, as when its owner switched them
    /// off, or it came to run others' calls with no choice made: how many it gave back.
    pub(super) fn chrome_off(&self, why: &str) -> usize {
        let running: Vec<Vm> = self.vms().into_iter().filter(|v| v.kind == NAME).collect();
        if running.is_empty() {
            return 0;
        }
        match self.machines() {
            Ok((me, _)) if self.chrome_row(&me)["allowed"] == false => {}
            _ => return 0, // allowed, or not known now
        }
        let mut n = 0;
        for v in running {
            let done = if self.loans().contains_key(&v.name) { self.give_back_vm(&v, why) } else { self.delete_vm(&v) };
            match done {
                Ok(()) => n += 1,
                Err(err) => tracing::warn!("giving back {}: {err:#}", v.name),
            }
        }
        n
    }

    /// Puts a browser profile's last save in a chrome device's folder: its files and folders only.
    fn unpack_profile(&self, v: &Vm, p: &Profile) -> Result<()> {
        if p.blob.is_empty() {
            return Ok(());
        }
        let plain = self.opened(p)?;
        untar(&plain, Path::new(&v.dir)).map_err(|err| anyhow!("unpacking the {} profile: {err:#}", p.name))
    }

    /// Lends a new chrome device here, its Chrome started with a copy of the browser profile in, if any.
    pub(super) fn borrow_chrome(&self, caller: &Caller, input: &Input) -> Result<Value> {
        let why = text(input, "for");
        if why.is_empty() {
            refuse!("say what it's for, in a few words: the user sees it");
        }
        let no = self.chrome_why_not();
        if !no.is_empty() {
            refuse!("none of your devices runs chrome devices now: {no}");
        }
        if !super::kinds::room_for(GB) {
            refuse!("this machine has no memory to spare for another Chrome now: give back one you're done with");
        }
        let p = super::profiles::profile_in(input)?;
        let mut v = self.make_vm(&Chrome, &super::pool::new_pool_name(), None)?;
        let mut l = Loan { by: or(&text(input, "by"), &caller.name()), r#for: why, no_screen: input.get("screen") == Some(&json!(false)), since: super::now(), ..Default::default() };
        if let Some(p) = &p {
            (l.profile, l.item) = (p.name.clone(), p.item.clone());
        }
        let (by, r#for) = (l.by.clone(), l.r#for.clone());
        self.reserve(&v.name, l);
        self.touch(&v.name);
        let ready = (|| -> Result<String> {
            let mut cookies = Vec::new();
            if let Some(p) = &p {
                self.unpack_profile(&v, p)?;
                if cfg!(target_os = "macos") {
                    cookies = super::cookies::take_cookies(Path::new(&v.dir))?;
                }
            }
            self.awake(&mut v)?;
            let b = live_browser(&v)?;
            b.give_cookies(&cookies);
            let session = b.current_page()?;
            Ok(b.page(&session).map(|p| p.target).unwrap_or_default())
        })();
        let target = match ready {
            Ok(t) => t,
            Err(err) => {
                let _ = self.delete_vm(&v);
                self.end_loan(&v.name);
                return Err(err);
            }
        };
        self.lend(&v.name);
        let me = self.me();
        let profile = p.as_ref().map(|p| p.name.clone()).unwrap_or_default();
        tracing::info!("{by} borrowed {} (chrome) for {for:?}, profile {profile:?}", v.name);
        let mut out = json!({ "device": v.name, "on": me, "kind": NAME, "target_id": target_of(&v), "tab_id": tab_id(&target) });
        let mut msg = format!(
            "Borrowed {}, Chrome on {me}, deleted with everything in it when you give it back. Pass device {:?} to the browser tools, with target_id {:?} and tab_id {:?}: its Chrome is running, no browser_prepare needed.",
            v.name,
            v.name,
            target_of(&v),
            tab_id(&target)
        );
        if let Some(p) = &p {
            out["profile"] = json!(p.name);
            msg += &format!(" It started with a copy of the {} profile.", p.name);
        }
        out["text"] = json!(msg + " Give it back with give_back when done; it's given back for you after 15 minutes without a call.");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chrome is allowed on a machine that runs only its owner's calls, and not on one that runs anyone else's, until
    /// its owner chooses; what they chose holds either way.
    #[test]
    fn chrome_by_default() {
        let runs = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (runs_for, chosen, want) in [
            (runs(&["user:u1"]), None, true),
            (runs(&[]), None, true),
            (runs(&["user:u1", "global"]), None, false),
            (runs(&["user:u1", "group:g1"]), None, false),
            (runs(&["user:u1", "user:u2"]), None, false),
            (runs(&["user:u1", "global"]), Some(true), true),
            (runs(&["user:u1"]), Some(false), false),
        ] {
            assert_eq!(allowed(chosen, runs_shared("u1", &runs_for)), want, "running {runs_for:?}, chosen {chosen:?}");
        }
    }

    /// A machine whose service says it runs others' calls too is shared, though its own row doesn't say.
    #[test]
    fn what_a_service_says_it_runs() {
        let st = json!({ "device": { "name": "mac", "runs_for": ["user:u1", "global"] }, "me": { "name": "mac" }, "user": { "id": "u1" },
            "network": { "devices": [{ "name": "mac", "here": true, "online": true }] } });
        let (me, _) = machines_in(&st);
        assert!(runs_shared(&me.user, me.runs_for.as_deref().unwrap_or_default()));
    }

    /// Only a machine's owner switches Chrome on or off there.
    #[test]
    fn only_its_owner_chooses() {
        assert!(may_choose("u1", "u1", "linux").is_ok());
        assert!(may_choose("u2", "u1", "linux").is_err(), "someone else may switch it");
        assert!(may_choose("", "u1", "linux").is_err(), "a call from no one may switch it");
        assert!(may_choose("u1", "", "linux").is_err(), "it may be switched before its owner is known");
    }
}
