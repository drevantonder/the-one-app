//! What a client records of itself for Telemetry, with OpenTelemetry's own SDK: its requests that failed or were
//! slow, each a span a server's span of it is under (`traceparent`), how long no server answered it, how long its
//! views took to open, what its window showed went wrong and its crashes. The SDK's OTLP exporter writes each export
//! request to the device's reports file (`Kept`), which every client of the device adds to, and whichever next
//! reaches a server tells it them all (`Client::send_reports`): so what went wrong while nothing could be reached is
//! told once something can. Names, sizes, durations and outcomes only: never what anyone wrote, was given or
//! answered, and never a secret.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use data_encoding::BASE64;
use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry::metrics::{Histogram, Meter, MeterProvider};
use opentelemetry::trace::{Span, SpanKind, Status, Tracer, TracerProvider};
use opentelemetry::{Context, KeyValue, Value as OtelValue};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::WithHttpConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider, SimpleSpanProcessor, SpanData, SpanProcessor};
use serde_json::{Value, json};

use crate::{Client, BUILD, name};

/// What a client's exports are kept in until a server is told: a line of JSON each, `{"signal", "otlp"}` with the
/// export request in base64, in its home folder, which every client of the device adds to, and past this many bytes
/// no more.
const REPORTS: &str = "reports";
const REPORTS_MOST: u64 = 512 * 1024;

/// How long a request takes before its span is kept though it was answered.
const SLOW: Duration = Duration::from_secs(2);

/// The buckets a duration in milliseconds is counted in, as a server's are (`OneCore.Otlp.buckets/0`).
const BUCKETS: [f64; 16] =
    [1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0, 30000.0, 60000.0, 300000.0];

/// The SDK's providers for one client, made with what it says of itself as it first records something.
pub(crate) struct Telemetry {
    traces: SdkTracerProvider,
    metrics: SdkMeterProvider,
    logs: SdkLoggerProvider,
    tracer: SdkTracer,
    meter: Meter,
    logger: opentelemetry_sdk::logs::SdkLogger,
    histograms: std::sync::Mutex<HashMap<String, Histogram<f64>>>,
}

impl Telemetry {
    /// The providers, exporting to `home`'s reports, saying `resource` of the client: which it is (`service.name`),
    /// its build, its device's name and its operating system.
    fn new(home: PathBuf, resource: &serde_json::Map<String, Value>) -> Self {
        let resource = Resource::builder_empty().with_attributes(attributes(resource)).build();
        let kept = Kept(home);
        let spans = opentelemetry_otlp::SpanExporter::builder().with_http().with_http_client(kept.clone()).build();
        // Each export says what was measured since the one before (delta), as the server's do (config/runtime.exs) and
        // Telemetry adds them up: the SDK's default, cumulative, sent each measurement again every minute until the
        // program ended, so one memory report was counted for hours.
        let numbers = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_http_client(kept.clone())
            .with_temporality(opentelemetry_sdk::metrics::Temporality::Delta)
            .build();
        let lines = opentelemetry_otlp::LogExporter::builder().with_http().with_http_client(kept).build();
        let (spans, numbers, lines) = (spans.expect("no I/O"), numbers.expect("no I/O"), lines.expect("no I/O"));
        let traces = SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_span_processor(Noteworthy(SimpleSpanProcessor::new(spans)))
            .build();
        let reader = PeriodicReader::builder(numbers).with_interval(Duration::from_secs(60)).build();
        let metrics = SdkMeterProvider::builder().with_resource(resource.clone()).with_reader(reader).build();
        let logs = SdkLoggerProvider::builder().with_resource(resource).with_simple_exporter(lines).build();
        let (tracer, meter, logger) = (traces.tracer("one"), metrics.meter("one"), logs.logger("one"));
        Self { traces, metrics, logs, tracer, meter, logger, histograms: Default::default() }
    }

    /// Records `record`, as Telemetry's tool `record` takes one: with a `value`, a number of a histogram's; with
    /// `took`, in milliseconds, a span of something that ended now, with an `error` if it failed; and otherwise a
    /// log line, an error if it has an `error`.
    fn record(&self, record: &Value) {
        let name = record["name"].as_str().unwrap_or("record").to_owned();
        let said = record["attributes"].as_object().map(attributes).unwrap_or_default();
        let error = record["error"].as_str().map(short);
        if let Some(value) = record["value"].as_f64() {
            let mut histograms = self.histograms.lock().expect("no record panics");
            let histogram = histograms
                .entry(name.clone())
                .or_insert_with(|| self.meter.f64_histogram(name).with_boundaries(BUCKETS.to_vec()).build());
            histogram.record(value, &said);
        } else if let Some(took) = record["took"].as_f64() {
            let now = SystemTime::now();
            let mut span = self
                .tracer
                .span_builder(name)
                .with_start_time(now.checked_sub(Duration::from_secs_f64(took.max(0.0) / 1000.0)).unwrap_or(now))
                .with_attributes(said)
                .start_with_context(&self.tracer, &Context::new());
            if let Some(error) = error {
                span.set_status(Status::error(error));
            }
            span.end_with_timestamp(now);
        } else {
            let mut line = self.logger.create_log_record();
            line.set_timestamp(SystemTime::now());
            line.set_severity_number(if error.is_some() { Severity::Error } else { Severity::Info });
            line.set_severity_text(if error.is_some() { "ERROR" } else { "INFO" });
            line.set_body(AnyValue::from(error.unwrap_or_else(|| name.clone())));
            line.add_attribute("event.name", name);
            line.add_attributes(said.into_iter().map(|said| {
                let value = match said.value {
                    OtelValue::Bool(yes) => AnyValue::from(yes),
                    OtelValue::I64(number) => AnyValue::from(number),
                    OtelValue::F64(number) => AnyValue::from(number),
                    other => AnyValue::from(other.to_string()),
                };
                (said.key, value)
            }));
            self.logger.emit(line);
        }
    }

    /// Exports what's gathered and not yet exported: the numbers, which are exported a minute at a time.
    pub(crate) fn flush(&self) {
        for flushed in [self.metrics.force_flush(), self.traces.force_flush(), self.logs.force_flush()] {
            if let Err(error) = flushed {
                failed("export", error);
            }
        }
    }
}

/// A request's span as it starts: under none, a trace of its own, MCP's name for it, its method and the tool it
/// calls, kept only if it fails or is slow (`Noteworthy`).
pub(crate) fn request_span(telemetry: &Telemetry, method: &str, tool: Option<&str>) -> opentelemetry_sdk::trace::Span {
    let mut said = vec![KeyValue::new("mcp.method.name", method.to_owned())];
    if let Some(tool) = tool {
        said.push(KeyValue::new("gen_ai.tool.name", tool.to_owned()));
    }
    telemetry
        .tracer
        .span_builder(tool.map_or(method.to_owned(), |tool| format!("{method} {tool}")))
        .with_kind(SpanKind::Client)
        .with_attributes(said)
        .start_with_context(&telemetry.tracer, &Context::new())
}

/// Keeps a client's span only where it failed or was slow: a request answered quickly is its server's span alone.
#[derive(Debug)]
struct Noteworthy<P>(P);

impl<P: SpanProcessor> SpanProcessor for Noteworthy<P> {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &Context) {
        self.0.on_start(span, cx)
    }

    fn on_end(&self, span: SpanData) {
        let slow = span.end_time.duration_since(span.start_time).is_ok_and(|took| took > SLOW);
        if span.span_kind != SpanKind::Client || slow || matches!(span.status, Status::Error { .. }) {
            self.0.on_end(span)
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource)
    }
}

/// The OTLP exporters' HTTP client, which sends nothing: it keeps each export request in the device's reports, by
/// the signal its path names, for a server to be told (`Client::send_reports`).
#[derive(Debug, Clone)]
struct Kept(PathBuf);

#[async_trait::async_trait]
impl HttpClient for Kept {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let signal = request.uri().path().rsplit('/').next().unwrap_or_default().to_owned();
        keep(&self.0, &json!({"signal": signal, "otlp": BASE64.encode(request.body())}));
        Ok(Response::builder().status(200).body(Bytes::new())?)
    }
}

/// Keeps `line` in `home`'s reports, for whichever client of the device next reaches a server.
fn keep(home: &std::path::Path, line: &Value) {
    let file = home.join(REPORTS);
    if fs::metadata(&file).is_ok_and(|metadata| metadata.len() > REPORTS_MOST) {
        return;
    }
    let line = format!("{line}\n");
    let log = fs::OpenOptions::new().create(true).append(true).open(&file);
    if let Err(error) = log.and_then(|mut log| log.write_all(line.as_bytes())) {
        failed("report", error);
    }
}

/// Says where the client logs (its stderr) that telemetry failed, once: a full disk or an exporter that's down fails
/// every time, and telemetry about telemetry would be as much noise as that.
fn failed(what: &str, error: impl std::fmt::Display) {
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| eprintln!("telemetry's {what} failed, and it won't say again: {error}"));
}

/// Attributes as OpenTelemetry holds them, each text cut short.
fn attributes(said: &serde_json::Map<String, Value>) -> Vec<KeyValue> {
    said.iter()
        .filter_map(|(name, value)| {
            let value = match value {
                Value::String(text) => OtelValue::from(short(text)),
                Value::Bool(yes) => OtelValue::from(*yes),
                Value::Number(number) if number.is_i64() => OtelValue::from(number.as_i64()?),
                Value::Number(number) => OtelValue::from(number.as_f64()?),
                _ => return None,
            };
            Some(KeyValue::new(name.clone(), value))
        })
        .collect()
}

fn short(text: &str) -> String {
    text.chars().take(1000).collect()
}

/// What a client tells a server of what it reported, for the apps that hear `measured`.
const MEASURED: &str = "notifications/one/measured";

/// What a client of program `service` says of itself in what it reports.
pub(crate) fn resource(service: &str) -> serde_json::Map<String, Value> {
    serde_json::Map::from_iter([
        ("service.name".to_owned(), json!(service)),
        ("service.version".to_owned(), json!(BUILD)),
        ("os.type".to_owned(), json!(env::consts::OS)),
    ])
}

impl Client {
    /// This client's OpenTelemetry SDK, made with what it says of itself the first time it records something.
    pub(crate) fn telemetry(&self) -> &Telemetry {
        self.telemetry.get_or_init(|| {
            let mut resource = self.resource.write().expect("no report panics");
            resource.entry("host.name").or_insert_with(|| json!(name()));
            Telemetry::new(self.home.clone(), &resource)
        })
    }

    /// Keeps `record` for Telemetry, as from this client, and tells a server once one answers, with whatever else
    /// this device's clients kept: so what went wrong while nothing could be reached is told once something can. A
    /// record is Telemetry's (its tool `record`): a `name` and `attributes`, and a `value`, or how long something
    /// `took` with its `error` if it failed, or else a log line, with an `error` if it is one. Names, sizes and
    /// outcomes only: never what anyone wrote, was given or answered, and never a secret.
    pub fn report(&self, record: Value) {
        self.telemetry().record(&record);
        if self.state.borrow().connection().answers() {
            self.send_soon();
        }
    }

    /// Reports a panic of this program's when it happens, so it's told next time any of its device's clients
    /// reaches a server, if not before.
    pub(crate) fn panics(&self) {
        static HOOKED: std::sync::Once = std::sync::Once::new();
        let Some(me) = self.me.upgrade() else { return };
        let me = Arc::downgrade(&me);
        HOOKED.call_once(move || {
            let before = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                let message = panic.payload_as_str().unwrap_or("panicked");
                let mut record = json!({"name": "exception", "error": message.lines().next().unwrap_or(message),
                    "attributes": {"exception.type": "panic"}});
                if let Some(at) = panic.location() {
                    record["attributes"]["code.file.path"] = json!(at.file());
                    record["attributes"]["code.line.number"] = json!(at.line());
                }
                if let Some(me) = me.upgrade() {
                    me.telemetry().record(&record);
                }
                before(panic)
            }));
        });
    }

    /// Tells a server what was reported, soon after it's asked something else, and never in the way of that.
    pub(crate) fn send_soon(&self) {
        if let Some(me) = self.me.upgrade()
            && self.home.join(REPORTS).exists()
        {
            self.runtime.spawn(async move { me.send_reports().await });
        }
    }

    /// Tells a server, in one batch, what this device's clients reported and kept (`report`), for the apps that
    /// hear `measured`. What it can't tell is kept again for next time. Telling reports nothing itself.
    pub(crate) async fn send_reports(&self) {
        let _one = self.sending.lock().await;
        // The kept exports are this run's to tell once it has moved them: other clients keep theirs anew.
        let sending = self.home.join(format!("{}.{}", REPORTS, self.link().endpoint.id()));
        if fs::rename(self.home.join(REPORTS), &sending).is_err() {
            return;
        }
        let stored = fs::read_to_string(&sending).unwrap_or_default();
        let _ = fs::remove_file(&sending);
        // A line a client before OpenTelemetry's SDK kept, a record, is let go.
        let exports: Vec<Value> = stored
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|line| line["signal"].is_string())
            .collect();
        let notification = json!({"jsonrpc": "2.0", "method": MEASURED, "params": {"otlp": exports}});
        if !exports.is_empty() && self.ask(notification.to_string().as_bytes()).await.is_err() {
            for export in &exports {
                keep(&self.home, export);
            }
        }
    }
}
