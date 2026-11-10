//! `insights` on the command line, which asks the tool `insights` (`super::tools`).

use serde_json::Value;

use crate::cli::Command;

pub fn commands() -> Command {
    Command {
        help: vec![("insights [--service <name>] [--trace-id <id>] [--since <1h>] [--limit <n>] [--status error|unset] [--raw [--name <name>]] [--check]".into(), "how things are going, as lines to read: each service's requests, errors and P95 and what broke most, a service's operations and failing or slow traces (--service), a trace's spans (--trace-id), or the rows as JSON (--raw). --status unset lists what didn't break, as what a caller had refused. --check checks telemetry's own path first, as it's checked every 15 minutes. --since takes s, m, h or d".into())],
        run: |g| {
            let words = g.words();
            let ["insights"] = words.as_slice() else { return Ok(false) };
            g.takes_only(&["service", "trace-id", "raw", "name", "since", "status", "limit", "check"])?;
            let schema = super::tools::feature().tools.remove(0).spec["input"].clone();
            let input = crate::tools::input(&schema, &[], vec![], g.flags.clone(), &[]).map_err(|e| crate::cli::Failed::Typed(format!("{e:#}")))?;
            let said = g.node.post_for(g.agent, "/api/insights/insights", Value::Object(input)).map_err(|e| crate::tools::in_cli_error(g.node, e))?;
            match &said {
                Value::String(text) => println!("{text}"),
                rows => println!("{}", serde_json::to_string_pretty(rows)?),
            }
            Ok(true)
        },
    }
}
