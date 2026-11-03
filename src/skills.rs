//! What agents are told, as Agent Skills (agentskills.io): a root skill, short enough to load up front, and deeper ones
//! loaded when needed: one for each app with a guide (its instructions and operations, from the same declarations its
//! tools come from), the Vault's, and a few areas of their own (`skills/`). One source, every way out: the agents
//! sidecar writes them into each harness's skills folder, `<cli> mcp` serves them (Skills over MCP, SEP-2640), a
//! device serves the root's body as `/llms.txt` and each skill's files at `/skills/<name>/<path>`, and `<cli> skills`
//! prints them. MCP's instructions are the root's short version.

use serde_json::{Value, json};

use crate::config::{CLI, NAME, REPO};

/// Claude Code keeps only the first 4,096 characters (UTF-16 units) of an MCP server's instructions; stay under them.
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
        "Never ask for a password, key or token in the chat, and never paste one. `vault_request` gives the person a link to a sheet on their own device, where they pick or type it; `vault_fetch` makes a web request with an item they let agents use, and you never see the secret. If someone pastes one anyway, keep it with `vault_set` and don't repeat it.".to_string(),
        "If something says you aren't allowed, stop and tell the person. Nothing widens access by itself.".to_string(),
        "Post to the board only when the person asked, or when you changed something; never a decision they didn't make.".to_string(),
    ]
    .iter()
    .map(|r| format!("- {r}"))
    .collect::<Vec<_>>()
    .join("\n")
}

/// How to find what's possible, from the command line.
pub fn finding() -> String {
    format!(
        "- Each area has a skill of its own: load the one for what you're doing. `{CLI} skills` lists them and `{CLI} skills <name>` prints one.\n- `{CLI} search \"<words>\"` finds the operations for a task in every app, each with how to call it and its app's guide. `{CLI} <app>` lists an app's operations and its guide; `{CLI} <app> <op> --help` says what one takes.\n- Every MCP tool is also a command: `board_post` is `{CLI} board post`.\n- `{CLI} run <file>` runs a short script that calls several operations and prints what it returns: `{CLI} search` gives each call as the script writes it.\n- `{CLI} status` says who you are, and which devices are in the install."
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
        "On a device where `{CLI}` runs, MCP is a command there, one of two entries:\n\n- `{CLI} mcp`: every tool, two hundred or so, and `search`. For agents that already find tools themselves or run code with them: Claude Code, OpenCode, Codex, Cursor. In Claude Code: `claude mcp add --scope user {server} -- {CLI} mcp`\n- `{CLI} mcp --code`: a few tools. `search` finds what's possible, `execute` runs a short script with it, and {} stay tools of their own. For every other agent.\n\nPick the one that fits you, or tell the person in one line which you're adding and why. On their devices, {NAME} adds the right one to Claude Code, Codex, OpenCode and Amp by itself.",
        crate::config::ALWAYS_ON.iter().map(|a| format!("{a}'s tools")).chain(["`vault_request`".to_string()]).collect::<Vec<_>>().join(" and ")
    )
}

/// How to get the command onto a device, for this white label.
fn install() -> String {
    match REPO.is_empty() {
        false => format!("install it with `curl -fsSL https://github.com/{REPO}/releases/latest/download/install.sh | sh`"),
        true => format!("ask whoever runs their install to set {NAME} up on it"),
    }
}

/// Connecting for a person, at the install's `address` where it's known: a shell takes the command line, anything
/// else adds the MCP server at the address.
pub fn connecting(address: Option<&str>) -> String {
    let mcp = match address {
        Some(a) => format!("`{}/mcp`", web(a)),
        None => format!("their {NAME}'s web address followed by `/mcp`, like `https://{CLI}.example.com/mcp`"),
    };
    format!(
        "When someone asks you to connect to {NAME}, do it yourself, and tell them only what they need to do, one step at a time, in everyday words.\n\n- If you can run commands (Claude Code, Codex, OpenCode, Cursor): run `{CLI} status`. If it answers, you're on one of their devices already: use the command line, and add MCP as below. If there's no `{CLI}`, {}, then ask them for a pairing code (Settings, then Devices) and run `{CLI} join <code>`.\n- If you can't (ChatGPT, Claude and other chat apps): add {mcp} as a connector, a custom MCP server. They sign in when it asks, and you act as them.",
        install()
    )
}

/// An address as a URL: with https:// unless it says otherwise, and no trailing slash.
fn web(address: &str) -> String {
    let address = address.trim().trim_end_matches('/');
    if address.contains("://") { address.to_string() } else { format!("https://{address}") }
}

/// The guide to connecting, the root skill's body without its list of skills: what anyone may read at the install's
/// `address` (`/llms.txt`), since it says nothing of what apps an install has.
pub fn guide(address: Option<&str>) -> String {
    format!(
        "# {NAME}\n\n{}\n\n## Rules\n\n{}\n\n## Connecting for a person\n\n{}\n\n## Adding MCP\n\n{}\n\n## Finding your way\n\n{}\n",
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
    /// Its SKILL.md: the frontmatter, then the body.
    pub fn skill_md(&self) -> String {
        // JSON strings are YAML's double-quoted ones.
        format!("---\nname: {}\ndescription: {}\n---\n\n{}\n", self.name, json!(self.description), self.body.trim_end())
    }

    /// Every file, SKILL.md first, by its path in the skill.
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
        "{NAME}: the person's email, messages, calendar, files, board and more, as apps their agents use. Use whenever they mention {NAME} or its apps, ask you to connect to it, or want a message sent or a key or password used; read it before any of its tools."
    )
}

/// The root skill, pointing to the others, each named by `link` (a skill's name in a harness, a URL in /llms.txt).
pub fn root(others: &[Skill], link: impl Fn(&Skill) -> String) -> Skill {
    let list: Vec<String> = others.iter().map(|s| format!("- {}: {}", link(s), s.description)).collect();
    let body = format!("{}\n## Skills\n\n{}\n", guide(None), list.join("\n"));
    Skill { name: server_name(), description: root_description(), body, files: vec![] }
}

/// The skills under the root, for `apps` (as `GET /v1/apps` gives them, those installed for the person having their
/// operations): the areas of their own, the Vault's, then one for each app with a guide.
pub fn deeper(apps: &[Value]) -> Vec<Skill> {
    let code = match REPO.is_empty() {
        false => format!("https://github.com/{REPO}"),
        true => "ask whoever runs the install where it is".to_string(),
    };
    let fill = |t: &str| t.replace("{{name}}", NAME).replace("{{cli}}", CLI).replace("{{server}}", &server_name()).replace("{{code}}", &code);
    let written = [include_str!("../skills/accounts/SKILL.md"), include_str!("../skills/build/SKILL.md")];
    let mut skills: Vec<Skill> = written
        .into_iter()
        .map(|md| {
            let md = fill(md);
            let (front, body) = md.trim_start_matches("---\n").split_once("\n---\n").unwrap_or(("", &md));
            let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}: "))).unwrap_or("").to_string();
            Skill { name: field("name"), description: field("description"), body: body.trim().to_string(), files: vec![] }
        })
        .collect();
    skills.push(app_skill(&vault_app()));
    skills.push(app_skill(&backups_app()));
    skills.push(app_skill(&groups_app()));
    skills.extend(apps.iter().filter(|a| a["instructions"].as_str().is_some_and(|i| !i.trim().is_empty()) && a["ops"].as_array().is_some_and(|o| !o.is_empty())).map(app_skill));
    skills
}

/// The root and every skill under it, the root's list naming each by its name.
pub fn all(apps: &[Value]) -> Vec<Skill> {
    let deeper = deeper(apps);
    [vec![root(&deeper, |s| format!("`{}`", s.name))], deeper].concat()
}

/// A skill's name, from what it's about: `<server>-<area>`, in the characters a name may have.
fn skill_name(area: &str) -> String {
    let area: String = area.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let name = format!("{}-{}", server_name(), area.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-"));
    name.chars().take(64).collect::<String>().trim_end_matches('-').to_string()
}

/// An app's skill: its guide, then each operation as its tool says it.
pub fn app_skill(app: &Value) -> Skill {
    let (name, title) = (app["name"].as_str().unwrap_or(""), app["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(app["name"].as_str().unwrap_or("")));
    let about = app["description"].as_str().map(|d| d.trim().trim_end_matches('.')).filter(|d| !d.is_empty()).map(|d| format!(": {d}")).unwrap_or_default();
    let description: String = format!("{title} in {NAME}{about}. Use when the person asks about {title}, and before using its tools ({name}_...).").chars().take(1024).collect();
    let ops: Vec<&Value> = app["ops"].as_array().into_iter().flatten().filter(|o| !o["name"].as_str().unwrap_or("").starts_with('_')).collect();
    // The guide names operations as tools; a harness with only the command line reads each as its command too.
    let mut guide = app["instructions"].as_str().unwrap_or("").trim().to_string();
    let mut by_length: Vec<&str> = ops.iter().filter_map(|o| o["name"].as_str()).collect();
    by_length.sort_by_key(|o| std::cmp::Reverse(o.len()));
    for op in by_length {
        let tool = format!("{name}_{op}");
        let mut out = String::new();
        let mut rest = guide.as_str();
        while let Some(i) = rest.find(&tool) {
            let after = &rest[i + tool.len()..];
            let whole = !rest[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '`' || c == '-') && !after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '`');
            out.push_str(&rest[..i]);
            out.push_str(&if whole { format!("`{CLI} {name} {op}` ({tool})") } else { tool.clone() });
            rest = after;
        }
        out.push_str(rest);
        guide = out;
    }
    let mut body = format!(
        "# {title}\n\n{}\n\n## Operations\n\nEach is the command `{CLI} {name} <op>` (`--help` says more) and the MCP tool `{name}_<op>`. Never ask for a password, key or token in the chat: `{CLI} vault request` (vault_request) gives the person a link. What sends or can't be undone takes `confirm: true` (`--confirm`), given only once the person agreed to that very action. Post to the board only when the person asked, or when you changed something.\n",
        guide,
    );
    for op in ops {
        let mark = match (op["reads"] == true, op["confirm"] == true) {
            (true, _) => " It only reads.",
            (_, true) => " Needs `confirm: true`.",
            _ => "",
        };
        let o = op["name"].as_str().unwrap_or("");
        body.push_str(&format!("\n- `{}` · `{name}_{o}({})`: {}{mark}", command(name, op), ts(&op["input"]), op["description"].as_str().unwrap_or("").trim()));
    }
    Skill { name: skill_name(name), description, body, files: vec![] }
}

/// An operation as a command: `one board post [<topic>] <text>`, its words as --help shows them, then its other
/// required inputs as flags.
fn command(app: &str, op: &Value) -> String {
    let required: Vec<&str> = op["input"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let words: Vec<&str> = match op["words"].as_array() {
        Some(w) => w.iter().filter_map(Value::as_str).collect(),
        None => required.clone(),
    };
    let mut line = format!("{CLI} {app} {}", op["name"].as_str().unwrap_or(""));
    for w in &words {
        line.push_str(&match required.contains(w) {
            true => format!(" <{}>", w.replace('_', "-")),
            false => format!(" [<{}>]", w.replace('_', "-")),
        });
    }
    for r in required.iter().filter(|r| !words.contains(r)) {
        line.push_str(&format!(" --{} <{}>", r.replace('_', "-"), r.replace('_', "-")));
    }
    line
}

/// How the Vault's tools are explained, as an app's guide is.
const VAULT_GUIDE: &str = "The Vault keeps the person's keys, sign-ins and accounts as items, each a page's (theirs, a group's, everyone's): fields (secret ones never read back), the sites they're sent to, and the apps switched on for it. Never ask the person to paste a secret (an API key, a password, a token) into the chat, and never paste one yourself. To keep one, call vault_request and give them its link: they pick one of their items or type a new one into a sheet on their own device, and you never see it. To use one yourself, ask leaving out app, then call vault_fetch with {{<field>}} where it goes: your device fills it in, only for the item's sites, and you get the answer. If someone pastes a secret anyway, keep it with vault_set and don't repeat it. vault_list shows what's there.";

/// The Vault's tools as an app's operations, as search, scripts and its skill see them.
pub fn vault_app() -> Value {
    node_app("vault", "Vault", "Keys, sign-ins and accounts, used without being seen", VAULT_GUIDE, crate::vault::tools())
}

/// How backups' tools are explained, as an app's guide is.
const BACKUPS_GUIDE: &str = "Each device backs up the install's data it holds, every hour that something changed, to each place: a folder on a device's disk, or S3-compatible storage. Everything is encrypted before it leaves, with keys only your devices and the recovery key open. To add S3 storage, call backups_add_place with its endpoint and give the person the link: they type the key on their own device. Snapshots are kept hourly for 2 days, daily for a month and monthly for a year. backups_status says how it's going, and what's wrong and its fix if anything is; backups_check restores the newest backup into a throwaway copy and says what came back.";

/// Backups' tools as an app's operations.
pub fn backups_app() -> Value {
    node_app("backups", "Backups", "Encrypted copies of everything, to a disk or S3-compatible storage", BACKUPS_GUIDE, crate::backups::tools())
}

/// How the groups' tool is explained, as an app's guide is.
const GROUPS_GUIDE: &str = "Groups are the people who share a page, its apps and its data, like a family or a team. groups_list says who's in each, as what (admin or member), and who's invited but hasn't joined.";

/// The groups' tool as an app's operations.
pub fn groups_app() -> Value {
    node_app("groups", "Groups", "Who's in each group, as admin or member, and who's invited", GROUPS_GUIDE, groups_tools())
}

/// The groups' MCP tools.
pub fn groups_tools() -> Vec<Value> {
    vec![json!({ "name": "groups_list", "description": "Every group: who's in it, as admin or member, and who's invited but hasn't joined yet.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } })]
}

/// The node's own tools, shown as an app's operations are, to search, scripts and skills.
pub fn node_app(name: &str, title: &str, description: &str, guide: &str, tools: Vec<Value>) -> Value {
    json!({
        "name": name, "title": title, "description": description, "instructions": guide,
        "ops": tools.into_iter().map(|t| {
            let confirm = t["annotations"]["destructiveHint"] == true;
            let mut input = t["inputSchema"].clone();
            if confirm {
                input["properties"]["confirm"] = json!({ "type": "boolean", "description": "true once the person has agreed: this can't be undone" });
            }
            json!({ "name": t["name"].as_str().unwrap_or("").trim_start_matches(&format!("{name}_")), "description": t["description"], "input": input, "reads": t["annotations"]["readOnlyHint"] == true, "confirm": confirm })
        }).collect::<Vec<_>>(),
    })
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

/// An operation's path in a code mode script: `tools.gmail.search`, or `tools["google-calendar"].events`.
pub fn path(app: &str, op: &str) -> String {
    let key = |k: &str| match k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit()) {
        true => format!(".{k}"),
        false => format!("[{k:?}]"),
    };
    format!("tools{}{}", key(app), key(op))
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
        Mode::Code => "`search` finds the operations for a task across every app, with exact call signatures and each app's guide. `execute` runs a short script with them: `const r = await tools.<app>.<op>({ ... }); return r`. Scripts call several in one go; what they return comes back to you. Guides name operations <app>_<op>: in a script, that's tools.<app>.<op>.".to_string(),
    };
    let mut text = format!(
        "{}\n\nRules, before anything else:\n{}\n\n{finding} Each app's guide is also a skill, `{}-<app>`, with one for the whole of {NAME}, `{}`: load them from your skills, or `{CLI} skills`.",
        about(),
        rules(),
        server_name(),
        server_name()
    );
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

/// Where this device serves the guide to the other devices on its tailnet, once it does.
static ADDRESS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The guide's address on the tailnet, `http://<address>:<port>/llms.txt`, if this device serves it there.
pub fn address() -> Option<String> {
    ADDRESS.lock().unwrap().clone()
}

pub fn set_address(address: Option<String>) {
    *ADDRESS.lock().unwrap() = address;
}
