//! The device's browser describes a page as an outline, one node per line:
//!
//! ```text
//! - row "Kim Unread message: Lunch? 2 hours ago More options for Kim"
//!   - link "Kim Unread message: Lunch? 2 hours ago" [focusable=true]
//! - statictext "Kim"
//! ```
//!
//! Messenger's chat list, a conversation and search results are read from it.

use std::collections::HashMap;

use anyhow::{Result, anyhow};

pub struct Node {
    pub role: String,
    pub label: String,
}

pub fn outline(text: &str) -> Vec<Node> {
    text.lines().filter_map(line).collect()
}

/// `- role "label"`, the label unquoted.
fn line(l: &str) -> Option<Node> {
    let rest = l.trim_start().strip_prefix("- ")?;
    let n = rest.bytes().take_while(u8::is_ascii_lowercase).count();
    if n == 0 {
        return None;
    }
    let mut label = rest[n..].strip_prefix(" \"").and_then(quoted).unwrap_or_default();
    if label.contains('\\')
        && let Ok(unquoted) = serde_json::from_str::<String>(&format!("\"{label}\""))
    {
        label = unquoted;
    }
    Some(Node { role: rest[..n].to_string(), label })
}

/// The text up to the closing quote, escapes and all; None if it isn't closed.
fn quoted(s: &str) -> Option<String> {
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next()?;
            }
            '"' => return Some(s[..i].to_string()),
            _ => {}
        }
    }
    None
}

/// A row of the chat list.
#[derive(Clone, Debug, Default)]
pub struct Chat {
    pub name: String,
    /// The last message, as the list shows it.
    pub last: String,
    pub time: String,
    pub unread: bool,
    /// The row's link, by its name among the page's refs.
    pub link: String,
}

/// One message of the open conversation.
pub struct Message {
    /// "You" for the account's own.
    pub sender: String,
    pub time: String,
    pub text: String,
}

/// A chat with messages a search found.
pub struct Match {
    pub chat: String,
    /// When the chat has several.
    pub matches: u64,
    /// The message, when it has one.
    pub text: String,
    pub time: String,
}

pub const MENU: &str = "More options for ";

/// One row of a list: its link, its time, and its texts before the time.
#[derive(Default)]
struct Row {
    label: String,
    link: String,
    time: String,
    texts: Vec<String>,
}

fn rows(nodes: &[Node]) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let mut timed = false;
    for n in nodes {
        if n.role == "row" {
            out.push(Row { label: n.label.clone(), ..Default::default() });
            timed = false;
            continue;
        }
        let Some(row) = out.last_mut() else { continue };
        if n.role == "link" && row.link.is_empty() {
            row.link = n.label.clone();
        } else if timed {
        } else if n.role == "abbr" {
            (row.time, timed) = (n.label.clone(), true);
        } else if n.role == "statictext" || n.role == "image" {
            let label = n.label.split(". To get missing image descriptions").next().unwrap_or_default().trim();
            if !label.is_empty() {
                row.texts.push(label.to_string());
            }
        }
    }
    out
}

pub fn chats(nodes: &[Node]) -> Vec<Chat> {
    let mut out = Vec::new();
    for r in rows(nodes) {
        let Some(i) = r.label.rfind(MENU) else { continue };
        let mut c = Chat { name: r.label[i + MENU.len()..].to_string(), time: r.time, link: r.link, ..Default::default() };
        let mut texts = Vec::new();
        for t in r.texts {
            if t == "Unread message:" {
                c.unread = true;
            } else if !(t == "Active now" || t.starts_with("Active ") && t.ends_with(" ago")) {
                texts.push(t);
            }
        }
        c.last = without_name(&texts, &c.name).join(" ");
        out.push(c);
    }
    out
}

/// The chat list's rows across a page's parts, each once: a row cut between two parts shows in both, each missing what
/// the other has.
pub fn chats_in(parts: &[Vec<Node>]) -> Vec<Chat> {
    let mut out: Vec<Chat> = Vec::new();
    let mut at = HashMap::new();
    for part in parts {
        for c in chats(part) {
            let Some(&i) = at.get(&c.name) else {
                at.insert(c.name.clone(), out.len());
                out.push(c);
                continue;
            };
            let link = if out[i].link.is_empty() { c.link.clone() } else { out[i].link.clone() };
            if out[i].time.is_empty() && !c.time.is_empty() {
                out[i] = c;
            }
            out[i].link = link;
        }
    }
    out
}

/// A name whatever its spacing and emoji variants.
fn squash(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{fe0f}')).collect()
}

/// Drops the leading texts that spell the chat's name: a group's emoji come as images between its words.
fn without_name<'a>(texts: &'a [String], name: &str) -> &'a [String] {
    let (want, mut got) = (squash(name), String::new());
    for (i, t) in texts.iter().enumerate() {
        if got == want {
            return &texts[i..];
        }
        let next = got.clone() + &squash(t);
        if !want.starts_with(&next) {
            break;
        }
        got = next;
    }
    match (got == want, texts.is_empty()) {
        (true, _) => &[],
        // The name didn't line up: it's the first text.
        (false, false) => &texts[1..],
        (false, true) => texts,
    }
}

pub fn messages(nodes: &[Node]) -> Vec<Message> {
    nodes
        .iter()
        .filter(|n| n.role == "button")
        .filter_map(|n| message(&n.label))
        .map(|(time, sender, text)| Message { sender: sender.into(), time: time.into(), text: if text.trim().is_empty() { "(attachment)".into() } else { text.trim().into() } })
        .collect()
}

/// A message's button: `Enter, Message sent <time> by <sender>[: <text>]`, its time the shortest that fits.
fn message(label: &str) -> Option<(&str, &str, &str)> {
    let rest = label.strip_prefix("Enter, Message sent ")?;
    for (i, _) in rest.match_indices(" by ").filter(|(i, _)| *i > 0) {
        let after = &rest[i + 4..];
        match after.find(':') {
            None if !after.is_empty() => return Some((&rest[..i], after, "")),
            Some(k) if k > 0 && after[k..].starts_with(": ") => return Some((&rest[..i], &after[..k], &after[k + 2..])),
            _ => {}
        }
    }
    None
}

pub fn matches(nodes: &[Node]) -> Vec<Match> {
    let mut out = Vec::new();
    for r in rows(nodes) {
        if r.texts.is_empty() || r.label.contains(MENU) {
            continue;
        }
        let rest = r.texts[1..].join(" ");
        let count = rest.strip_suffix(" matched messages").or_else(|| rest.strip_suffix(" matched message")).filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        let (matches, text) = match count {
            Some(n) => (n.parse().unwrap_or(0), String::new()),
            None => (0, rest),
        };
        out.push(Match { chat: r.texts[0].clone(), matches, text, time: r.time });
    }
    out
}

/// No recent chat has the name asked for.
#[derive(Debug)]
pub struct NoChat(String);

impl std::fmt::Display for NoChat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoChat {}

/// The chat called `want`: by its whole name, or by a part of it only one chat has.
pub fn pick(chats: &[Chat], want: &str) -> Result<Chat> {
    let w = want.trim().to_lowercase();
    let mut partial = Vec::new();
    for c in chats {
        let n = c.name.to_lowercase();
        if n == w {
            return Ok(c.clone());
        }
        if n.contains(&w) {
            partial.push(c);
        }
    }
    let names = |cs: &mut dyn Iterator<Item = &Chat>| cs.map(|c| c.name.as_str()).collect::<Vec<_>>().join("; ");
    match partial.len() {
        1 => Ok(partial[0].clone()),
        0 => Err(NoChat(format!("no recent chat called {want:?}; recent chats: {}", names(&mut chats.iter()))).into()),
        _ => Err(anyhow!("{want:?} matches several chats: {}", names(&mut partial.into_iter()))),
    }
}
