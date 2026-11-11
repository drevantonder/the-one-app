//! Where a frame of built code came from: the app's `app.js.map`, each screen's `<name>.html.map`. What's kept of a map is
//! its files and lines, none of the code; and a frame stays only ever a location.

use serde_json::Value;

/// The map of the file a frame is in: an app's module's from its bundle's `files`, or a screen's, an app's own or, with no
/// bundle, the client's.
pub fn of(file: &str, files: &[(String, Vec<u8>)]) -> Option<String> {
    let in_bundle = |name: &str| files.iter().find(|(path, _)| path == name).and_then(|(_, bytes)| String::from_utf8(bytes.clone()).ok());
    if file == "app.js" {
        return in_bundle(::deploy::bundle::BUILT);
    }
    let screen = file.strip_suffix(".screen.js")?;
    if files.is_empty() {
        return crate::screen::client_map(screen).map(str::to_string);
    }
    in_bundle(&format!("src/dist/{screen}.html.map"))
}

/// What `map` says line `line`, column `col` of the built file came from: a file and where in it, each from 1.
pub fn located(map: &str, line: u64, col: u64) -> Option<(String, u64, u64)> {
    let map: Value = serde_json::from_str(map).ok()?;
    let sources = map["sources"].as_array()?;
    let (mut source, mut from_line, mut from_col) = (0_i64, 0_i64, 0_i64);
    let mut found = None;
    for (at, group) in map["mappings"].as_str()?.split(';').enumerate() {
        let (here, mut built) = (at as u64 + 1 == line, 0_i64);
        for segment in group.split(',').filter(|s| !s.is_empty()) {
            let fields = decoded(segment)?;
            built += fields[0];
            if fields.len() >= 4 {
                (source, from_line, from_col) = (source + fields[1], from_line + fields[2], from_col + fields[3]);
            }
            if here && (built as u64) < col {
                found = (fields.len() >= 4).then_some((source, from_line, from_col));
            }
        }
        if here {
            break;
        }
    }
    let (source, line, col) = found?;
    Some((sources.get(source as usize)?.as_str()?.to_string(), line as u64 + 1, col as u64 + 1))
}

/// A segment's fields, each base-64 VLQ: its low bit the sign, and five bits a digit, low first.
fn decoded(segment: &str) -> Option<Vec<i64>> {
    const DIGITS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut fields, mut value, mut shift) = (Vec::new(), 0_i64, 0);
    for c in segment.chars() {
        let digit = DIGITS.find(c)? as i64;
        value |= (digit & 31) << shift;
        shift += 5;
        if digit & 32 == 0 {
            fields.push(if value & 1 == 1 { -(value >> 1) } else { value >> 1 });
            (value, shift) = (0, 0);
        }
    }
    (!fields.is_empty()).then_some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_frame_resolves_to_where_it_was_written() {
        // Bun's own map, as `deploy::bundle::source_map` leaves it, of lib/f.js's `x.length` and app.js's `f(null)`.
        let map = serde_json::to_string(&json!({ "sources": ["lib/f.js", "app.js"], "mappings": ";AACO,SAAS,CAAC,CAAC,GAAG;AAAA,EACnB,OAAO,EAAE;AAAA;;;ACDX,IAAe,gBAAE,KAAK,GAAE;AAAA,EAAE,OAAO,EAAE,IAAI;AAAA,EAAI;" })).unwrap();
        assert_eq!(located(&map, 3, 12), Some(("lib/f.js".to_string(), 3, 12)), "x.length, on its own line of lib/f.js");
        assert_eq!(located(&map, 8, 3), Some(("app.js".to_string(), 2, 27)));
        assert_eq!(located(&map, 99, 1), None, "past the end nothing says where it came from");
    }
}
