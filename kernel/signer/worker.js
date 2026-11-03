// The URL signer: the only holder of the install's bucket's credentials (`BUCKET`, `{ endpoint, bucket, region,
// key_id, secret }`). It gives presigned URLs to endpoints of the install, which the relay keeps the list of (`RELAY`,
// asked `allowed(key)`), for a request each signs with its own key (src/keys.rs `signed_request`, domain `one/urls`):
//
//   POST /urls { read, head, create: [item], replace: [{ name, etag }], remove: [name],
//                list: [{ prefix, delimiter?, token? }], confirm: [{ name, etag }], at, by, sig }
//   → { read, head, create, replace, remove: { name: url }, list: [url], confirm: { name: bool }, until }
//
// An item is a name, or a database's steps from one position to another (`{ steps: prefix, from, to }`), so a node
// asks for the positions it reads or writes next in one trip. Any endpoint of the install may read, list and create
// (only if absent: `If-None-Match: *` is signed in); a `…/newest` is replaced only by its ETag (`If-Match`); only an
// admin's endpoint removes. `confirm` says whether each object is there with that ETag, which opens nothing. It knows
// nobody's scopes and checks none: everything in the bucket is sealed, so what it hands out opens nothing without keys.

import { checkRequest } from "../shared/crypto.js";
import { bucketUrls } from "../shared/s3.js";

/// How long URLs work, in seconds, the most it signs at once, and the most objects it confirms at once.
const EXPIRES = 15 * 60;
const MOST = 256;
const CONFIRM = 100;
/// How long the relay's answer about a key is kept here: a removed endpoint is refused within this.
const ALLOWED_FOR = 5_000;

const refuse = (status, error) => Response.json({ error }, { status });

class Refused extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

const fail = (status, message) => {
  throw new Refused(status, message);
};

/// A name in the bucket: parts that are neither empty, `.` nor `..`.
const isName = (name) => typeof name === "string" && name.length <= 1024 && name.split("/").every((p) => p && p !== "." && p !== "..");

/// The names an item says: a name, or `{ steps, from, to }`, a database's steps at those positions (src/db.rs's
/// `step_name`).
function names(item) {
  if (typeof item === "string") return isName(item) ? [item] : fail(400, `${item} isn't a name in the bucket`);
  const { steps, from, to } = item || {};
  if (!isName(steps) || !Number.isSafeInteger(from) || !Number.isSafeInteger(to) || from < 1 || to < from || to - from >= MOST) fail(400, "steps are { steps, from, to }, from 1, at most 256");
  return Array.from({ length: to - from + 1 }, (_, i) => `${steps}/${String(from + i).padStart(12, "0")}`);
}

const list = (v) => (v === undefined ? [] : Array.isArray(v) ? v : fail(400, "each kind is a list"));

/// What the relay said about each key lately: key → { at, said }.
const known = new Map();

async function allowed(env, key) {
  const now = Date.now();
  const kept = known.get(key);
  if (kept && now - kept.at < ALLOWED_FOR) return kept.said;
  const said = (await env.RELAY.allowed(key)) || null;
  if (known.size > 10_000) known.clear();
  known.set(key, { at: now, said });
  return said;
}

/// Whether each object is in the bucket with the ETag given: by the R2 binding where there is one, else by HEAD.
async function confirm(env, bucket, asked) {
  const bare = (etag) => String(etag).replace(/^W\//, "").replace(/"/g, "");
  const heads = env.R2 ? null : (await bucketUrls(bucket, { head: asked.map((a) => a.name) }, 60)).head;
  const out = {};
  await Promise.all(asked.map(async ({ name, etag }) => {
    const now = env.R2 ? (await env.R2.head(name))?.etag : await fetch(heads[name], { method: "HEAD" }).then((r) => (r.ok ? r.headers.get("etag") : null));
    out[name] = now != null && bare(now) === bare(etag);
  }));
  return out;
}

async function urls(request, env) {
  if (!env.BUCKET) fail(503, "your One App has no bucket yet");
  const body = await request.json().catch(() => fail(400, "the body isn't JSON"));
  let checked;
  try {
    checked = await checkRequest("one/urls", body);
  } catch (err) {
    fail(401, err.message);
  }
  const { by, fields } = checked;
  const who = await allowed(env, by);
  if (!who) fail(403, "only your One App's devices reach its bucket");
  const read = list(fields.read).flatMap(names);
  const head = list(fields.head).flatMap(names);
  const create = list(fields.create).flatMap(names);
  const replace = list(fields.replace);
  const remove = list(fields.remove);
  const pages = list(fields.list);
  const confirms = list(fields.confirm);
  if (read.length + head.length + create.length + replace.length + remove.length + pages.length > MOST) fail(400, `at most ${MOST} URLs at once`);
  if (confirms.length > CONFIRM) fail(400, `at most ${CONFIRM} objects confirmed at once`);
  for (const r of [...replace, ...confirms]) if (!isName(r?.name) || typeof r.etag !== "string" || !r.etag) fail(400, "give each a name and its ETag");
  if (!replace.every((r) => r.name.endsWith("/newest"))) fail(403, "only a database's newest is replaced");
  for (const name of remove) if (!isName(name)) fail(400, `${name} isn't a name in the bucket`);
  if (remove.length && !who.admin) fail(403, "only an admin's device removes from the bucket");
  for (const p of pages) {
    const ok = p && (p.prefix === "" || (typeof p.prefix === "string" && p.prefix.endsWith("/") && isName(p.prefix.slice(0, -1))));
    if (!ok || ![undefined, "/"].includes(p.delimiter) || ![undefined, null].includes(p.token) && typeof p.token !== "string") fail(400, "a list is { prefix, delimiter?, token? }, its prefix a folder's, ending in /");
  }
  const bucket = JSON.parse(env.BUCKET);
  const until = Date.now() + EXPIRES * 1000;
  const out = await bucketUrls(bucket, { read, head, create, replace, remove, list: pages }, EXPIRES);
  return { ...out, confirm: confirms.length ? await confirm(env, bucket, confirms) : {}, until };
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/urls" || request.method !== "POST") return refuse(404, "the URL signer answers only POST /urls");
    try {
      return Response.json(await urls(request, env));
    } catch (err) {
      if (err instanceof Refused) return refuse(err.status, err.message);
      console.error(err.stack || err);
      return refuse(500, "the URL signer failed");
    }
  },
};
