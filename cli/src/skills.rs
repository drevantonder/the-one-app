//! `skills` on the command line: the guides agents load (`one::skills`).

use custom_build::config::{CLI, NAME};
use one::skills::server_name;

/// `skills` on the command line: the list, or one skill's file.
pub fn commands() -> ::api::cli::Command {
    ::api::cli::Command {
        help: vec![("skills [<name> [<file>]]".into(), format!("the skills agents load: {NAME}'s own (how to connect, the rules, which MCP to add; also /llms.txt), and each area's and app's"))],
        run: |g| {
            match g.words().as_slice() {
                ["skills"] => {
                    let skills = one::mcp::skills(g.node);
                    let width = skills.iter().map(|s| s.name.len()).max().unwrap_or(0);
                    for s in &skills {
                        println!("{:width$}  {}", s.name, s.description);
                    }
                    println!("\n{CLI} skills <name> prints one; {CLI} skills <name> <file> one of its other files.");
                }
                ["skills", name, path @ ..] => {
                    let skill = one::mcp::skills(g.node).into_iter().find(|s| s.name == *name || s.name.strip_prefix(&format!("{}-", server_name())) == Some(*name));
                    let skill = skill.ok_or_else(|| ::api::cli::Failed::Typed(format!("no skill {name}: {CLI} skills lists them")))?;
                    let path = path.first().copied().unwrap_or("SKILL.md");
                    print!("{}", anyhow::Context::with_context(skill.file(path), || format!("{} has no {path}: it has {}", skill.name, skill.all_files().iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join(", ")))?);
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
