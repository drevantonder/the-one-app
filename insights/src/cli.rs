//! `insights` on the command line, which asks the tool `insights` (`super::tools`).

use serde_json::Value;

use ::api::cli::Command;

pub fn commands() -> Command {
    Command {
        help: vec![("insights [--service <name>] [--trace-id <id>] [--since <1h>] [--limit <n>] [--status error|unset] [--effect-of <build>] [--raw [--name <name>] [--min-ms <n>] [--parts requests|spans|logs|metrics|errors|traces]...] [--check]".into(), "how things are going, as lines to read: each service's requests, errors and P95 and what broke most, a service's operations, failing or slow traces and readings (--service), a trace's spans (--trace-id), what a build changed against the one before it (--effect-of), or the rows as JSON (--raw): traces are the newest with a span of --service, --name, --status and --min-ms, whole. --status unset lists what didn't break, as what a caller had refused. --parts takes one kind of row, given again for each. --check checks telemetry's own path first, as it's checked every 15 minutes. --since takes s, m, h or d".into())],
        run: |g| {
            let words = g.words();
            let ["insights"] = words.as_slice() else { return Ok(false) };
            g.takes_only(&["service", "trace-id", "effect-of", "parts", "raw", "name", "min-ms", "since", "status", "limit", "check"])?;
            let schema = super::tools::feature().tools.remove(0)["input"].clone();
            let input = ::api::tools::input(&schema, &[], vec![], g.flags.clone(), &[]).map_err(|e| ::api::cli::Failed::Typed(format!("{e:#}")))?;
            let ran = g.node.route(g.agent, "/api/insights/insights", &Value::Object(input), g.trace);
            let said = ran.ok.inspect_err(|_| ::api::cli::print_trace(ran.trace.as_deref()))?;
            match &said {
                Value::String(text) => println!("{text}"),
                rows => println!("{}", serde_json::to_string_pretty(rows)?),
            }
            ::api::cli::print_trace(ran.trace.as_deref());
            Ok(true)
        },
    }
}
