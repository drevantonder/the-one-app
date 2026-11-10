//! Telemetry's store on Cloudflare: a bucket of its own with Basin's catalog and table maintenance, and for each table a
//! stream, a sink and a pipeline. Set up by every deploy and changing only what isn't there yet, so a network or a custom
//! build needs nothing done by hand. A sink can't change its table and a stream can't change its schema, so `TABLES`'
//! columns are few and stable and all else goes in `attrs`; a new column would mean a new stream, sink, pipeline and table.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::workers::{Cloudflare, Settings};

/// A column: its name, its type, and whether every row has it.
type Column = (&'static str, &'static str, bool);

/// Each table's columns. The same as the telemetry Worker's (`TABLES`), as tests/contracts.rs holds them.
pub const TABLES: &[(&str, &[Column])] = &[
    ("spans", &[("time", "int64", true), ("trace", "string", true), ("span", "string", true), ("parent", "string", false), ("name", "string", true), ("service", "string", true), ("version", "string", false), ("instance", "string", true), ("kind", "string", true), ("status", "string", true), ("duration", "int64", true), ("message", "string", false), ("attrs", "string", true)]),
    ("logs", &[("time", "int64", true), ("trace", "string", false), ("span", "string", false), ("severity_number", "int32", true), ("severity_text", "string", true), ("body", "string", true), ("service", "string", true), ("version", "string", false), ("instance", "string", true), ("attrs", "string", true)]),
    ("metrics", &[("time", "int64", true), ("name", "string", true), ("type", "string", true), ("unit", "string", true), ("service", "string", true), ("instance", "string", true), ("count", "int64", true), ("sum", "float64", true), ("buckets", "string", false), ("attrs", "string", true)]),
];

/// The namespace each table is in.
const NAMESPACE: &str = "telemetry";
/// How often a sink rolls a file: how long a row takes to be read. Cloudflare's default never showed new rows.
const ROLL_SECONDS: u64 = 10;
/// The Cloudflare permission groups of the Worker's own token: on the account "Workers R2 SQL Read", which SQL refuses to
/// take for one bucket, and on the bucket Basin's catalog and its objects, which a sink writes with.
const ACCOUNT_PERMISSIONS: [&str; 1] = ["f45430d92e2b4a6cb9f94f2594c141b8"];
const BUCKET_PERMISSIONS: [&str; 3] = ["d229766a2f7f4d299f20eaa8c9b1fde9", "6a018a9f2fc74eb6b293b0c548f38b39", "2efd5506f9c8494dacb1fa10a3e7d5b6"];

/// The permissions the API token a deploy uses needs for this. An error saying "Unauthorized" doesn't say which.
const NEEDS: &str = "Account API Tokens Write, Pipelines Write, Workers R2 Data Catalog Write and Workers R2 Storage Write";

/// Where a network's telemetry is on Cloudflare.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Telemetry {
    pub account: String,
    pub bucket: String,
    /// Each table's stream id, which the Worker's binding names.
    pub streams: BTreeMap<String, String>,
    /// Each pipeline's name by its id, which Cloudflare's analytics name it by: a deploy's token may not list them.
    #[serde(default)]
    pub pipelines: BTreeMap<String, String>,
}

impl Telemetry {
    /// Where Basin SQL answers.
    pub fn sql(&self) -> String {
        format!("https://api.sql.cloudflarestorage.com/api/v1/accounts/{}/basin-sql/query/{}", self.account, self.bucket)
    }
}

/// A resource's name: Basin takes underscores and no hyphens.
fn named(table: &str, kind: &str) -> String {
    format!("{}_{table}_{kind}", crate::config::CLI.replace('-', "_"))
}

fn fields(columns: &[Column]) -> Value {
    json!(columns.iter().map(|(name, kind, required)| json!({ "name": name, "type": kind, "required": required })).collect::<Vec<_>>())
}

/// Sets up what's missing and notes it in `settings`, keeping the token it makes beside the Workers' secrets.
pub fn provision(cf: &Cloudflare, settings: &mut Settings) -> Result<()> {
    let account = cf.account.clone();
    let bucket = format!("{}-telemetry", crate::config::CLI);
    let provisioned = (|| -> Result<_> {
        let buckets = format!("/accounts/{account}/r2/buckets");
        if cf.found(&format!("{buckets}/{bucket}"))?.is_none() {
            cf.post(&buckets, json!({ "name": bucket }))?;
        }
        let catalog = format!("/accounts/{account}/r2-catalog/{bucket}");
        if cf.found(&catalog)?.is_none_or(|c| c["status"] != "active") {
            cf.post(&format!("{catalog}/enable"), json!({}))?;
        }
        let token = match &settings.telemetry_token {
            Some(token) => token.clone(),
            None => {
                let token = worker_token(cf, &account, &bucket)?;
                settings.set_secret("TELEMETRY_TOKEN", &token)?;
                settings.telemetry_token = Some(token.clone());
                token
            }
        };
        cf.post(&format!("{catalog}/credential"), json!({ "token": token }))?;
        cf.post(&format!("{catalog}/maintenance-configs"), json!({ "compaction": { "state": "enabled", "target_size_mb": "128" }, "snapshot_expiration": { "state": "enabled", "max_snapshot_age": "1d", "min_snapshots_to_keep": 5 } }))?;
        let (mut streams, mut pipelines) = (BTreeMap::new(), BTreeMap::new());
        for (table, columns) in TABLES {
            let (stream, id) = pipeline(cf, &account, &bucket, &token, table, columns)?;
            streams.insert(table.to_string(), stream);
            pipelines.insert(id, named(table, "pipeline"));
        }
        Ok((streams, pipelines))
    })();
    let (streams, pipelines) = provisioned.with_context(|| format!("setting up telemetry in Basin, which needs a Cloudflare API token with {NEEDS}"))?;
    settings.telemetry = Some(Telemetry { account, bucket, streams, pipelines });
    settings.save()
}

/// The Cloudflare permission group of the token the telemetry Worker reads the account's analytics with, and nothing else:
/// "Account Analytics Read".
const ANALYTICS_PERMISSIONS: [&str; 1] = ["b89a480218d04ceb98b4fe57ca29dc1f"];

/// What of the Cloudflare account is the network's (`settings.usage`), each as Cloudflare's analytics filter it, to what
/// it's called: its Workers, buckets, Durable Object namespaces, pipelines and Basin's catalog, which the telemetry Worker
/// reads the hourly usage of (cloudflare/telemetry/usage.js). The token it reads with is made the first time, which a
/// deploy's token may not, and kept beside the Workers' secrets.
pub fn usage(cf: &Cloudflare, settings: &mut Settings) -> Result<()> {
    let telemetry = settings.telemetry.clone().context("telemetry isn't in Basin yet")?;
    if settings.analytics_token.is_none() {
        let policies = json!([{ "effect": "allow", "resources": { format!("com.cloudflare.api.account.{}", cf.account): "*" }, "permission_groups": ANALYTICS_PERMISSIONS.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>() }]);
        let made = cf.post(&format!("/accounts/{}/tokens", cf.account), json!({ "name": format!("{}-usage", crate::config::CLI), "policies": policies })).context("making the token the telemetry Worker reads the account's analytics with, which needs a Cloudflare API token with Account API Tokens Write")?;
        let token = made["value"].as_str().context("Cloudflare made the token and didn't give its value")?.to_string();
        settings.set_secret("ANALYTICS_TOKEN", &token)?;
        settings.analytics_token = Some(token);
    }
    let named = |names: Vec<String>| names.into_iter().map(|n| (n.clone(), n)).collect::<BTreeMap<_, _>>();
    let scripts: Vec<String> = super::WORKERS.iter().chain(settings.cloudflare_node.then_some(&super::NODE)).map(|w| w.worker_name()).collect();
    let namespaces: BTreeMap<String, String> = cf.list(&format!("/accounts/{}/workers/durable_objects/namespaces", cf.account))?.iter().filter(|n| n["script"].as_str().is_some_and(|s| scripts.iter().any(|w| w == s))).filter_map(|n| Some((n["id"].as_str()?.to_string(), n["name"].as_str()?.to_string()))).collect();
    let buckets: Vec<String> = [Some(telemetry.bucket.clone()), settings.bucket.as_ref().and_then(|b| b["bucket"].as_str()).map(str::to_string)].into_iter().flatten().collect();
    settings.usage = Some(json!({
        "account": cf.account,
        "scripts": named(scripts),
        "buckets": named(buckets),
        "namespaces": namespaces,
        "pipelines": telemetry.pipelines,
        "warehouses": { format!("{}_{}", cf.account, telemetry.bucket): telemetry.bucket },
    }));
    Ok(())
}

/// The name of Cloudflare's destination for the Workers' own traces.
pub fn destination() -> String {
    format!("{}-worker-traces", crate::config::CLI)
}

/// Points Cloudflare's trace destination at the telemetry Worker, with a header only it knows: Cloudflare's export can't
/// sign, and is OTLP/JSON. It's the telemetry Worker's own address, never the gateway's, or each export would be a request
/// of the gateway's for the next export to carry; a deploy that can't yet (Workers Observability Write) leaves it as it is.
pub fn traces(cf: &Cloudflare, settings: &mut Settings) -> Result<()> {
    let url = format!("https://{}.{}.workers.dev/telemetry/otlp", crate::cloudflare::worker_name("telemetry"), cf.subdomain()?);
    if settings.traces.as_deref() == Some(url.as_str()) {
        return Ok(());
    }
    let name = destination();
    let path = format!("/accounts/{}/workers/observability/destinations", cf.account);
    let there = cf.list(&path).context("reading Cloudflare's trace destinations, which needs an API token with Workers Observability Write")?;
    let token = match &settings.otlp_token {
        Some(token) => token.clone(),
        None => {
            let token = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
            settings.set_secret("OTLP_TOKEN", &token)?;
            settings.otlp_token = Some(token.clone());
            token
        }
    };
    let configuration = json!({ "type": "logpush", "logpushDataset": "opentelemetry-traces", "url": url, "headers": { "authorization": format!("Bearer {token}") } });
    // Cloudflare's destinations change their URL only with new header values, and the token stays: it's made anew at the same name.
    if there.iter().any(|d| d["name"] == name) {
        cf.delete(&format!("{path}/{name}")).context("replacing Cloudflare's trace destination")?;
    }
    cf.post(&path, json!({ "name": name, "enabled": true, "skipPreflightCheck": true, "configuration": configuration })).context("making Cloudflare's trace destination")?;
    settings.traces = Some(url);
    settings.save()
}

/// A token only for that bucket's SQL, catalog and files: what the Worker reads with and the sinks write with.
fn worker_token(cf: &Cloudflare, account: &str, bucket: &str) -> Result<String> {
    let resource = format!("com.cloudflare.edge.r2.bucket.{account}_default_{bucket}");
    let groups = |ids: &[&str]| ids.iter().map(|id| json!({ "id": id })).collect::<Vec<Value>>();
    let policies = json!([
        { "effect": "allow", "resources": { format!("com.cloudflare.api.account.{account}"): "*" }, "permission_groups": groups(&ACCOUNT_PERMISSIONS) },
        { "effect": "allow", "resources": { resource: "*" }, "permission_groups": groups(&BUCKET_PERMISSIONS) },
    ]);
    let made = cf.post(&format!("/accounts/{account}/tokens"), json!({ "name": format!("{}-telemetry", crate::config::CLI), "policies": policies }))?;
    Ok(made["value"].as_str().context("Cloudflare made the token and didn't give its value")?.to_string())
}

/// `table`'s stream, sink and pipeline, those that aren't there made: → the stream's id and the pipeline's.
fn pipeline(cf: &Cloudflare, account: &str, bucket: &str, token: &str, table: &str, columns: &[Column]) -> Result<(String, String)> {
    let base = format!("/accounts/{account}/pipelines/v1");
    let existing = |kind: &str, name: &str| -> Result<Option<String>> {
        let list = cf.list(&format!("{base}/{kind}"))?;
        Ok(list.iter().find(|r| r["name"] == name).and_then(|r| r["id"].as_str()).map(str::to_string))
    };
    let (stream, sink, pipeline) = (named(table, "stream"), named(table, "sink"), named(table, "pipeline"));
    let id = match existing("streams", &stream)? {
        Some(id) => id,
        None => {
            let made = cf.post(&format!("{base}/streams"), json!({ "name": stream, "format": { "type": "json", "unstructured": false }, "schema": { "fields": fields(columns) }, "http": { "enabled": false, "authentication": true }, "worker_binding": { "enabled": true } }))?;
            made["id"].as_str().context("Cloudflare made the stream and didn't give its id")?.to_string()
        }
    };
    if existing("sinks", &sink)?.is_none() {
        cf.post(&format!("{base}/sinks"), json!({ "name": sink, "type": "basin_catalog", "format": { "type": "parquet", "compression": "zstd" }, "schema": { "fields": fields(columns) }, "config": { "account_id": account, "bucket": bucket, "namespace": NAMESPACE, "table_name": table, "token": token, "rolling_policy": { "interval_seconds": ROLL_SECONDS } } }))?;
    }
    let made = match existing("pipelines", &pipeline)? {
        Some(made) => made,
        None => cf.post(&format!("{base}/pipelines"), json!({ "name": pipeline, "sql": format!("INSERT INTO {sink} SELECT * FROM {stream}") }))?["id"].as_str().context("Cloudflare made the pipeline and didn't give its id")?.to_string(),
    };
    ensure!(!id.is_empty(), "Cloudflare gave {stream} no id");
    Ok((id, made))
}
