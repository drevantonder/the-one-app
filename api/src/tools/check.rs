//! Checking an input against its tool's schema, and what to say when a name is wrong: its nearest.

use anyhow::{Result, bail, ensure};
use serde_json::Value;

use super::input::{required, takes};

/// Checks a tool's input against its schema before anything runs: types, choices, required inputs, and no
/// inputs it doesn't take. What's wrong names the input (`limit takes a whole number, not "many"`).
pub fn check(schema: &Value, input: &Value) -> Result<()> {
    check_at(schema, input, "")
}

fn check_at(schema: &Value, v: &Value, at: &str) -> Result<()> {
    let named = if at.is_empty() { "the input" } else { at };
    if let Some(choices) = schema["enum"].as_array() {
        ensure!(choices.contains(v), "{named} is one of {}, not {v}", choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", "));
    }
    let types: Vec<&str> = match &schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    let is = |t: &str| match t {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    };
    if !types.is_empty() && !types.iter().any(|t| is(t)) {
        let want = types.iter().map(|t| match *t { "string" => "text", "integer" => "a whole number", "number" => "a number", "boolean" => "true or false", "array" => "a list", "object" => "an object", t => t }).collect::<Vec<_>>().join(" or ");
        bail!("{named} takes {want}, not {v}");
    }
    if let (Some(items), Some(list)) = (schema.get("items"), v.as_array()) {
        for (i, x) in list.iter().enumerate() {
            check_at(items, x, &format!("{named}[{i}]"))?;
        }
    }
    if let Some(o) = v.as_object() {
        let props = schema["properties"].as_object().cloned().unwrap_or_default();
        let inner = |k: &str| if at.is_empty() { k.to_string() } else { format!("{at}.{k}") };
        for r in required(schema) {
            if o.get(&r).is_none_or(Value::is_null) {
                let about = props.get(&r).and_then(|p| p["description"].as_str()).map(|d| format!(": {d}")).unwrap_or_default();
                bail!("missing {}{about}", inner(&r));
            }
        }
        for (k, x) in o {
            match props.get(k) {
                Some(_) if x.is_null() => {}
                Some(p) => check_at(p, x, &inner(k))?,
                // A tool's own inputs are all listed, unless it says it takes any; deeper objects may hold more,
                // unless they say not.
                None if (at.is_empty() && !takes(schema, k)) || schema["additionalProperties"] == false => {
                    let names: Vec<&str> = props.keys().map(String::as_str).collect();
                    let near = nearest(k, &names);
                    match (names.is_empty(), near.first()) {
                        (true, _) => bail!("{} isn't an input: this takes none", inner(k)),
                        (_, Some(n)) => bail!("{} isn't an input: did you mean {n}? It takes {}", inner(k), names.join(", ")),
                        (_, None) => bail!("{} isn't an input: it takes {}", inner(k), names.join(", ")),
                    }
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// The few of `names` nearest to `word`, nearest first: a typo's fix, by edit distance (a swap of two letters is one
/// edit), within one edit for a short word and up to three for a long one.
pub fn nearest(word: &str, names: &[&str]) -> Vec<String> {
    let w: Vec<char> = word.to_lowercase().chars().collect();
    let most = match w.len() { 0..=3 => 1, 4..=6 => 2, _ => 3 };
    let mut found: Vec<(usize, &str)> = names.iter().map(|n| (distance(&w, &n.to_lowercase().chars().collect::<Vec<_>>()), *n)).filter(|(d, _)| *d <= most).collect();
    found.sort();
    found.into_iter().take(3).map(|(_, n)| n.to_string()).collect()
}

/// Edits from `a` to `b`: a letter added, taken out, changed, or two side by side swapped.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}
