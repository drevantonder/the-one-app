//! Secrets in requests: an app writes `{{name}}` where a secret goes, and the node fills it in on the way out, only
//! for the secret's hosts. See SPEC.md, "Secrets in calls".

use anyhow::{Result, bail};

/// Fills each `{{name}}` (or `{{<group id>/name}}`) in `text` with the secret `lookup` finds by that name, as
/// `(hosts, value)`; refuses when that secret isn't sent to `host`. A placeholder no secret has is left as it is.
pub fn fill(text: &str, host: &str, lookup: impl Fn(&str) -> Option<(Vec<String>, String)>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push_str(&rest[..start]);
        let name = &after[..end];
        match placeholder(name).then(|| lookup(name)).flatten() {
            Some((hosts, value)) => {
                if !hosts.iter().any(|h| h == host) {
                    bail!("the secret {name} is sent only to {}, not {host}", hosts.join(", "));
                }
                out.push_str(&value);
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Whether `name` could name a secret: `name`, or `<group id>/name`.
fn placeholder(name: &str) -> bool {
    let name = match name.split_once('/') {
        Some((group, name)) if crate::entry::is_id(group) => name,
        Some(_) => return false,
        None => name,
    };
    crate::members::valid_secret_name(name)
}

#[test]
fn fills_secrets_only_for_their_hosts() {
    let lookup = |name: &str| match name {
        "token" => Some((vec!["api.example.com".to_string()], "s3cret".to_string())),
        "0123456789abcdef/team" => Some((vec!["api.example.com".to_string()], "shared".to_string())),
        _ => None,
    };
    assert_eq!(fill("Bearer {{token}}", "api.example.com", lookup).unwrap(), "Bearer s3cret");
    assert_eq!(fill("{{token}}:{{0123456789abcdef/team}}", "api.example.com", lookup).unwrap(), "s3cret:shared");
    assert!(fill("Bearer {{token}}", "evil.example.com", lookup).is_err(), "never to another host");
    assert_eq!(fill("{{nope}} {{ not one }} {{", "evil.example.com", lookup).unwrap(), "{{nope}} {{ not one }} {{");
}
