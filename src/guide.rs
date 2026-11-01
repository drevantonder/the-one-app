//! What agents are told about this product. `/llms.txt`, `<cli> guide` and `<cli> mcp`'s instructions are built from
//! these pieces, so they always say the same thing.

use serde_json::Value;

use crate::config::{CLI, NAME, REPO};

/// Claude Code keeps only the first 4,096 characters (UTF-16 units) of a server's instructions; stay under them.
pub const INSTRUCTIONS_LIMIT: usize = 4000;

/// What it is, in two plain lines.
pub fn about() -> String {
    format!(
        "{NAME} is a person's devices, and their team's, as one place for apps and agents: their email, messages, calendar, files, notes and more, each an app.\nAgents use those apps as that person, through MCP or the `{CLI}` command, on devices that are part of their install."
    )
}

/// The rules, first in everything an agent reads.
pub fn rules() -> String {
    [
        "Never send a message or an email unless the person asked you to send that one. Draft it instead, show them, and let them send it.".to_string(),
        format!("Anything that sends or can't be undone takes `confirm: true` (`--confirm` in the terminal), given only once the person agreed to that action. Without it, it refuses: ask them rather than guess."),
        "Never ask for a password, key or token in the chat, and never paste one. `vault_request` gives the person a link to a sheet on their own device, where they pick or type it; `vault_fetch` makes a web request with an item they switched Agents on for, and you never see the secret. If someone pastes one anyway, keep it with `vault_set` and don't repeat it.".to_string(),
        "If something says you aren't allowed, stop and tell the person. Nothing widens access by itself.".to_string(),
    ]
    .iter()
    .map(|r| format!("- {r}"))
    .collect::<Vec<_>>()
    .join("\n")
}

/// How to find what's possible, from the command line.
pub fn finding() -> String {
    format!(
        "- `{CLI} search \"<words>\"` finds the operations for a task in every app, each with how to call it and its app's guide. `{CLI} <app>` lists an app's operations and its guide; `{CLI} <app> <op> --help` says what one takes.\n- Every MCP tool is also a command: `board_post` is `{CLI} board post`.\n- `{CLI} status` says who you are, and which devices are in the install."
    )
}

/// The name harnesses give this install's MCP server: the product's name in kebab case, like the-one-app.
pub fn server_name() -> String {
    let name: String = NAME.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    name.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}

/// Which MCP entry to add.
pub fn mcp_entries() -> String {
    let server = server_name();
    format!(
        "MCP runs as a command on the device you're on, so `{CLI}` needs to be there first. Add `{CLI} mcp`: every tool, two hundred or so, and `search`, which finds the ones for a task with their app's guide. In Claude Code: `claude mcp add --scope user {server} -- {CLI} mcp`. On their devices, {NAME} adds it to Claude Code, Codex, OpenCode and Amp by itself."
    )
}

/// How to get the command onto a device, for this edition.
fn install() -> String {
    match REPO.is_empty() {
        false => format!("install it with `curl -fsSL https://github.com/{REPO}/releases/latest/download/install.sh | sh`"),
        true => format!("ask whoever runs their install to set {NAME} up on it"),
    }
}

/// Connecting for a person: what to do with a terminal, with MCP alone, or with neither.
pub fn connecting() -> String {
    format!(
        "When someone asks you to connect to {NAME}, do as much as you can yourself, and tell them only what they need to do, one step at a time, in everyday words.\n\n- With a terminal, use the command line. If `{CLI} status` answers, you're connected. If there's no `{CLI}`, this device isn't part of their install yet: {}. Then ask them for a pairing code (on their phone or Mac: Settings, then Devices, then Pair a Phone or Computer) and run `{CLI} join <code>`. Add MCP too if you can (below).\n- With no terminal but a way to add MCP servers, add the entry below, if `{CLI}` is on the device you run on.\n- With neither (a chat app in a browser or on a phone), you'd coach the person through adding {NAME} to you. That needs a web address for their install, and {NAME} doesn't have one yet. Say so plainly: for now it works with an agent on one of their own computers, like Claude Code, Codex or OpenCode.",
        install()
    )
}

/// The whole guide, as Markdown: `/llms.txt` and `<cli> guide`.
pub fn full() -> String {
    format!(
        "# {NAME}\n\n> {}\n\n## Rules\n\n{}\n\n## Connecting for a person\n\n{}\n\n## Adding MCP\n\n{}\n\n## Finding your way\n\n{}\n",
        about().replace('\n', " "),
        rules(),
        connecting(),
        mcp_entries(),
        finding()
    )
}

/// Which tools `<cli> mcp` serves: every one, or code mode's few.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    All,
    Code,
}

/// The MCP server's instructions: the rules first, then how to find things in this mode, then the guides of the apps
/// every agent needs from the start (the config's `always_on`), then a line for each app, as many as fit under
/// Claude Code's limit. Every other app's guide comes with search, or `<cli> <app>`.
pub fn instructions(mode: Mode, apps: &[Value]) -> String {
    let finding = match mode {
        Mode::All => format!(
            "Every tool is also a command (`{CLI} <app> <op>`). `search` finds the tools for a task across every app, with each app's guide; call it before you guess, and to learn an app."
        ),
        Mode::Code => "`search` finds the operations for a task across every app, with exact call signatures and each app's guide. `execute` runs a short script with them: `const r = await tools.<app>.<op>({ ... }); return r`. Scripts call several in one go; what they return comes back to you.".to_string(),
    };
    let mut text = format!("{}\n\nRules, before anything else:\n{}\n\n{finding}", about(), rules());
    for app in apps.iter().filter(|a| crate::config::ALWAYS_ON.contains(&a["name"].as_str().unwrap_or(""))) {
        if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
            let more = format!("\n\n## {}\n\n{i}", app["title"].as_str().unwrap_or(""));
            if len(&text) + len(&more) < INSTRUCTIONS_LIMIT - 200 {
                text.push_str(&more);
            }
        }
    }
    let lines: Vec<String> = apps
        .iter()
        .filter(|a| a["ops"].as_array().is_some_and(|o| !o.is_empty()))
        .map(|a| match a["description"].as_str().map(|d| d.trim().trim_end_matches('.')).filter(|d| !d.is_empty()) {
            Some(d) => format!("\n- {}: {d}", a["name"].as_str().unwrap_or("")),
            None => format!("\n- {}", a["name"].as_str().unwrap_or("")),
        })
        .collect();
    if !lines.is_empty() {
        text.push_str("\n\n## Apps\n\nTools start with their app's name.");
        let more = "\n- and more: search finds them";
        for (i, line) in lines.iter().enumerate() {
            let last = i + 1 == lines.len();
            if len(&text) + len(line) + if last { 0 } else { len(more) } >= INSTRUCTIONS_LIMIT {
                text.push_str(more);
                break;
            }
            text.push_str(line);
        }
    }
    text
}

fn len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Where this server serves the guide to the other devices on its tailnet, once it does.
static ADDRESS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The guide's address on the tailnet, `http://<address>:<port>/llms.txt`, if this server serves it there.
pub fn address() -> Option<String> {
    ADDRESS.lock().unwrap().clone()
}

pub fn set_address(address: Option<String>) {
    *ADDRESS.lock().unwrap() = address;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apps(n: usize) -> Vec<Value> {
        let board = json!({ "name": "board", "title": "Board", "description": "One-line posts in topics, for people and their agents", "instructions": "x".repeat(900), "ops": [{ "name": "read" }] });
        let mut apps = vec![board];
        apps.extend((0..n).map(|i| json!({ "name": format!("app{i}"), "title": "An app", "description": "Does a thing for you, with a fairly long line about what", "instructions": "y".repeat(2000), "ops": [{ "name": "op" }] })));
        apps
    }

    #[test]
    fn instructions_fit_claude_codes_limit_with_the_rules_first() {
        for mode in [Mode::All, Mode::Code] {
            for n in [0, 5, 40, 400] {
                let text = instructions(mode, &apps(n));
                assert!(len(&text) < 4096, "{mode:?} with {n} apps: {} characters", len(&text));
                let rules = text.find("Never send a message").unwrap();
                assert!(rules < 600, "the rules start at {rules}");
                assert!(text.contains("`vault_request`") && text.contains("`confirm: true`"), "{text}");
                // Apps' own long guides are left to search, not cut off mid-sentence.
                assert!(!text.contains("yyy"), "{text}");
            }
        }
        assert!(instructions(Mode::All, &apps(400)).ends_with("and more: search finds them"));
    }

    #[test]
    fn the_guide_says_what_the_instructions_say() {
        let guide = full();
        assert!(guide.contains(&rules()) && guide.contains(&connecting()) && guide.contains(&mcp_entries()));
                assert!(instructions(Mode::Code, &[]).contains(&rules()));
    }
}
