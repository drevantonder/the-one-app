//! The Vault's commands besides its tools: `vault requests` and `vault answer`.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::cli::{Command, Given};

/// The user's own answer to a request for an item, with its values on stdin: not a tool, since an agent that answered
/// one would see the secret vault_request keeps from it.
pub fn commands() -> Command {
    Command {
        help: vec![("vault requests".into(), "the requests waiting for you; vault answer <id> [--item <id> | --decline] answers one, a new item's values on stdin (one line, or a JSON object)".into())],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["vault", "requests"] => {
            g.takes_only(&[])?;
            for r in g.node.get("/v1/vault/requests")?.as_array().into_iter().flatten() {
                println!("{}  {} for {} in {}  {}  {}", r["id"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope_name"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["why"].as_str().unwrap_or(""));
            }
        }
        ["vault", "answer", id] => {
            g.takes_only(&["scope", "decline", "item"])?;
            let scope = g.flag("scope").unwrap_or("user");
            let mut body = json!({ "scope": scope, "id": id });
            if g.has("decline") {
                body["declined"] = json!(true);
            } else if let Some(item) = g.flag("item") {
                body["item"] = json!(item);
            } else {
                let asked = g.node.get(&format!("/v1/vault/requests?id={id}"))?;
                let r = asked.as_array().and_then(|a| a.first()).with_context(|| format!("no request {id}"))?;
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                let text = text.trim_end_matches(['\n', '\r']);
                body["fields"] = match serde_json::from_str::<Value>(text) {
                    Ok(v) if v.is_object() => v,
                    _ => json!({ r["fields"][0]["name"].as_str().unwrap_or(""): text }),
                };
            }
            g.node.post("/v1/vault/answer", body)?;
            println!("Answered.");
        }
        _ => return Ok(false),
    }
    Ok(true)
}
