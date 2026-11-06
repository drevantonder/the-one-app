//! The Vault's commands: `vault`, and `vault <what>`.

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::cli::{Command, Given};
use crate::config::CLI;

pub fn commands() -> Command {
    Command {
        help: vec![
            ("vault [--scope user|global|<group>]".into(), "the Vault's items you may change: fields (secret ones never shown), the sites they're sent to and the apps switched on for each".into()),
            ("vault set <item> <field> [--secret] [--hosts a,b] [--app <app>] [--scope s]".into(), "set a field of an item (by title or id; a new one by that title), its value read from stdin (a line typed, or a whole file); for a person's value, vault request asks them instead".into()),
            ("vault use <item> <app> [--off]".into(), "switch an app on (or off) for an item; only the person lets agents use one".into()),
            ("vault grant <device> [--off] [--scope s]".into(), "let a machine open a Vault's secrets and fill them in (or no longer); only its admins say, and nobody else, not even its people, reads a secret back".into()),
            ("vault fetch <url> [--method M] [--headers <json object>] [--body text]".into(), format!("make a request with {{{{field}}}} filled in from the items the person lets agents use, a secret field only for its item's sites: you get the answer, never the secret")),
            ("vault remove <item> [<field>] [--scope s]".into(), "delete an item, or one of its fields".into()),
            ("vault request [<app>] (--need <name> | --title <t> --fields a,b [--secret-fields b] --hosts a,b) [--why text]".into(), "ask the person for an item an app may use (with no app, for vault fetch): a link to a sheet where they pick or make one, so you never see it; vault status <id> says how it went".into()),
            ("vault requests".into(), "the requests waiting for you; vault answer <id> [--item <id> | --decline] answers one, a new item's values on stdin (one line, or a JSON object)".into()),
        ],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["vault"] | ["vault", "list"] => {
            g.takes_only(&["scope"])?;
            let scope = g.flags.get("scope").and_then(Value::as_str).map(|s| format!("?scope={s}")).unwrap_or_default();
            for it in g.node.get(&format!("/v1/vault{scope}"))?.as_array().into_iter().flatten() {
                println!("{}", super::app::item_line(it));
            }
        }
        ["vault", "set", item, field] => {
            g.takes_only(&["value", "scope", "hosts", "secret", "app"])?;
            let value = match g.flags.get("value").and_then(Value::as_str) {
                Some(v) => v.to_string(),
                None => {
                    // Typed, one line; from a file (a key's PEM), all of it.
                    let mut v = String::new();
                    match std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                        true => std::io::stdin().read_line(&mut v)?,
                        false => std::io::Read::read_to_string(&mut std::io::stdin(), &mut v)?,
                    };
                    v.trim_end_matches(['\n', '\r']).to_string()
                }
            };
            anyhow::ensure!(!value.is_empty(), "give the value on stdin");
            let scope = g.flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let hosts = g.flags.get("hosts").and_then(Value::as_str).map(|h| h.split(',').map(|h| h.trim().to_string()).collect());
            println!("{}", super::app::set(g.node, scope, item, field, &value, g.flags.get("secret").map(|_| true), hosts, g.flags.get("app").and_then(Value::as_str), g.agent)?);
        }
        ["vault", "use", item, app] => {
            g.takes_only(&["scope", "off"])?;
            let scope = g.flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", super::app::switch(g.node, scope, item, app, !g.has("off"), g.agent)?);
        }
        ["vault", "grant", device] => {
            g.takes_only(&["scope", "off"])?;
            let scope = g.flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", g.node.post("/v1/vault/grant", json!({ "scope": scope, "device": device, "on": !g.has("off") }))?["said"].as_str().unwrap_or(""));
        }
        ["vault", "fetch", url] => {
            g.takes_only(&["method", "headers", "body"])?;
            let r = g.node.post("/v1/vault/fetch", json!({ "url": url, "method": g.flags.get("method"), "headers": g.flags.get("headers"), "body": g.flags.get("body"), "agent": g.agent }))?;
            eprintln!("{}", r["status"]);
            match r["body"].as_str() {
                Some(body) => println!("{body}"),
                None => println!("{}", r["body_base64"].as_str().unwrap_or("")),
            }
            if let Some(cut) = r["cut"].as_str() {
                eprintln!("({cut})");
            }
        }
        ["vault", "remove", item] | ["vault", "remove", item, _] => {
            g.takes_only(&["scope", "confirm"])?;
            crate::cli::agreed(g.has("confirm"), &format!("{CLI} vault remove"))?;
            let scope = g.flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            println!("{}", super::app::remove(g.node, scope, item, g.words.get(3).map(String::as_str))?);
        }
        ["vault", "request", app @ ..] if app.len() < 2 => {
            g.takes_only(&["scope", "need", "title", "fields", "secret-fields", "hosts", "why"])?;
            let app = super::request_app(app.first().copied());
            let scope = g.flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let list = |k: &str| g.flags.get(k).and_then(Value::as_str).map(|v| v.split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
            let secret = list("secret-fields");
            let fields: Map<String, Value> = list("fields").into_iter().map(|f| (f.clone(), json!({ "secret": secret.contains(&f) }))).collect();
            let body = json!({ "scope": scope, "app": app, "need": g.flags.get("need"), "title": g.flags.get("title"), "fields": fields, "hosts": list("hosts"), "why": g.flags.get("why"), "agent": g.agent });
            let r = g.node.post("/v1/vault/request", body)?;
            println!("Give the person this link: {}\nTell them; {CLI} vault status {} says how it went once they say they've done it.", r["link"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
        }
        ["vault", "status", id] => {
            g.takes_only(&[])?;
            let r = super::app::request(g.node, id)?;
            println!("{} for {} in {}: {}", r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope_name"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""));
        }
        ["vault", "requests"] => {
            g.takes_only(&[])?;
            for r in g.node.get("/v1/vault/requests")?.as_array().into_iter().flatten() {
                println!("{}  {} for {} in {}  {}  {}", r["id"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), r["app"].as_str().unwrap_or(""), r["scope_name"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["why"].as_str().unwrap_or(""));
            }
        }
        ["vault", "answer", id] => {
            g.takes_only(&["scope", "decline", "item"])?;
            let scope = g.flags.get("scope").and_then(Value::as_str).unwrap_or("user");
            let mut body = json!({ "scope": scope, "id": id });
            if g.has("decline") {
                body["declined"] = json!(true);
            } else if let Some(item) = g.flags.get("item").and_then(Value::as_str) {
                body["item"] = json!(item);
            } else {
                let r = super::app::request(g.node, id)?;
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
