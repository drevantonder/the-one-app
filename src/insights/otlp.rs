//! OTLP, what services' programs may send, as spans and logs: only what closed sets allow survives, and anything else
//! is dropped. Protobuf, which Go's exporter sends by default, is read as the JSON form of the same message (`from_protobuf`). A span's name stays if it's a plain token (some exporters name it `GET /users/<id>`, which isn't), and its
//! attributes only by these keys.

use serde_json::Value;

use super::{SpanKind, token};

/// The most spans one OTLP request adds.
const MOST: usize = 200;

/// A span or log record as insights keeps it.
#[derive(Clone)]
pub struct Item {
    pub name: String,
    /// A span's kind; a log record has none.
    pub span_kind: Option<SpanKind>,
    pub ms: u64,
    pub ok: bool,
    /// `error.type`, unchecked here: `Attrs::error` holds it to its set.
    pub error: String,
    pub status_code: Option<u64>,
    /// Its trace, span and parent ids, if they're W3C's.
    pub ids: Option<(String, String, Option<String>)>,
}

/// The spans and log records in an OTLP/JSON `traces` or `logs` request.
pub fn items(request: &Value) -> Vec<Item> {
    let in_list = |key: &str, inner: &str, leaf: &str| {
        request[key].as_array().into_iter().flatten().flat_map(|r| r[inner].as_array().into_iter().flatten()).flat_map(|s| s[leaf].as_array().into_iter().flatten()).cloned().collect::<Vec<Value>>()
    };
    let traces = in_list("resourceSpans", "scopeSpans", "spans").into_iter().map(|s| span(&s));
    let logs = in_list("resourceLogs", "scopeLogs", "logRecords").into_iter().map(|r| record(&r));
    traces.chain(logs).take(MOST).collect()
}

fn span(s: &Value) -> Item {
    let nano = |key: &str| s[key].as_str().and_then(|n| n.parse::<u64>().ok()).or(s[key].as_u64()).unwrap_or(0);
    let ms = nano("endTimeUnixNano").saturating_sub(nano("startTimeUnixNano")) / 1_000_000;
    let ok = s["status"]["code"].as_u64() != Some(2) && s["status"]["code"] != "STATUS_CODE_ERROR";
    let name = s["name"].as_str().filter(|n| token(n)).unwrap_or("span").to_string();
    let trace = s["traceId"].as_str().filter(|t| hex(t, 32));
    let id = s["spanId"].as_str().filter(|t| hex(t, 16));
    let ids = trace.zip(id).map(|(t, i)| (t.to_string(), i.to_string(), s["parentSpanId"].as_str().filter(|p| hex(p, 16)).map(str::to_string)));
    let span_kind = match (s["kind"].as_u64(), s["kind"].as_str()) {
        (Some(2), _) | (_, Some("SPAN_KIND_SERVER")) => SpanKind::Server,
        (Some(3), _) | (_, Some("SPAN_KIND_CLIENT")) => SpanKind::Client,
        _ => SpanKind::Internal,
    };
    Item { name, span_kind: Some(span_kind), ms, ok, error: attribute(s, "error.type").unwrap_or_default(), status_code: number(s, "http.response.status_code"), ids }
}

fn record(r: &Value) -> Item {
    let name = r["eventName"].as_str().filter(|n| token(n)).unwrap_or("log").to_string();
    let ok = r["severityNumber"].as_u64().is_none_or(|n| n < 17);
    Item { name, span_kind: None, ms: 0, ok, error: attribute(r, "error.type").unwrap_or_default(), status_code: None, ids: None }
}

/// A metric as a program said it over one request: its name if that's a plain token, added up over every point, as its
/// attributes aren't kept. What it adds is taken as new, or, if `cumulative`, as a running total.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub name: String,
    /// As the metrics table says it.
    pub kind: &'static str,
    pub unit: String,
    pub count: u64,
    pub sum: f64,
    /// Its bucket counts, only for a histogram on OTEL's default bounds, which all of insights' are.
    pub buckets: Option<Vec<u64>>,
    pub cumulative: bool,
}

impl Point {
    pub fn add(&mut self, other: &Point) {
        self.count += other.count;
        self.sum += other.sum;
        self.buckets = self.buckets.as_ref().zip(other.buckets.as_ref()).map(|(a, b)| a.iter().zip(b).map(|(x, y)| x + y).collect());
    }

    /// What it adds since `was`, the same program's last: all of it if it went down, as the program restarted.
    pub fn since(&self, was: &Point) -> Point {
        let restarted = self.count < was.count || self.sum < was.sum;
        let buckets = self.buckets.as_ref().zip(was.buckets.as_ref()).map(|(now, then)| now.iter().zip(then).map(|(n, t)| n.saturating_sub(*t)).collect());
        match restarted {
            true => self.clone(),
            false => Point { count: self.count - was.count, sum: self.sum - was.sum, buckets, ..self.clone() },
        }
    }
}

/// The most metrics one OTLP request adds.
const MOST_METRICS: usize = 200;

/// The metrics in an OTLP/JSON `metrics` request: a name, a unit and numbers; nothing a program put in an attribute.
pub fn metrics(request: &Value) -> Vec<Point> {
    let all = request["resourceMetrics"].as_array().into_iter().flatten().flat_map(|r| r["scopeMetrics"].as_array().into_iter().flatten()).flat_map(|s| s["metrics"].as_array().into_iter().flatten());
    all.filter_map(metric).take(MOST_METRICS).collect()
}

fn metric(m: &Value) -> Option<Point> {
    let name = m["name"].as_str().filter(|n| token(n))?.to_string();
    let unit = m["unit"].as_str().filter(|u| u.len() <= 32 && u.bytes().all(|b| b.is_ascii_alphanumeric() || b"{}/%._-".contains(&b))).unwrap_or("").to_string();
    let (data, kind) = [("sum", "counter"), ("gauge", "gauge"), ("histogram", "histogram")].into_iter().find(|(key, _)| m[*key].is_object())?;
    let temporality = &m[data]["aggregationTemporality"];
    let cumulative = kind != "gauge" && (temporality.as_u64() == Some(2) || temporality == "AGGREGATION_TEMPORALITY_CUMULATIVE");
    let points = m[data]["dataPoints"].as_array()?;
    let mut point = Point { name, kind, unit, count: 0, sum: 0.0, buckets: (kind == "histogram").then(|| vec![0; super::BOUNDS.len() + 1]), cumulative };
    for p in points {
        let (count, sum, buckets) = match kind {
            "histogram" => (numeric(&p["count"])? as u64, numeric(&p["sum"]).unwrap_or(0.0), histogram_buckets(p)),
            _ => (1, numeric(&p["asDouble"]).or(numeric(&p["asInt"]))?, None),
        };
        point.add(&Point { count, sum, buckets, ..point.clone() });
    }
    (point.count > 0).then_some(point)
}

fn histogram_buckets(p: &Value) -> Option<Vec<u64>> {
    let bounds: Vec<f64> = p["explicitBounds"].as_array()?.iter().filter_map(Value::as_f64).collect();
    let ours: Vec<f64> = super::BOUNDS.iter().map(|b| *b as f64).collect();
    (bounds == ours).then(|| p["bucketCounts"].as_array().map(|b| b.iter().map(|n| numeric(n).unwrap_or(0.0) as u64).collect()))?
}

/// A number as OTLP/JSON says one: a string for a 64-bit integer.
fn numeric(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

/// The OTLP/JSON of a protobuf request to `path` (`/v1/traces`, `/v1/logs` or `/v1/metrics`), if it is one.
#[cfg(feature = "node")]
pub fn from_protobuf(path: &str, body: &[u8]) -> Option<Value> {
    use opentelemetry_proto::tonic::collector::{logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest, trace::v1::ExportTraceServiceRequest};
    use prost::Message;
    match path {
        "/v1/traces" => serde_json::to_value(ExportTraceServiceRequest::decode(body).ok()?).ok(),
        "/v1/logs" => serde_json::to_value(ExportLogsServiceRequest::decode(body).ok()?).ok(),
        "/v1/metrics" => serde_json::to_value(ExportMetricsServiceRequest::decode(body).ok()?).ok(),
        _ => None,
    }
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

    /// Protobuf from a program, as Go's exporter sends it, reads as its JSON does, ids as hex, held to the same closed sets.
    #[cfg(feature = "node")]
    #[test]
    fn protobuf_is_read_as_the_same_message() {
        use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value};
        use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, Status, status::StatusCode};
        use prost::Message;
        let attribute = KeyValue { key: "url.full".into(), value: Some(AnyValue { value: Some(any_value::Value::StringValue("https://x/anna".into())) }), ..Default::default() };
        let span = Span { name: "GET /users/anna@example.org".into(), trace_id: vec![0xab; 16], span_id: vec![0xcd; 8], parent_span_id: vec![0xef; 8], start_time_unix_nano: 1_000_000_000, end_time_unix_nano: 1_500_000_000, status: Some(Status { code: StatusCode::Error as i32, message: "anna@example.org".into() }), attributes: vec![attribute], ..Default::default() };
        let request = ExportTraceServiceRequest { resource_spans: vec![ResourceSpans { scope_spans: vec![ScopeSpans { spans: vec![span], ..Default::default() }], ..Default::default() }] };
        let read = items(&from_protobuf("/v1/traces", &request.encode_to_vec()).unwrap());
        assert_eq!(read.len(), 1);
        assert_eq!((read[0].name.as_str(), read[0].ms, read[0].ok), ("span", 500, false), "a name that isn't a plain token isn't kept");
        assert_eq!(read[0].ids, Some(("ab".repeat(16), "cd".repeat(8), Some("ef".repeat(8)))), "a program's ids are the trace it continues");
        assert!(from_protobuf("/v1/traces", b"not protobuf").is_none());
        assert!(from_protobuf("/elsewhere", &[]).is_none());
    }

    /// A metric keeps a plain name and numbers, never what's in its attributes; and a running total counts only what's new.
    #[test]
    fn a_programs_metrics_are_kept_as_numbers() {
        let at = |name: &str, value: i64| json!({ "resourceMetrics": [{ "scopeMetrics": [{ "metrics": [{ "name": name, "unit": "{request}", "sum": { "aggregationTemporality": 2, "dataPoints": [{ "asInt": value.to_string(), "attributes": [{ "key": "user", "value": { "stringValue": "anna@example.org" } }] }] } }] }] }] });
        let first = metrics(&at("requests", 10)).remove(0);
        assert_eq!((first.name.as_str(), first.kind, first.sum, first.cumulative), ("requests", "counter", 10.0, true));
        assert!(!format!("{first:?}").contains("anna"), "{first:?}");
        assert!(metrics(&at("calls_for_anna@example.org", 1)).is_empty(), "a name that isn't a plain token is dropped");
        let later = metrics(&at("requests", 13)).remove(0);
        assert_eq!(later.since(&first).sum, 3.0);
        assert_eq!(first.since(&later).sum, 10.0, "going down is a restart, and all of it is new");
    }

    #[test]
    fn a_program_can_only_say_what_closed_sets_allow() {
        let request = json!({ "resourceSpans": [{ "scopeSpans": [{ "spans": [
            { "name": "GET /users/anna@example.org", "traceId": "not hex", "startTimeUnixNano": "1000000000", "endTimeUnixNano": "1500000000", "status": { "code": 2, "message": "anna@example.org" }, "attributes": [{ "key": "url.full", "value": { "stringValue": "https://x/anna" } }, { "key": "error.type", "value": { "stringValue": "anna@example.org" } }, { "key": "http.response.status_code", "value": { "intValue": "500" } }] },
            { "name": "ping", "traceId": "0".repeat(32), "spanId": "1".repeat(16), "startTimeUnixNano": 0, "endTimeUnixNano": 2000000, "status": {} },
        ] }] }] });
        let spans = items(&request);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].name.as_str(), spans[0].ms, spans[0].ok, spans[0].status_code), ("span", 500, false, Some(500)));
        assert!(spans[0].ids.is_none());
        let attrs = super::super::Attrs::default().error(&spans[0].error);
        assert_eq!(attrs.json()["error.type"], super::super::OTHER, "what a program calls its error isn't kept");
        assert_eq!((spans[1].name.as_str(), spans[1].ms), ("ping", 2));
        assert!(spans[1].ids.is_some());
    }
}
