//! What this node and each of its services say, kept in a file of its own so a user's agent can read it over the
//! network (`one logs <device>`), without ssh and without knowing where the machine's supervisor puts it (systemd's
//! journal isn't readable by the node's own user). Each is `<home>/logs/<name>.log`, `node` or the service's name, each
//! line `<ms since 1970> <text>`, and the one before it, `.log.1`, once it passes `KEPT`: so at most twice that is kept.
//!
//! The text is what `tracing` and the services' programs write to stderr, which still goes there too. Nothing here
//! chooses what's logged: nothing may log a key, token or password (one found is made not to), and what a reader gets is
//! capped (`MOST_LINES`, `MOST_BYTES`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};

pub const NODE: &str = "node";
/// A client's: what its app, page and native side say, kept on the device itself and read through a node that holds its
/// connection (`Net::ask_client`).
pub const CLIENT: &str = "client";
/// The most a log file holds before the next line starts a new one.
const KEPT: u64 = 4 << 20;
/// The most of one line that's kept.
const LINE: usize = 2000;
/// What a read gives at most: lines, and bytes of them. The newest.
pub const MOST_LINES: usize = 1000;
pub const MOST_BYTES: usize = 48 << 10;
pub const LINES: usize = 200;

struct Sink {
    path: PathBuf,
    file: std::fs::File,
    size: u64,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

fn file(home: &Path, name: &str) -> PathBuf {
    home.join("logs").join(format!("{name}.log"))
}

/// Starts keeping what this process says, as `name`'s log (`NODE`, or a service's name), in this machine's home.
pub fn keep(name: &str) {
    keep_in(&platform::home(), name);
}

/// The same in `home`: a native app's own folder.
pub fn keep_in(home: &Path, name: &str) {
    let path = file(home, name);
    let opened = path.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::OpenOptions::new().create(true).append(true).open(&path));
    if let Ok(file) = opened {
        let size = file.metadata().map_or(0, |m| m.len());
        *SINK.lock().unwrap() = Some(Sink { path, file, size });
    }
}

/// Keeps `text`, each line of it as of now.
pub fn record(text: &str) {
    let mut sink = SINK.lock().unwrap();
    let Some(sink) = sink.as_mut() else { return };
    let now = now_ms();
    let mut out = String::new();
    for line in plain(text).lines().filter(|l| !l.trim().is_empty()) {
        out.push_str(&format!("{now} {}\n", line.chars().take(LINE).collect::<String>()));
    }
    if out.is_empty() {
        return;
    }
    if sink.size >= KEPT {
        let _ = std::fs::rename(&sink.path, sink.path.with_extension("log.1"));
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&sink.path) {
            (sink.file, sink.size) = (file, 0);
        }
    }
    if sink.file.write_all(out.as_bytes()).is_ok() {
        sink.size += out.len() as u64;
    }
}

/// Says `text` to stderr, and keeps it.
pub fn say(text: &str) {
    eprintln!("{text}");
    record(text);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// `text` as it's kept: no terminal colours or other escapes, and nothing that may be a secret. Nothing logs one, but a
/// library's error may quote the URL it asked (a signed one's query is its key), or a header: a URL's query and a
/// bearer token are left out, whatever says them.
fn plain(text: &str) -> String {
    let (text, mut out) = (strip(text), String::new());
    let mut rest = text.as_str();
    while let Some(at) = [rest.find("://"), rest.find("Bearer ")].into_iter().flatten().min() {
        let (head, tail) = rest.split_at(at);
        let end = |s: &str, from: usize, stops: &str| s[from..].find(|c: char| c.is_whitespace() || stops.contains(c)).map_or(s.len(), |i| from + i);
        if tail.starts_with("Bearer ") {
            let to = end(tail, 7, "\"')");
            out.push_str(head);
            out.push_str("Bearer …");
            rest = &tail[to..];
            continue;
        }
        let to = end(tail, 3, "\"')>]");
        let url = &tail[..to];
        out.push_str(head);
        match url.split_once(['?', '#']) {
            Some((before, _)) => out.push_str(&format!("{before}?…")),
            None => out.push_str(url),
        }
        rest = &tail[to..];
    }
    out.push_str(rest);
    out
}

/// `text` without terminal colours and other escapes.
fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What a reader asks for.
pub struct Ask {
    /// A service's log; none, the node's.
    pub service: Option<String>,
    /// Only the last this long.
    pub since: Option<Duration>,
    /// Only lines with this in them, whatever its case.
    pub grep: Option<String>,
    pub lines: usize,
}

impl Ask {
    pub fn from_json(input: &Value) -> Result<Ask> {
        // What a command line says as 5 is as much text as "5" is.
        let text = |k: &str| match &input[k] {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        Ok(Ask {
            service: text("service"),
            since: text("since").map(|s| since(&s)).transpose()?,
            grep: text("grep"),
            lines: input["lines"].as_u64().map_or(LINES, |n| (n as usize).clamp(1, MOST_LINES)),
        })
    }
}

/// `10m`, `2h`, `30s` or `1d`, as a time.
pub fn since(text: &str) -> Result<Duration> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (n, unit) = text.split_at(split);
    let n: u64 = n.parse().map_err(|_| anyhow::anyhow!("--since is a number and s, m, h or d: 10m, 2h"))?;
    let unit = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => bail!("--since is a number and s, m, h or d: 10m, 2h"),
    };
    Ok(Duration::from_secs(n * unit))
}

/// A log as a reader gets it: the newest lines that match, up to the caps, oldest first, each starting with when if it
/// doesn't say.
pub fn read(home: &Path, ask: &Ask) -> Result<Value> {
    let name = ask.service.as_deref().unwrap_or(NODE);
    ensure!(!name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'), "{name} isn't a service's name");
    let path = file(home, name);
    let mut stored = std::fs::read_to_string(path.with_extension("log.1")).unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Ok(now) => stored.push_str(&now),
        Err(_) if stored.is_empty() => bail!("{name} has no log here{}", match name {
            NODE => " (this node started before it kept one)",
            CLIENT => " (this app hasn't written one yet)",
            _ => ": it hasn't run on this device",
        }),
        Err(_) => {}
    }
    let from = ask.since.map_or(0, |s| now_ms().saturating_sub(s.as_millis() as u64));
    let grep = ask.grep.as_ref().map(|g| g.to_lowercase());
    let all: Vec<(u64, &str)> = stored.lines().filter_map(|l| l.split_once(' ')).filter_map(|(t, text)| Some((t.parse().ok()?, text))).collect();
    let oldest = all.first().map(|l| l.0);
    let matched: Vec<&(u64, &str)> = all.iter().filter(|(t, text)| *t >= from && grep.as_ref().is_none_or(|g| text.to_lowercase().contains(g))).collect();
    let mut shown: Vec<String> = matched.iter().rev().take(ask.lines).map(|(t, text)| stamped(*t, text)).collect();
    let mut bytes = 0;
    shown.retain(|l| {
        bytes += l.len() + 1;
        bytes <= MOST_BYTES
    });
    shown.reverse();
    Ok(json!({ "log": name, "matched": matched.len(), "shown": shown.len(), "oldest": oldest, "lines": shown }))
}

/// A client's answer when a node asks for its log (`Net::ask_client`): `{ ok }` as `read` gives it, or `{ error }`. A client
/// has the one log.
pub fn answer(home: &Path, input: &Value) -> Value {
    let read = Ask::from_json(input).and_then(|ask| {
        ensure!(ask.service.is_none(), "a client has one log, so --service names none");
        read(home, &Ask { service: Some(CLIENT.into()), ..ask })
    });
    match read {
        Ok(ok) => json!({ "ok": ok }),
        Err(e) => json!({ "error": format!("{e:#}") }),
    }
}

/// A line, with its time before it unless it starts with its own (tracing's).
fn stamped(ms: u64, text: &str) -> String {
    let own = text.len() > 20 && text.as_bytes()[..4].iter().all(u8::is_ascii_digit) && text.as_bytes()[4] == b'-';
    if own {
        return text.to_string();
    }
    format!("{} {text}", utc(ms))
}

/// `ms` since 1970 as `2026-11-06T22:25:00Z`.
fn utc(ms: u64) -> String {
    let (days, secs) = ((ms / 1000 / 86400) as i64, ms / 1000 % 86400);
    // The civil date of a day count (Howard Hinnant's days_from_civil, backwards).
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_is_capped_filtered_and_only_a_services_name() {
        let home = std::env::temp_dir().join(format!("logs-test-{}", now_ms()));
        std::fs::create_dir_all(home.join("logs")).unwrap();
        let now = now_ms();
        let mut text = String::new();
        for i in 0..3000 {
            text.push_str(&format!("{} line {i} {}\n", now - 3_600_000 + i, if i % 2 == 0 { "ERROR" } else { "fine" }));
        }
        std::fs::write(home.join("logs/node.log"), text).unwrap();
        let ask = |service: Option<&str>, since: Option<&str>, grep: Option<&str>, lines: u64| Ask::from_json(&json!({ "service": service, "since": since, "grep": grep, "lines": lines })).unwrap();
        let all = read(&home, &ask(None, None, None, 100_000)).unwrap();
        assert_eq!(all["shown"], MOST_LINES, "never more lines than the cap");
        assert!(all["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().len() + 1).sum::<usize>() <= MOST_BYTES);
        let errors = read(&home, &ask(None, None, Some("error"), 5)).unwrap();
        assert_eq!(errors["matched"], 1500);
        assert!(errors["lines"].as_array().unwrap().iter().all(|l| l.as_str().unwrap().contains("ERROR")));
        assert!(errors["lines"][4].as_str().unwrap().contains("line 2998"), "the newest, last");
        assert_eq!(read(&home, &ask(None, Some("1m"), None, 10)).unwrap()["matched"], 0, "nothing in the last minute");
        assert!(read(&home, &ask(Some("../node"), None, None, 10)).is_err(), "a service's name is not a path");
        assert!(read(&home, &ask(Some("whatsapp"), None, None, 10)).is_err(), "a service that never ran has no log");
        assert!(since("soon").is_err());
        assert_eq!([0, 951_868_799_999, 1_762_468_500_000, 4_107_542_400_000].map(utc), ["1970-01-01T00:00:00Z", "2000-02-29T23:59:59Z", "2025-11-06T22:35:00Z", "2100-03-01T00:00:00Z"]);
        let kept = plain("\x1b[31mWARN\x1b[0m asked https://bucket.example/a/b?X-Amz-Signature=abc123&k=v Bearer s3cr3t and http://127.0.0.1:4747/api/state ok");
        assert_eq!(kept, "WARN asked https://bucket.example/a/b?… Bearer … and http://127.0.0.1:4747/api/state ok");
        std::fs::remove_dir_all(&home).unwrap();
    }
}

pub mod tools;

#[cfg(feature = "node")]
pub mod cli;

#[cfg(feature = "node")]
mod tee;
#[cfg(feature = "node")]
pub use tee::*;
