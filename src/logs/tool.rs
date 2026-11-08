//! `one logs` and its tool, and what a node's and its services' output goes through to be kept (`super`'s log files).

use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

use super::{Ask, record};
use crate::api::Client;
use crate::config::CLI;

/// What `tracing` writes to: stderr, and the log.
#[derive(Clone, Copy)]
pub struct Tee;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Tee {
    type Writer = Tee;
    fn make_writer(&'a self) -> Tee {
        Tee
    }
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        record(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

/// Keeps what a program says as it says it (a service's, whose output nobody else reads), and passes it to stderr.
pub fn tee(from: impl std::io::Read + Send + 'static) {
    std::thread::spawn(move || {
        let mut from = std::io::BufReader::new(from);
        let mut line = Vec::new();
        while from.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            let text = String::from_utf8_lossy(&line).to_string();
            eprint!("{text}");
            record(&text);
            line.clear();
        }
    });
}

/// Reads a device's log as the tool `logs` does (`ask`), in words.
pub fn run(_: &str, input: &Map<String, Value>, asked: &crate::core_tools::Asked, node: &Client) -> anyhow::Result<Value> {
    let device = input.get("device").and_then(Value::as_str).unwrap_or("");
    let (lines, note) = said(&ask(node, input, asked.agent, asked.reader, false).ok?, device);
    let text: Vec<&str> = lines.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    Ok(json!(format!("{}{}", text.join("\n"), note.map(|n| format!("\n{n}")).unwrap_or_default())))
}

/// A device's log, or one of its services', as `logs` asks for it (`device`, `service`, `since`, `grep`, `lines`): the
/// node there says no unless the caller may remove that device (src/node/calls/running.rs).
pub fn ask(node: &Client, input: &Map<String, Value>, agent: Option<&str>, reader: crate::core_tools::Reader, trace: bool) -> crate::api::client::Called {
    let Some(device) = input.get("device").and_then(Value::as_str) else {
        return crate::api::client::Called { ok: Err(anyhow::anyhow!("which device? {CLI} devices lists them")), trace: None };
    };
    let mut asked = input.clone();
    asked.remove("device");
    asked.remove("trace");
    if let Err(e) = Ask::from_json(&Value::Object(asked.clone())) {
        return crate::api::client::Called { ok: Err(crate::cli::Failed::Typed(format!("{e:#}")).into()), trace: None };
    }
    node.call(&json!({ "device": device, "app": crate::database::scopes::APP, "tool": "logs", "scopes": [], "input": asked, "agent": agent, "client": reader.client() }), trace)
}

/// What a log read gives, as its lines and, if there's something to say about them, a note.
pub fn said(r: &Value, device: &str) -> (Value, Option<String>) {
    let (shown, matched) = (r["shown"].as_u64().unwrap_or(0), r["matched"].as_u64().unwrap_or(0));
    let note = if shown == 0 {
        let from = r["oldest"].as_u64().map(|ms| format!("; its log goes back to {}", crate::core_tools::say_time(ms))).unwrap_or_default();
        Some(format!("{device}'s {} log has no lines like that{from}.", r["log"].as_str().unwrap_or("node")))
    } else if matched > shown {
        Some(format!("{shown} of the {matched} lines that match, the newest: narrow it with --since or --grep."))
    } else {
        None
    };
    (r["lines"].clone(), note)
}

/// `logs <device>` on the command line: what the tool `logs` gives (`ask`), printed.
pub fn commands() -> crate::cli::Command {
    crate::cli::Command {
        help: vec![("logs <device> [--service <name>] [--since <10m>] [--grep <text>] [--lines <n>]".into(), "a device's node log, or one of its services', or an app's (what its page and app say went wrong, while it's open), read over the network, newest last, capped (the last 200 lines, at most 1000 or 48 KiB); only its user and the network's admins read them. --since takes s, m, h or d".into())],
        run: |g| {
            let words = g.words();
            let ["logs", device] = words.as_slice() else { return Ok(false) };
            g.takes_only(&["service", "since", "grep", "lines"])?;
            let mut input = crate::tools::input(&super::tool()["input"], &[], vec![], g.flags.clone(), &[]).map_err(|e| crate::cli::Failed::Typed(format!("{e:#}")))?;
            input.insert("device".into(), json!(device));
            let read = ask(g.node, &input, g.agent, crate::core_tools::Reader::Cli, g.trace);
            if read.ok.is_err() {
                crate::tools::print_trace(read.trace.as_deref());
            }
            let trace = read.trace;
            let (lines, note) = said(&read.ok.map_err(|e| anyhow::anyhow!(crate::tools::in_cli_words(g.node, &format!("{e:#}"))))?, device);
            match g.json {
                true => println!("{}", serde_json::to_string_pretty(&lines)?),
                false => {
                    lines.as_array().into_iter().flatten().filter_map(Value::as_str).for_each(|l| println!("{l}"));
                    note.iter().for_each(|n| eprintln!("{n}"));
                }
            }
            crate::tools::print_trace(trace.as_deref());
            Ok(true)
        },
    }
}
