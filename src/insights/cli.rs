//! `insights` on the command line, which asks the tool `insights` (`super::tools`).

use serde_json::Value;

use crate::cli::Command;

pub fn commands() -> Command {
    Command {
        help: vec![("insights [--app <name>] [--name <name>] [--since <1h>] [--status ok|error] [--fault caller|internal] [--limit <n>]".into(), "how things went: recent signals, hourly rollups and error fingerprints, of what you may see, as JSON. --since takes s, m, h or d".into())],
        run: |g| {
            let words = g.words();
            let ["insights"] = words.as_slice() else { return Ok(false) };
            g.takes_only(&["app", "name", "since", "status", "fault", "limit"])?;
            let schema = super::tools::feature().tools.remove(0).spec["input"].clone();
            let input = crate::tools::input(&schema, &[], vec![], g.flags.clone(), &[]).map_err(|e| crate::cli::Failed::Typed(format!("{e:#}")))?;
            let said = g.node.post_for(g.agent, "/api/insights/insights", Value::Object(input)).map_err(|e| crate::tools::in_cli_error(g.node, e))?;
            println!("{}", serde_json::to_string_pretty(&said)?);
            Ok(true)
        },
    }
}
