//! What a tool gives back, in words for a user at the command line, whatever the tool: text as it is, a list one line
//! for each, an object a line for each of what it says. `--json` gives it as it came.

use serde_json::{Map, Value};

/// `ok` in words.
pub fn said(ok: &Value) -> String {
    match ok {
        Value::Array(items) if items.is_empty() => "Nothing.".into(),
        Value::Array(items) => items.iter().map(line).collect::<Vec<_>>().join("\n"),
        Value::Object(o) => o.iter().filter(|(_, v)| !empty(v)).map(|(k, v)| match v {
            Value::Array(items) if items.iter().any(Value::is_object) => format!("{k}:\n{}", items.iter().map(|i| format!("  {}", line(i))).collect::<Vec<_>>().join("\n")),
            v => format!("{k}: {}", inline(k, v)),
        }).collect::<Vec<_>>().join("\n"),
        v => inline("", v),
    }
}

/// One thing on a line: what it's called first, then the rest of what it says, those with nothing to say left out.
fn line(v: &Value) -> String {
    let Value::Object(o) = v else { return inline("", v) };
    let (head, rest) = headed(o);
    let rest: Vec<String> = rest.iter().map(|(k, v)| field(k, v)).collect();
    [head.into_iter().collect::<Vec<_>>(), rest].concat().join("  ")
}

/// An object's name (its title, name or id) and its other fields with something to say.
fn headed(o: &Map<String, Value>) -> (Option<String>, Vec<(&String, &Value)>) {
    let head = ["title", "name", "id"].into_iter().find(|k| o.get(*k).is_some_and(Value::is_string));
    let rest = o.iter().filter(|(k, v)| Some(k.as_str()) != head && !empty(v)).collect();
    (head.and_then(|k| o[k].as_str()).map(str::to_string), rest)
}

/// A field: `key: value`, or for one that's simply so, its key alone (`secret`).
fn field(k: &str, v: &Value) -> String {
    match v {
        Value::Bool(true) => k.to_string(),
        v => format!("{k}: {}", inline(k, v)),
    }
}

/// A value inside a line: a list joined, a thing's fields in brackets after its name.
fn inline(key: &str, v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_u64().filter(|ms| moment(key, *ms)) {
            Some(ms) => crate::feature::say_time(ms),
            None => n.to_string(),
        },
        Value::Array(items) => items.iter().map(|i| inline(key, i)).collect::<Vec<_>>().join(", "),
        Value::Object(o) => {
            let (head, rest) = headed(o);
            let rest = rest.iter().map(|(k, v)| field(k, v)).collect::<Vec<_>>().join(", ");
            match (head, rest.is_empty()) {
                (Some(h), true) => h,
                (Some(h), false) => format!("{h} ({rest})"),
                (None, _) => rest,
            }
        }
        other => other.to_string(),
    }
}

/// Whether a number is a moment, in ms since 1970, as a key that says when names it.
fn moment(key: &str, ms: u64) -> bool {
    (1_000_000_000_000..10_000_000_000_000).contains(&ms) && ["time", "at", "since", "when"].iter().any(|w| key == *w || key.ends_with(&format!("_{w}")))
}

/// What says nothing: null, false, and empty text, lists and objects.
fn empty(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}
