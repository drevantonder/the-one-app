//! OTLP/JSON, what services' programs may send, as signals: only what closed sets allow survives, and anything else is
//! dropped. A span's name stays if it's a plain token (some exporters name it `GET /users/<id>`, which isn't), and its
//! attributes only by these keys.

use serde_json::Value;

use super::{Kind, token};

/// The most spans one OTLP request adds.
const MOST: usize = 200;

/// A span or log record as insights keeps it.
pub struct Span {
    pub name: String,
    pub kind: Kind,
    pub ms: u64,
    pub ok: bool,
    /// `error.type`, unchecked here: `Attrs::error` holds it to its set.
    pub error: String,
    pub status_code: Option<u64>,
    /// Its trace, span and parent ids, if they're W3C's.
    pub ids: Option<(String, String, Option<String>)>,
}

/// The spans and log records in an OTLP/JSON `traces` or `logs` request.
pub fn spans(request: &Value) -> Vec<Span> {
    let in_list = |key: &str, inner: &str, leaf: &str| {
        request[key].as_array().into_iter().flatten().flat_map(|r| r[inner].as_array().into_iter().flatten()).flat_map(|s| s[leaf].as_array().into_iter().flatten()).cloned().collect::<Vec<Value>>()
    };
    let traces = in_list("resourceSpans", "scopeSpans", "spans").into_iter().map(|s| span(&s));
    let logs = in_list("resourceLogs", "scopeLogs", "logRecords").into_iter().map(|r| record(&r));
    traces.chain(logs).take(MOST).collect()
}

fn span(s: &Value) -> Span {
    let nano = |key: &str| s[key].as_str().and_then(|n| n.parse::<u64>().ok()).or(s[key].as_u64()).unwrap_or(0);
    let ms = nano("endTimeUnixNano").saturating_sub(nano("startTimeUnixNano")) / 1_000_000;
    let ok = s["status"]["code"].as_u64() != Some(2) && s["status"]["code"] != "STATUS_CODE_ERROR";
    let name = s["name"].as_str().filter(|n| token(n)).unwrap_or("span").to_string();
    let trace = s["traceId"].as_str().filter(|t| hex(t, 32));
    let id = s["spanId"].as_str().filter(|t| hex(t, 16));
    let ids = trace.zip(id).map(|(t, i)| (t.to_string(), i.to_string(), s["parentSpanId"].as_str().filter(|p| hex(p, 16)).map(str::to_string)));
    Span { name, kind: Kind::Span, ms, ok, error: attribute(s, "error.type").unwrap_or_default(), status_code: number(s, "http.response.status_code"), ids }
}

fn record(r: &Value) -> Span {
    let name = r["eventName"].as_str().filter(|n| token(n)).unwrap_or("log").to_string();
    let ok = r["severityNumber"].as_u64().is_none_or(|n| n < 17);
    Span { name, kind: Kind::Event, ms: 0, ok, error: attribute(r, "error.type").unwrap_or_default(), status_code: None, ids: None }
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn attribute(item: &Value, key: &str) -> Option<String> {
    item["attributes"].as_array()?.iter().find(|a| a["key"] == key)?["value"]["stringValue"].as_str().map(str::to_string)
}

fn number(item: &Value, key: &str) -> Option<u64> {
    let value = &item["attributes"].as_array()?.iter().find(|a| a["key"] == key)?["value"];
    value["intValue"].as_str().and_then(|n| n.parse().ok()).or(value["intValue"].as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_program_can_only_say_what_closed_sets_allow() {
        let request = json!({ "resourceSpans": [{ "scopeSpans": [{ "spans": [
            { "name": "GET /users/anna@example.org", "traceId": "not hex", "startTimeUnixNano": "1000000000", "endTimeUnixNano": "1500000000", "status": { "code": 2, "message": "anna@example.org" }, "attributes": [{ "key": "url.full", "value": { "stringValue": "https://x/anna" } }, { "key": "error.type", "value": { "stringValue": "anna@example.org" } }, { "key": "http.response.status_code", "value": { "intValue": "500" } }] },
            { "name": "ping", "traceId": "0".repeat(32), "spanId": "1".repeat(16), "startTimeUnixNano": 0, "endTimeUnixNano": 2000000, "status": {} },
        ] }] }] });
        let spans = spans(&request);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].name.as_str(), spans[0].ms, spans[0].ok, spans[0].status_code), ("span", 500, false, Some(500)));
        assert!(spans[0].ids.is_none());
        let attrs = super::super::Attrs::default().error(&spans[0].error);
        assert_eq!(attrs.get("error.type"), Some(&json!(super::super::OTHER)), "what a program calls its error isn't kept");
        assert_eq!((spans[1].name.as_str(), spans[1].ms), ("ping", 2));
        assert!(spans[1].ids.is_some());
    }
}
