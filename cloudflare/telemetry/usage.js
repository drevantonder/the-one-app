// What the network uses of Cloudflare, an hour at a time, as metric rows like any other: one GraphQL Analytics query for
// the account, of only the network's own resources (`USAGE`, which the deploy lists: its Workers, buckets, Durable Object
// namespaces and pipelines), grouped by those, the hour, and the kind of operation where that's what's billed (an R2
// operation's, a Durable Object invocation's), never by who asked or what: there's no request data in it to leave out. It
// runs at ten past each hour, by when Cloudflare's analytics have the hour before whole. Usage, never dollars:
// src/insights/cost.rs prices it when it's read.
//
// A product is one entry of `PRODUCTS`: its dataset, which of the network's resources it's filtered and named by, and its
// metrics, each OTEL's name and unit and how to read it from a group. Checked against the account's GraphQL schema on
// 2026-11-10.

const ANALYTICS = "https://api.cloudflare.com/client/v4/graphql";
const MOST_GROUPS = 10_000;

/// Each R2 operation's class, as Cloudflare bills it (https://developers.cloudflare.com/r2/pricing/, 2026-11-10): any other is free.
const R2_CLASS_A = ["ListBuckets", "PutBucket", "ListObjects", "PutObject", "CopyObject", "CompleteMultipartUpload", "CreateMultipartUpload", "LifecycleStorageTierTransition", "ListMultipartUploads", "UploadPart", "UploadPartCopy", "ListParts", "PutBucketEncryption", "PutBucketCors", "PutBucketLifecycleConfiguration"];
const R2_CLASS_B = ["HeadBucket", "HeadObject", "GetObject", "UsageSummary", "GetBucketEncryption", "GetBucketLocation", "GetBucketCors", "GetBucketLifecycleConfiguration"];
const r2Class = (operation) => (R2_CLASS_A.includes(operation) ? "A" : R2_CLASS_B.includes(operation) ? "B" : "free");

/// `dimension` is what a product's resource is filtered and grouped by and `of` which of `USAGE`'s lists names them; `more`
/// a dimension besides, if any, and what it adds to a row's attrs; `only` a filter of Cloudflare's own, if it needs one.
export const PRODUCTS = [
  {
    product: "workers", dataset: "workersInvocationsAdaptive", dimension: "scriptName", of: "scripts", fields: "sum { requests cpuTimeUs }",
    metrics: [
      { name: "cloudflare.workers.requests", unit: "{request}", type: "counter", value: (g) => g.sum.requests },
      { name: "cloudflare.workers.cpu_time", unit: "ms", type: "counter", value: (g) => g.sum.cpuTimeUs / 1000 },
    ],
  },
  {
    product: "r2", dataset: "r2OperationsAdaptiveGroups", dimension: "bucketName", of: "buckets", fields: "sum { requests }",
    more: { dimension: "actionType", attrs: (operation) => ({ "cloudflare.r2.operation": operation, "cloudflare.r2.class": r2Class(operation) }) },
    metrics: [{ name: "cloudflare.r2.operations", unit: "{operation}", type: "counter", value: (g) => g.sum.requests }],
  },
  {
    product: "r2", dataset: "r2StorageAdaptiveGroups", dimension: "bucketName", of: "buckets", fields: "max { payloadSize metadataSize }",
    metrics: [{ name: "cloudflare.r2.storage", unit: "By", type: "gauge", value: (g) => g.max.payloadSize + g.max.metadataSize }],
  },
  {
    // Each by what it was: `http`, `jsrpc`, `hibernation` (a WebSocket message), or another Cloudflare names.
    product: "durable_objects", dataset: "durableObjectsInvocationsAdaptiveGroups", dimension: "namespaceId", of: "namespaces", fields: "sum { requests }",
    more: { dimension: "type", attrs: (type) => ({ "cloudflare.durable_objects.type": type }) },
    metrics: [{ name: "cloudflare.durable_objects.requests", unit: "{request}", type: "counter", value: (g) => g.sum.requests }],
  },
  {
    // What's billed as duration: the time an object was active, not the wall time of the WebSockets it holds.
    product: "durable_objects", dataset: "durableObjectsPeriodicGroups", dimension: "namespaceId", of: "namespaces", fields: "sum { activeTime }",
    metrics: [{ name: "cloudflare.durable_objects.duration", unit: "s", type: "counter", value: (g) => g.sum.activeTime / 1e6 }],
  },
  {
    // Read from its streams: a row with no stream is the same bytes again, further on.
    product: "pipelines", dataset: "pipelinesOperatorAdaptiveGroups", dimension: "pipelineId", of: "pipelines", only: 'streamId_neq: ""', fields: "sum { bytesIn }",
    metrics: [{ name: "cloudflare.pipelines.processed", unit: "By", type: "counter", value: (g) => g.sum.bytesIn }],
  },
  {
    // Billed on what it delivers uncompressed.
    product: "pipelines", dataset: "pipelinesSinkAdaptiveGroups", dimension: "pipelineId", of: "pipelines", fields: "sum { uncompressedBytesWritten }",
    metrics: [{ name: "cloudflare.pipelines.delivered", unit: "By", type: "counter", value: (g) => g.sum.uncompressedBytesWritten }],
  },
  {
    product: "sql", dataset: "r2sqlOperationsAdaptiveGroups", dimension: "bucket", of: "buckets", fields: "count sum { r2BytesRead }",
    metrics: [
      { name: "cloudflare.sql.queries", unit: "{query}", type: "counter", value: (g) => g.count },
      { name: "cloudflare.sql.scanned", unit: "By", type: "counter", value: (g) => g.sum.r2BytesRead },
    ],
  },
  {
    product: "catalog", dataset: "r2CatalogDataOperationsAdaptiveGroups", dimension: "warehouseName", of: "warehouses", fields: "count",
    metrics: [{ name: "cloudflare.catalog.operations", unit: "{operation}", type: "counter", value: (g) => g.count }],
  },
];

/// The one query of every product's hour, each by its alias `p<i>`.
export function query(usage) {
  const parts = PRODUCTS.map((p, i) => {
    const dimensions = [p.dimension, p.more?.dimension].filter(Boolean).join(" ");
    return `p${i}: ${p.dataset}(limit: ${MOST_GROUPS}, filter: { datetime_geq: $from, datetime_lt: $to, ${p.dimension}_in: ${JSON.stringify(Object.keys(usage[p.of] ?? {}))}${p.only ? `, ${p.only}` : ""} }) { dimensions { ${dimensions} } ${p.fields} }`;
  });
  return `query($account: String!, $from: Time!, $to: Time!) { viewer { accounts(filter: { accountTag: $account }) { ${parts.join(" ")} } } }`;
}

/// What GraphQL answered (`accounts[0]`) as metric rows of the hour from `from`, each resource named as `usage` names it.
export function rows(answer, usage, from, instance) {
  return PRODUCTS.flatMap((p, i) => (answer?.[`p${i}`] ?? []).flatMap((g) => {
    const resource = usage[p.of]?.[g.dimensions[p.dimension]];
    if (!resource) return [];
    const attrs = { "cloudflare.product": p.product, "cloudflare.resource": resource, ...(p.more ? p.more.attrs(g.dimensions[p.more.dimension]) : {}) };
    return p.metrics.map((m) => ({ time: from, name: m.name, type: m.type, unit: m.unit, service: "cloudflare", instance, count: 1, sum: Number(m.value(g)) || 0, attrs: JSON.stringify(attrs) }));
  }));
}

/// The hour before `at`'s, as metric rows: none if this network has no analytics token yet.
export async function cloudflareUsage(env, at, instance) {
  if (!env.ANALYTICS_TOKEN || !env.USAGE) return [];
  const usage = JSON.parse(env.USAGE);
  const to = at - (at % 3_600_000), from = to - 3_600_000;
  const said = await fetch(ANALYTICS, {
    method: "POST",
    headers: { authorization: `Bearer ${env.ANALYTICS_TOKEN}`, "content-type": "application/json", "user-agent": `${env.CLI}-telemetry` },
    body: JSON.stringify({ query: query(usage), variables: { account: usage.account, from: new Date(from).toISOString(), to: new Date(to).toISOString() } }),
  });
  const body = await said.json().catch(() => ({}));
  if (!said.ok || body.errors?.length) throw new Error(`Cloudflare's analytics said ${said.status}: ${(body.errors ?? []).map((e) => e.message).join("; ")}`);
  return rows(body.data?.viewer?.accounts?.[0], usage, from, instance);
}
