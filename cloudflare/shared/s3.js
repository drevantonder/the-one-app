// Presigned URLs for the install's S3-compatible bucket (AWS Signature Version 4, in the query), so nodes reach it
// without holding the bucket's credentials: only the URL signer (cloudflare/signer) does. A create URL signs
// `If-None-Match: *` in, so the request must send it and can only make an object that isn't there; a replace URL signs
// `If-Match: <ETag>` in, so it changes only the version it names.

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

/// A presigned URL for `method` on `<scheme>://<host><path>?<query>`, good for `expires` seconds from `date`.
/// `headers` are signed in, so the request must send them as given.
export async function presign({ method, scheme = "https", host, path, query = {}, region, keyId, secret, expires, date = new Date(), headers = {} }) {
  const stamp = date.toISOString().replace(/[-:]/g, "").replace(/\.\d+/, "");
  const day = stamp.slice(0, 8);
  const scope = `${day}/${region}/s3/aws4_request`;
  const signed = { host, ...Object.fromEntries(Object.entries(headers).map(([k, v]) => [k.toLowerCase(), String(v).trim()])) };
  const names = Object.keys(signed).sort();
  const all = {
    ...query,
    "X-Amz-Algorithm": "AWS4-HMAC-SHA256",
    "X-Amz-Credential": `${keyId}/${scope}`,
    "X-Amz-Date": stamp,
    "X-Amz-Expires": String(expires),
    "X-Amz-SignedHeaders": names.join(";"),
  };
  const canonicalQuery = Object.keys(all).map((k) => [encode(k), encode(all[k])]).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([k, v]) => `${k}=${v}`).join("&");
  const canonicalPath = path.split("/").map(encode).join("/");
  const canonical = [method, canonicalPath, canonicalQuery, names.map((n) => `${n}:${signed[n]}\n`).join(""), names.join(";"), "UNSIGNED-PAYLOAD"].join("\n");
  const toSign = ["AWS4-HMAC-SHA256", stamp, scope, hex(await crypto.subtle.digest("SHA-256", text.encode(canonical)))].join("\n");
  let key = await hmac(`AWS4${secret}`, day);
  for (const part of [region, "s3", "aws4_request"]) key = await hmac(key, part);
  return `${scheme}://${host}${canonicalPath}?${canonicalQuery}&X-Amz-Signature=${hex(await hmac(key, toSign))}`;
}

/// URLs in a bucket `{ endpoint, bucket, region, key_id, secret }` (path-style), good for `expires` seconds: to read
/// objects, HEAD them, create them, replace them while they're the version an ETag names, delete them, and list
/// under a prefix (`{ prefix, delimiter?, token? }`, a page from a continuation token), in the order asked.
export async function bucketUrls(bucket, { read = [], head = [], create = [], replace = [], remove = [], list = [] }, expires) {
  const endpoint = new URL(bucket.endpoint.includes("://") ? bucket.endpoint : `https://${bucket.endpoint}`);
  // http only for a throwaway network's, on its own machine (src/throwaway.rs).
  const common = { scheme: endpoint.protocol === "http:" ? "http" : "https", host: endpoint.host, region: bucket.region || "auto", keyId: bucket.key_id, secret: bucket.secret, expires, date: new Date() };
  const root = `${endpoint.pathname.replace(/\/$/, "")}/${bucket.bucket}`;
  const at = (name) => `${root}/${name}`;
  const out = { read: {}, head: {}, create: {}, replace: {}, remove: {}, list: [] };
  for (const name of read) out.read[name] = await presign({ ...common, method: "GET", path: at(name) });
  for (const name of head) out.head[name] = await presign({ ...common, method: "HEAD", path: at(name) });
  for (const name of create) out.create[name] = await presign({ ...common, method: "PUT", path: at(name), headers: { "if-none-match": "*" } });
  for (const { name, etag } of replace) out.replace[name] = await presign({ ...common, method: "PUT", path: at(name), headers: { "if-match": etag } });
  for (const name of remove) out.remove[name] = await presign({ ...common, method: "DELETE", path: at(name) });
  for (const { prefix, delimiter, token } of list) {
    const query = { "list-type": "2", prefix };
    if (delimiter) query.delimiter = delimiter;
    if (token) query["continuation-token"] = token;
    out.list.push(await presign({ ...common, method: "GET", path: root, query }));
  }
  return out;
}
