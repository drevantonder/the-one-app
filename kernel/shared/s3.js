// Presigned URLs for the install's S3-compatible bucket (AWS Signature Version 4, in the query), so nodes
// read and create objects without holding the bucket's credentials. A create URL signs `If-None-Match: *` in, so the
// request must send it and can only make an object that isn't there; a replace URL signs `If-Match: <ETag>` in, so it
// changes only the version it names. Nothing a node does deletes data: only the vault deletes, erasing an app's key.

const text = new TextEncoder();
const hex = (b) => [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, "0")).join("");

async function hmac(key, data) {
  const k = await crypto.subtle.importKey("raw", typeof key === "string" ? text.encode(key) : key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return new Uint8Array(await crypto.subtle.sign("HMAC", k, text.encode(data)));
}

/// RFC 3986 encoding, as Signature Version 4 wants it.
function encode(s) {
  return encodeURIComponent(s).replace(/[!'()*]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
}

/// A presigned URL for `method` on `https://<host><path>`, good for `expires` seconds from `date`. `headers` are
/// signed in, so the request must send them as given.
export async function presign({ method, host, path, region, keyId, secret, expires, date = new Date(), headers = {}, params = {} }) {
  const stamp = date.toISOString().replace(/[-:]/g, "").replace(/\.\d+/, "");
  const day = stamp.slice(0, 8);
  const scope = `${day}/${region}/s3/aws4_request`;
  const signed = { host, ...Object.fromEntries(Object.entries(headers).map(([k, v]) => [k.toLowerCase(), String(v).trim()])) };
  const names = Object.keys(signed).sort();
  const query = {
    ...params,
    "X-Amz-Algorithm": "AWS4-HMAC-SHA256",
    "X-Amz-Credential": `${keyId}/${scope}`,
    "X-Amz-Date": stamp,
    "X-Amz-Expires": String(expires),
    "X-Amz-SignedHeaders": names.join(";"),
  };
  const canonicalQuery = Object.keys(query).sort().map((k) => `${encode(k)}=${encode(query[k])}`).join("&");
  const canonicalPath = path.split("/").map(encode).join("/");
  const canonical = [method, canonicalPath, canonicalQuery, names.map((n) => `${n}:${signed[n]}\n`).join(""), names.join(";"), "UNSIGNED-PAYLOAD"].join("\n");
  const toSign = ["AWS4-HMAC-SHA256", stamp, scope, hex(await crypto.subtle.digest("SHA-256", text.encode(canonical)))].join("\n");
  let key = await hmac(`AWS4${secret}`, day);
  for (const part of [region, "s3", "aws4_request"]) key = await hmac(key, part);
  return `https://${host}${canonicalPath}?${canonicalQuery}&X-Amz-Signature=${hex(await hmac(key, toSign))}`;
}

/// URLs to read objects, to create them, to replace them while they're the version an ETag names, and to delete them
/// (for the vault itself), in a bucket `{ endpoint, bucket, region, key_id, secret }` (a Backup place's fields),
/// path-style, good for `expires` seconds.
export async function bucketUrls(bucket, { read = [], create = [], replace = [], remove = [] }, expires = 900) {
  const endpoint = new URL(bucket.endpoint.includes("://") ? bucket.endpoint : `https://${bucket.endpoint}`);
  const common = { host: endpoint.host, region: bucket.region || "auto", keyId: bucket.key_id, secret: bucket.secret, expires, date: new Date() };
  const at = (name) => `${endpoint.pathname.replace(/\/$/, "")}/${bucket.bucket}/${name}`;
  const out = { read: {}, create: {}, replace: {} };
  if (remove.length) out.remove = {};
  for (const name of read) out.read[name] = await presign({ ...common, method: "GET", path: at(name) });
  for (const name of create) out.create[name] = await presign({ ...common, method: "PUT", path: at(name), headers: { "if-none-match": "*" } });
  for (const { name, etag } of replace) out.replace[name] = await presign({ ...common, method: "PUT", path: at(name), headers: { "if-match": etag } });
  for (const name of remove) out.remove[name] = await presign({ ...common, method: "DELETE", path: at(name) });
  return out;
}

/// A URL listing the folders directly in `prefix` (which ends in /), as ListObjectsV2 does with a delimiter.
export async function bucketList(bucket, prefix, expires = 60) {
  const endpoint = new URL(bucket.endpoint.includes("://") ? bucket.endpoint : `https://${bucket.endpoint}`);
  const path = `${endpoint.pathname.replace(/\/$/, "")}/${bucket.bucket}`;
  const params = { "list-type": "2", prefix, delimiter: "/" };
  return presign({ method: "GET", host: endpoint.host, path, region: bucket.region || "auto", keyId: bucket.key_id, secret: bucket.secret, expires, params });
}

/// The folders' names in a ListObjectsV2 answer, without the prefix or the trailing /.
export function listedFolders(xml, prefix) {
  return [...xml.matchAll(/<CommonPrefixes>\s*<Prefix>([^<]*)<\/Prefix>/g)].map((m) => m[1].slice(prefix.length).replace(/\/$/, ""));
}
