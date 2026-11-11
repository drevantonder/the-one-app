//! `logs <device>` on the command line, which asks the tool `logs` (`super::tools`).

use serde_json::{Value, json};

use ::api::cli::Command;

pub fn commands() -> Command {
    Command {
        help: vec![("logs <device> [--service <name>] [--since <10m>] [--grep <text>] [--lines <n>]".into(), "a device's node log, or one of its services', or an app's (what its page and app say went wrong, while it's open), read over the network, newest last, capped (the last 200 lines, at most 1000 or 48 KiB); only its user and the network's admins read them. --since takes s, m, h or d".into())],
        run: |g| {
            let words = g.words();
            let ["logs", device] = words.as_slice() else { return Ok(false) };
            g.takes_only(&["service", "since", "grep", "lines"])?;
            let schema = super::tools::feature().tools.remove(0)["input"].clone();
            let mut input = ::api::tools::input(&schema, &[], vec![], g.flags.clone(), &[]).map_err(|e| ::api::cli::Failed::Typed(format!("{e:#}")))?;
            input.insert("device".into(), json!(device));
            let said = g.node.post_for(g.agent, "/api/logs/logs", Value::Object(input)).map_err(|e| crate::tools::in_cli_error(g.node, e))?;
            println!("{}", said["said"].as_str().unwrap_or(""));
            Ok(true)
        },
    }
}
