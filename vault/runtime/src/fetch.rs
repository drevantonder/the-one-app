//! `vault_fetch`: an agent's web request, with the items its user lets agents use filled in.

use anyhow::{Context, Result, bail, ensure};
use api::Caller;
use api::runtime::prelude::*;
use scope::scopes::{self, Scope};
use serde_json::{Value, json};
use vault::Lookup;

use crate::ASK_FOR_AGENTS;
use crate::items::lookup;

/// Whether `host` is the name of an app or service, which an item's hosts name to let its fields into calls to that
/// service: never a site's, so a web request to it gets none.
pub fn names_app(rt: &Rt, host: &str) -> bool {
    rt.read_or(false, |r| Ok(r.db().app(host)?.is_some() || r.db().switches()?.keys().any(|(_, name)| name == host)))
}

/// The scopes `user` may use that this device holds: where an agent's request and a service's are filled from.
fn usable_scopes(rt: &Rt, user: &str) -> Result<Vec<Scope>> {
    rt.read(|r| Ok(r.db().may_scopes(user, "use")?.into_iter().filter(|s| r.holds(s)).collect()))
}

/// Makes an HTTP request for an agent of `caller`'s (`{ url, method?, headers?, body? }`), as the built-in agents
/// service: the fields of the items a user lets agents use are filled in, a secret one only for its item's hosts, as an
/// app's requests are. → `{ status, headers, body }`, a secret field's value blotted out wherever the site echoes it
/// back, so the agent never sees one.
pub(crate) async fn fetch(rt: &Rt, caller: &Caller, req: &Value) -> Result<Value> {
    let url = req["url"].as_str().context("give url")?;
    let uri: ureq::http::Uri = url.parse().with_context(|| format!("{url} isn't a web address"))?;
    ensure!(matches!(uri.scheme_str(), Some("https" | "http")), "only http and https addresses");
    let host = uri.host().context("the address has no host")?.trim_start_matches('[').trim_end_matches(']').to_lowercase();
    ensure!(!names_app(rt, &host), "{host} is an app's name, not a site's");
    let lookup = lookup(rt, caller, scopes::AGENTS, &usable_scopes(rt, &caller.user)?).await?;
    let used = std::sync::Mutex::new(Vec::<String>::new());
    let missing = std::sync::Mutex::new(Vec::<String>::new());
    let fill = |text: &str| {
        vault::fill(text, &host, |name| {
            let found = lookup.get(name);
            match &found {
                Some((hosts, value)) if !hosts.is_empty() => used.lock().unwrap().push(value.clone()),
                Some(_) => {}
                None => missing.lock().unwrap().push(name.to_string()),
            }
            found
        })
    };
    lookup.refuse_shut(&req.to_string())?;
    let filled = fill(url)?;
    let mut headers = Vec::new();
    for (name, value) in req["headers"].as_object().into_iter().flatten() {
        headers.push((name.clone(), fill(value.as_str().context("a header's value is text")?)?.into_bytes()));
    }
    let body = match &req["body"] {
        Value::Null => Vec::new(),
        Value::String(text) => fill(text)?.into_bytes(),
        other => fill(&other.to_string())?.into_bytes(),
    };
    // A placeholder left as it is would go to the site as it is: say why it wasn't filled instead, sending nothing.
    if let Some(name) = missing.into_inner().unwrap().first() {
        bail!("Not sent: {}", why_unfilled(&lookup, name));
    }
    let method = req["method"].as_str().unwrap_or(if body.is_empty() { "GET" } else { "POST" }).to_uppercase();
    let ports = rt.own_ports();
    let (status, headers, bytes) = tokio::task::spawn_blocking(move || platform::reach::send(&method, &filled, headers, body, ports)).await??;
    let secrets = used.into_inner().unwrap();
    let blot = |text: &str| secrets.iter().filter(|s| !s.is_empty()).fold(text.to_string(), |t, s| t.replace(s.as_str(), "••••"));
    let headers: serde_json::Map<String, Value> = headers.iter().map(|(n, v)| (n.clone(), json!(blot(&String::from_utf8_lossy(v))))).collect();
    const SHOWN: usize = 1 << 20;
    let cut = bytes.len() > SHOWN;
    let shown = &bytes[..bytes.len().min(SHOWN)];
    let mut out = json!({ "status": status, "headers": headers });
    match std::str::from_utf8(shown) {
        Ok(text) => out["body"] = json!(blot(text)),
        Err(_) => out["body_base64"] = json!(data_encoding::BASE64.encode(shown)),
    }
    if cut {
        out["cut"] = json!(format!("only the first {SHOWN} of {} bytes", bytes.len()));
    }
    Ok(out)
}

/// Why an agent's placeholder `name` found nothing in the items it was filled from (`lookup`), and what to do, in a
/// sentence.
fn why_unfilled(lookup: &Lookup, name: &str) -> String {
    let ask = format!("{ASK_FOR_AGENTS} asks the user to let you");
    let found = name.split_once(':').map(|(id, field)| (id, field, lookup.scopes.iter().find_map(|(_, items)| items.get(id).cloned())));
    match found {
        Some((id, _, None)) => format!("no item {id} is in a Vault you may use: vault_list shows them, and {ask} use one"),
        Some((_, _, Some(it))) if !it.apps.contains(scopes::AGENTS) => format!("agents may not use {} yet: {ask}", it.title),
        Some((_, field, Some(it))) => {
            let fields: Vec<&str> = it.fields.keys().map(String::as_str).collect();
            format!("{} has no field {field}; it has {}", it.title, fields.join(", "))
        }
        None => {
            let field = name.rsplit('/').next().unwrap_or(name);
            format!("no item agents may use has a field {field}: {ASK_FOR_AGENTS} asks the user for one")
        }
    }
}
