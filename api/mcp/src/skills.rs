//! What agents are told, as Agent Skills (agentskills.io): a root skill, short enough to load up front, and deeper ones
//! loaded when needed: one for each app with a guide (its instructions and tools, from the same declarations its
//! tools come from), the Vault's, and a few areas of their own (`src/skills/`). One source, every way out: the agents
//! service writes them into each harness's skills folder, `<cli> mcp` serves them (Skills over MCP, SEP-2640), a
//! device serves the root's body as `/llms.txt` and each skill's files at `/skills/<name>/<path>`, and `<cli> skills`
//! prints them. MCP's instructions are the root's short version.

use serde_json::{Value, json};

use ::custom_build::config::{CLI, NAME, REPO, YOURS};
use ::api::SENDS;

/// Claude Code keeps only the first 4,096 characters (UTF-16 units) of an MCP server's instructions; stay under them.
pub const INSTRUCTIONS_LIMIT: usize = 4000;

pub fn about() -> String {
    format!(
        "{NAME} is a user's devices, and their team's, as one place for apps and agents: their email, messages, calendar, files, notes and more, each an app.\nAgents use those apps as that user, through MCP or the `{CLI}` command, on any device in {YOURS}."
    )
}

/// The rules, first in everything an agent reads.
pub fn rules() -> String {
    [
        "Never send a message or an email unless the user asked you to send that one. Draft it instead, show them, and let them send it.".to_string(),
        format!("Anything that {SENDS} takes `confirm: true` (`--confirm` in the terminal). That is you saying the user agreed to that action in conversation: ask them as a colleague would, then give it. Nothing else checks."),
        "Never ask for a password, key or token in the chat, and never paste one. `vault_request` gives the user a link to a sheet on their own device, where they pick or type it; `vault_fetch` makes a web request with an item they let agents use, and you never see the secret. If someone pastes one anyway, keep it with `vault_set` and don't repeat it.".to_string(),
        "If something says you aren't allowed, stop and tell the user. Nothing widens access by itself.".to_string(),
    ]
    .iter()
    .map(|r| format!("- {r}"))
    .collect::<Vec<_>>()
    .join("\n")
}

pub fn finding() -> String {
    format!(
        "- Each area has a skill of its own: load the one for what you're doing. `{CLI} skills` lists them and `{CLI} skills <name>` prints one.\n- `{CLI} search \"<words>\"` finds the tools for a task in every app, each with how to call it and its app's guide. `{CLI} <app>` lists an app's tools and its guide; `{CLI} <app> <tool> --help` says what one takes.\n- Every MCP tool is also a command: `repos_list` is `{CLI} repos list`.\n- `{CLI} run <file>` runs a short script that calls several tools and prints what it returns: `{CLI} search` gives each call as the script writes it.\n- `{CLI} status` says who you are, and the users and devices in {YOURS}. You act for the user it marks (you): write their name, never this computer's login.\n- When a call fails or is slow, `{CLI} <app> <tool> --trace` (`trace: true` in MCP) says which device ran it and where it waited, and `{CLI} logs <device>` (the `logs` tool) reads that device's log, or one of its services' with `--service`: the user's own, or an admin's. `{CLI} insights` (the `insights` tool) says what's failing or slow across {YOURS}, down to one trace.\n- Before saying you can't, search: an app the user hasn't added yet may do it, and search offers it.\n- When the user asks you to make an app, tracker or tool, make it here, where it's on all their devices: the `{}-build` skill says how.",
        server_name()
    )
}

/// The name harnesses give this network's MCP server: the product's name in kebab case, like the-one-app.
pub fn server_name() -> String {
    let name: String = NAME.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    name.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}

pub fn mcp_entries() -> String {
    let server = server_name();
    format!(
        "On a device where `{CLI}` runs, MCP is a command there, one of two entries:\n\n- `{CLI} mcp`: every tool, and `search`. For agents that already find tools themselves or run code with them: Claude Code, OpenCode, Codex, Cursor. In Claude Code: `claude mcp add --scope user {server} -- {CLI} mcp`\n- `{CLI} mcp --code`: a few tools. `search` finds what's possible, `execute` runs a short script with it, and {} stay tools of their own. For every other agent.\n\nPick the one that fits you, or tell the user in one line which you're adding and why. On their devices, {NAME} adds the right one to Claude Code, Codex, OpenCode and Amp by itself.",
        ::custom_build::config::ALWAYS_ON.iter().map(|a| format!("{a}'s tools")).chain(crate::code::DIRECT.iter().map(|t| format!("`{t}`"))).collect::<Vec<_>>().join(", ")
    )
}

fn install() -> String {
    match scope::state::install_line() {
        Some(line) => format!("install it with `{line}`"),
        None => format!("ask an admin of {YOURS} to set it up there"),
    }
}

/// Connecting for a user, at the network's `address` where it's known: a shell takes the command line, anything
/// else adds the MCP server at the address.
pub fn connecting(address: Option<&str>) -> String {
    let mcp = match address {
        Some(a) => format!("`{}/mcp`", web(a)),
        None => format!("the web address of {YOURS} followed by `/mcp`, like `https://{CLI}.example.com/mcp`"),
    };
    format!(
        "When someone asks you to connect to {NAME}, do it yourself, and tell them only what they need to do, one step at a time, in everyday words.\n\n- If you can run commands (Claude Code, Codex, OpenCode, Cursor): run `{CLI} status`. If it answers, you're on one of their devices already: use the command line, and add MCP as below. If there's no `{CLI}`, {}, then ask them for a pairing code (Settings, then Devices) and run `{CLI} join <code>`.\n- If you can't (ChatGPT, Claude and other chat apps): you can't add it yourself, so show them where this app adds a connector (a custom MCP server) and have them add {mcp}. They sign in when it asks, and then you act as them.",
        install()
    )
}

/// An address as a URL: with https:// unless it says otherwise, and no trailing slash.
fn web(address: &str) -> String {
    let address = address.trim().trim_end_matches('/');
    if address.contains("://") { address.to_string() } else { format!("https://{address}") }
}

/// The guide to connecting, the root skill's body without its list of skills: what anyone may read at the network's
/// `address` (`/llms.txt`), since it says nothing of what apps a network has.
pub fn guide(address: Option<&str>) -> String {
    format!(
        "# {NAME}\n\n{}\n\n## Rules\n\n{}\n\n## Connecting for a user\n\n{}\n\n## Adding MCP\n\n{}\n\n## Finding your way\n\n{}\n",
        about(),
        rules(),
        connecting(address),
        mcp_entries(),
        finding()
    )
}

/// A skill: its name and description (its frontmatter), the body of its SKILL.md, and its other files.
#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub files: Vec<(String, String)>,
}

impl Skill {
    pub fn skill_md(&self) -> String {
        // JSON strings are YAML's double-quoted ones.
        format!("---\nname: {}\ndescription: {}\n---\n\n{}\n", self.name, json!(self.description), self.body.trim_end())
    }

    pub fn all_files(&self) -> Vec<(String, String)> {
        [vec![("SKILL.md".to_string(), self.skill_md())], self.files.clone()].concat()
    }

    pub fn file(&self, path: &str) -> Option<String> {
        self.all_files().into_iter().find(|(p, _)| p == path).map(|(_, t)| t)
    }
}

/// The root skill's description: short, so a harness can keep it loaded.
fn root_description() -> String {
    format!(
        "Use whenever the user pastes or mentions a key, token or password (keep it in the Vault, never use it raw), wants a message sent, mentions {NAME} or its apps, asks about their devices (set up a laptop or phone), their agents' instructions and skills or their AI plans, or asks you to connect to it; read it before any of its tools. {NAME} is their email, messages, calendar, files and more, as apps their agents use."
    )
}

/// The root skill, pointing to the others, each named by `link` (a skill's name in a harness, a URL in /llms.txt).
pub fn root(others: &[Skill], link: impl Fn(&Skill) -> String) -> Skill {
    let list: Vec<String> = others.iter().map(|s| format!("- {}: {}", link(s), s.description)).collect();
    let body = format!("{}\n## Skills\n\n{}\n", guide(None), list.join("\n"));
    Skill { name: server_name(), description: root_description(), body, files: vec![] }
}

/// A skill's text by its name, or one of its other files as `<name>/<path>`.
pub fn read(sets: &[Value], name: &str) -> Option<String> {
    let (skill, file) = name.split_once('/').unwrap_or((name, "SKILL.md"));
    all(sets).into_iter().find(|s| s.name == skill)?.file(file)
}

/// The skills under the root, for `sets` of tools, the core's features first, then the apps (as the API's document
/// lists them, or onet's `features()` then `GET /api/apps`, those added for the user having their tools): the areas
/// of their own, then one for each set with a guide.
pub fn deeper(sets: &[Value]) -> Vec<Skill> {
    let code = match REPO.is_empty() {
        false => REPO.to_string(),
        true => format!("ask an admin of {YOURS} where it is"),
    };
    let fill = |t: &str| t.replace("{{name}}", NAME).replace("{{cli}}", CLI).replace("{{server}}", &server_name()).replace("{{code}}", &code).replace("{{yours}}", YOURS);
    let written = [include_str!("skills/accounts/SKILL.md"), include_str!("skills/build/SKILL.md")];
    let mut skills: Vec<Skill> = written
        .into_iter()
        .map(|md| {
            let md = fill(md);
            let (front, body) = md.trim_start_matches("---\n").split_once("\n---\n").unwrap_or(("", &md));
            let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}: "))).unwrap_or("").to_string();
            Skill { name: field("name"), description: field("description"), body: body.trim().to_string(), files: vec![] }
        })
        .collect();
    // The build skill carries the code that says what an app gets, as it is in this build, so an agent with only MCP
    // reads it here rather than on the web: what a call's ctx has, a screen's one, @one/ui (its widget nodes too), and
    // the app to copy.
    if let Some(build) = skills.iter_mut().find(|s| s.name.ends_with("-build")) {
        build.files = [
            ("ctx.js", include_str!("../../../app-runtime/src/app/runtime/shim.js")),
            ("one.js", include_str!("../../../ui/bridge.js")),
            ("ui/index.ts", include_str!("../../../ui/kit/src/index.ts")),
            ("ui/layout.tsx", include_str!("../../../ui/kit/src/components/layout.tsx")),
            ("ui/problem.tsx", include_str!("../../../ui/kit/src/components/problem.tsx")),
            ("ui/empty.tsx", include_str!("../../../ui/kit/src/components/empty.tsx")),
            ("ui/icon.tsx", include_str!("../../../ui/kit/src/components/icon.tsx")),
            ("ui/app-icon.tsx", include_str!("../../../ui/kit/src/components/app-icon.tsx")),
            ("ui/widget.tsx", include_str!("../../../ui/kit/src/components/widget.tsx")),
            ("example/app.js", include_str!("skills/build/example/app.js")),
            ("example/screens/main.jsx", include_str!("skills/build/example/screens/main.jsx")),
        ]
        .into_iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
    }
    // Each set of tools with a guide.
    skills.extend(sets.iter().filter(|a| a["instructions"].as_str().is_some_and(|i| !i.trim().is_empty()) && a["tools"].as_array().is_some_and(|o| !o.is_empty())).map(app_skill));
    skills
}

pub fn all(sets: &[Value]) -> Vec<Skill> {
    let deeper = deeper(sets);
    [vec![root(&deeper, |s| format!("`{}`", s.name))], deeper].concat()
}

/// A skill's name, from what it's about: `<server>-<area>`, in the characters a name may have.
fn skill_name(area: &str) -> String {
    let area: String = area.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let name = format!("{}-{}", server_name(), area.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-"));
    name.chars().take(64).collect::<String>().trim_end_matches('-').to_string()
}

/// An app's skill: its guide, then each tool as its tool says it.
pub fn app_skill(app: &Value) -> Skill {
    let (name, title) = (app["name"].as_str().unwrap_or(""), app["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(app["name"].as_str().unwrap_or("")));
    let about = app["description"].as_str().map(|d| d.trim().trim_end_matches('.')).filter(|d| !d.is_empty()).map(|d| format!(": {d}")).unwrap_or_default();
    let description: String = format!("{title} in {NAME}{about}. Use when the user asks about {title}, and before using its tools ({name}_...).").chars().take(1024).collect();
    let tools: Vec<&Value> = app["tools"].as_array().into_iter().flatten().filter(|o| !o["name"].as_str().unwrap_or("").starts_with('_')).collect();
    // The guide names tools as MCP does (gmail_search); a harness with only the command line reads each as its command too.
    let mut guide = app["instructions"].as_str().unwrap_or("").trim().to_string();
    let mut by_length: Vec<&str> = tools.iter().filter_map(|o| o["name"].as_str()).collect();
    by_length.sort_by_key(|o| std::cmp::Reverse(o.len()));
    for short in by_length {
        let tool = format!("{name}_{short}");
        let mut out = String::new();
        let mut rest = guide.as_str();
        while let Some(i) = rest.find(&tool) {
            let after = &rest[i + tool.len()..];
            let whole = !rest[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '`' || c == '-') && !after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '`');
            out.push_str(&rest[..i]);
            out.push_str(&if whole { format!("`{CLI} {name} {short}` ({tool})") } else { tool.clone() });
            rest = after;
        }
        out.push_str(rest);
        guide = out;
    }
    let mut body = format!(
        "# {title}\n\n{}\n\n## Tools\n\nEach is the command `{CLI} {name} <tool>` (`--help` says more). Over MCP, it is the tool `{name}_<tool>`, with the same inputs. Never ask for a password, key or token in the chat: `{CLI} vault request` (vault_request) gives the user a link. What {SENDS} takes `confirm: true` (`--confirm`): that is you saying the user agreed to that very action in conversation, and nothing else checks.\n",
        guide,
    );
    for tool in tools {
        let mark = match (tool["reads"] == true, tool["confirm"] == true) {
            (true, _) => " It only reads.",
            (_, true) => " Needs `confirm: true`.",
            _ => "",
        };
        body.push_str(&format!("\n- `{}`: {}{mark}", crate::tools::usage(name, tool), tool["description"].as_str().unwrap_or("").trim()));
    }
    Skill { name: skill_name(name), description, body, files: vec![] }
}

/// An MCP tool's hints, all three always, since a client takes one left out as the worst: whether it only reads (so it
/// can run beside others and be asked again), and whether it sends or can't be undone (a tool's `confirm`).
pub fn hints(reads: bool, confirm: bool) -> Value {
    json!({ "readOnlyHint": reads, "destructiveHint": confirm && !reads, "idempotentHint": reads })
}

/// The input that says the user agreed, on what sends or can't be undone.
pub fn confirm_input() -> Value {
    json!({ "type": "boolean", "description": format!("true once the user agreed to this very action: it {SENDS}") })
}

/// A JSON Schema as a TypeScript type, as code mode's signatures show it.
pub fn ts(schema: &Value) -> String {
    if let Some(choices) = schema["enum"].as_array() {
        return choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" | ");
    }
    match schema["type"].as_str() {
        Some("string") => "string".into(),
        Some("integer" | "number") => "number".into(),
        Some("boolean") => "boolean".into(),
        Some("array") => format!("{}[]", match ts(&schema["items"]) { t if t.contains(' ') => format!("({t})"), t => t }),
        Some("object") | None if schema["properties"].is_object() => {
            let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            // The required ones first, as they're asked for.
            let props = schema["properties"].as_object().cloned().unwrap_or_default();
            let mut keys: Vec<&String> = required.iter().filter_map(|r| props.get_key_value(*r).map(|(k, _)| k)).collect();
            keys.extend(props.keys().filter(|k| !required.contains(&k.as_str())));
            let props: Vec<String> = keys.iter().map(|k| format!("{k}{}: {}", if required.contains(&k.as_str()) { "" } else { "?" }, ts(&props[*k]))).collect();
            if props.is_empty() { "{}".into() } else { format!("{{ {} }}", props.join(", ")) }
        }
        Some("object") => "object".into(),
        _ => "unknown".into(),
    }
}

/// Which tools `<cli> mcp` serves: every one, or code mode's few.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    All,
    Code,
}

impl Mode {
    /// Who reads a search's answer in this mode: an MCP client calling tools, or a script.
    pub fn reader(self) -> ::api::Reader {
        match self {
            Mode::All => ::api::Reader::Mcp,
            Mode::Code => ::api::Reader::Code,
        }
    }
}

/// The MCP server's instructions: the rules first, then how to find things in this mode, then the guides of the apps
/// every agent needs from the start (the config's `always_on`), then a line for each of the core's features and each
/// app, as many as fit under Claude Code's limit. Every other app's guide comes with search, or `<cli> <app>`.
pub fn instructions(mode: Mode, apps: &[Value], user: Option<&str>) -> String {
    let finding = match mode {
        Mode::All => format!(
            "Every tool is also a command (`{CLI} <app> <tool>`). `search` finds the tools for a task across every app, with each app's guide; call it before you guess, to learn an app, and before saying you can't: it offers apps the user hasn't added yet."
        ),
        Mode::Code => "`search` finds the tools for a task across every app, with exact call signatures and each app's guide. `execute` runs a short script with them: `const r = await tools.<app>.<tool>({ ... }); return r`. Scripts call several in one go; what they return comes back to you. Guides name tools <app>_<tool>: in a script, that's tools.<app>.<tool>.".to_string(),
    };
    let user = user.filter(|p| !p.trim().is_empty()).map(|p| format!("\n\nYou act for {p}: groups_list says who else is in {YOURS}.")).unwrap_or_default();
    let mut text = format!(
        "{}{user}\n\nRules, before anything else:\n{}\n\n{finding} Each app's guide is also a skill, `{}-<app>`, with one for the whole of {NAME}, `{}`: load them from your skills, or give `search` a skill's name, or `{CLI} skills`. When the user asks you to make an app, tracker or tool, make it here, where it's on all their devices: the `{}-build` skill says how.",
        about(),
        rules(),
        server_name(),
        server_name(),
        server_name()
    );
    for app in apps.iter().filter(|a| ::custom_build::config::ALWAYS_ON.contains(&a["name"].as_str().unwrap_or(""))) {
        if let Some(i) = app["instructions"].as_str().filter(|i| !i.is_empty()) {
            let more = format!("\n\n## {}\n\n{i}", app["title"].as_str().unwrap_or(""));
            if len(&text) + len(&more) < INSTRUCTIONS_LIMIT - 200 {
                text.push_str(&more);
            }
        }
    }
    let lines: Vec<String> = apps
        .iter()
        .filter(|a| a["tools"].as_array().is_some_and(|o| !o.is_empty()))
        .map(|a| match a["description"].as_str().map(|d| d.trim().trim_end_matches('.')).filter(|d| !d.is_empty()) {
            Some(d) => format!("\n- {}: {d}", a["name"].as_str().unwrap_or("")),
            None => format!("\n- {}", a["name"].as_str().unwrap_or("")),
        })
        .collect();
    if !lines.is_empty() {
        text.push_str(&format!("\n\n## Tools\n\nTools start with the name of what they're for: {NAME}'s own first, then apps."));
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

