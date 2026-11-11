//! `mcp` on the command line: the MCP server (`api_features::mcp`) over stdio, for an agent on this machine, asking its onet as
//! a client does.

use anyhow::Result;
use serde_json::{Value, json};

use api::Client;
use api_features::mcp::{Session, agent_name, answer};
use api_features::skills::Mode;

/// `mcp` on the command line: this server, over stdio.
pub fn commands() -> ::api::cli::Command {
    ::api::cli::Command {
        help: vec![("mcp [--code]".into(), "the local MCP server, over stdio, for agents: every tool, or with --code, search and execute".into())],
        run: |g| {
            if g.words() != ["mcp"] {
                return Ok(false);
            }
            stdio(g.node.again(), if g.has("code") { Mode::Code } else { Mode::All })?;
            Ok(true)
        },
    }
}

/// Serves MCP over stdio, asking `onet`, for an agent on this machine.
pub fn stdio(onet: Client, mode: Mode) -> Result<()> {
    use std::io::BufRead;
    let mut session = Session { agent: agent_from_env(), mode, stdio: true };
    for line in std::io::stdin().lock().lines() {
        let Ok(msg) = serde_json::from_str::<Value>(&line?) else { continue };
        let onet = onet.again();
        // A tool call is answered beside the rest, which go on meanwhile: some wait (a chat's view waiting on a change).
        if msg["method"] == "tools/call" {
            let mut session = Session { agent: session.agent.clone(), ..session };
            std::thread::spawn(move || say(answer(&onet, &msg, &mut session)));
            continue;
        }
        say(answer(&onet, &msg, &mut session))?;
    }
    Ok(())
}

/// Writes an answer, and that the tools listed changed when they did.
fn say(answered: Option<(Value, bool)>) -> std::io::Result<()> {
    use std::io::Write;
    let Some((reply, changed)) = answered else { return Ok(()) };
    let mut out = std::io::stdout().lock();
    writeln!(out, "{reply}")?;
    if changed {
        writeln!(out, "{}", json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }))?;
    }
    out.flush()
}

/// The agent running this program, if one is: `<CLI>_AGENT`, else what agent harnesses say about themselves.
pub fn agent_from_env() -> Option<String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    if let Some(a) = var(&::custom_build::config::env("AGENT")) {
        return Some(a);
    }
    // AI_AGENT is `<name>_<version>_<kind>`, like claude-code_2-1-314_agent.
    if let Some(a) = var("AI_AGENT") {
        return Some(agent_name(a.split('_').next().unwrap_or(&a)));
    }
    let named = [("CLAUDECODE", "Claude Code"), ("CODEX_THREAD_ID", "Codex"), ("CODEX_SANDBOX", "Codex"), ("OPENCODE", "OpenCode"), ("AMP_THREAD_ID", "Amp")];
    named.iter().find(|(k, _)| var(k).is_some()).map(|(_, n)| n.to_string())
}
