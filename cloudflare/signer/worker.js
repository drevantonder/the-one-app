// The URL signer: the only holder of the network's bucket's credentials (`BUCKET`, `{ endpoint, bucket, region,
// key_id, secret }`). It gives presigned URLs to endpoints of the network, which the relay keeps the list of (`RELAY`,
// asked `allowed(key)`), for a request each signs with its own key (src/keys.rs `signed_request`, domain `one/urls`):
//
//   POST /urls { read, head, create: [item], replace: [{ name, etag }], remove: [name],
//                list: [{ prefix, delimiter?, token? }], confirm: [{ name, etag }], newest: [prefix], get: [item],
//                at, by, sig }
//   → { read, head, create, replace, remove: { name: url }, list: [url], confirm: { name: bool },
//       newest: { prefix: text | null }, get: { name: base64url | null }, until }
//
// An item is a name, or a database's steps from one position to another (`{ steps: prefix, from, to }`), so a node
// asks for the positions it reads or writes next in one trip. Any endpoint of the network may read, list and create
// (only if absent: `If-None-Match: *` is signed in); a `…/newest` is replaced only by its ETag (`If-Match`); only an
// admin's endpoint removes. A step at position n of a database or log (`<prefix>/<n>`, n > 1) is created only once the
// one before it is there, or the copy of the steps up to it (`<prefix>/copy/<n-1>`), or the database's older steps
// were deleted (`<prefix>/tidied`, src/database/stored.rs), past which its readers look for a copy that holds; and a
// copy only once its step is: so no endpoint without the keys leaves a gap in the steps its readers take for the end,
// nor plants one past it. Others asked for in the same trip are left out, and asked for again when they're next.
// `confirm` says whether each object is there with that ETag, `newest` what each database's newest hint says
// (`<prefix>/newest`, never sealed), and `get` each object's bytes, sealed as they are, for browsers, which can't read
// the bucket: all open nothing. It knows nobody's scopes and checks none: everything in the bucket is sealed, so what it
// hands out opens nothing without keys.
// A request from a device that says it reads less of who's who than this build (`reads`, shared/protocol.js's `READS`;
// an older build says nothing) is 426, telling it to update, if it would create, replace or remove, or comes from a
// client alone. A machine that runs things still reads, so a node takes the release it updates to.

import { b64u, behind, checkRequest, rolesOf } from "../shared/crypto.js";
import { COPY, MOST_URLS, NEWEST, STEP_DIGITS, URLS, copyName, newestName, stepName, tidiedName, updateRefusal } from "../shared/protocol.js";
import { bucketUrls } from "../shared/s3.js";

const EXPIRES = 15 * 60;
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

const isName = (name) => typeof name === "string" && name.length <= 1024 && name.split("/").every((p) => p && p !== "." && p !== "..");

// stepName is src/database/stored.rs's `step_name`.
function names(item) {
  if (typeof item === "string") return isName(item) ? [item] : fail(400, `${item} isn't a name in the bucket`);
  const { steps, from, to } = item || {};
  if (!isName(steps) || !Number.isSafeInteger(from) || !Number.isSafeInteger(to) || from < 1 || to < from || to - from >= MOST_URLS) fail(400, `steps are { steps, from, to }, from 1, at most ${MOST_URLS}`);
  return Array.from({ length: to - from + 1 }, (_, i) => stepName(steps, from + i));
}

const list = (v) => (v === undefined ? [] : Array.isArray(v) ? v : fail(400, "each kind is a list"));

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

async function newest(env, bucket, prefixes) {
  const names = prefixes.map(newestName);
  const reads = env.R2 ? null : (await bucketUrls(bucket, { read: names }, 60)).read;
  const out = {};
  await Promise.all(prefixes.map(async (p, i) => {
    if (env.R2) return void (out[p] = (await (await env.R2.get(names[i]))?.text()) ?? null);
    const r = await fetch(reads[names[i]]);
    if (!r.ok && r.status !== 404) throw new Error(`the bucket said ${r.status}`);
    out[p] = r.ok ? await r.text() : null;
  }));
  return out;
}

async function present(env, bucket, names) {
  const heads = env.R2 || !names.length ? null : (await bucketUrls(bucket, { head: names }, 60)).head;
  const out = new Map();
  await Promise.all(names.map(async (name) => {
    if (env.R2) return void out.set(name, (await env.R2.head(name)) !== null);
    const r = await fetch(heads[name], { method: "HEAD" });
    if (!r.ok && r.status !== 404) throw new Error(`the bucket said ${r.status}`);
    out.set(name, r.ok);
  }));
  return out;
}

/// What must be in the bucket before `name` is created, any one of: the step before a step, the copy up to it, or its
/// database's `tidied`; and a copy's own step. None for anything else, or a database's first step.
function after(name) {
  const step = new RegExp(`^(.+)/(\\d{${STEP_DIGITS}})$`).exec(name);
  if (!step) return null;
  const [, prefix, n] = step;
  const at = Number(n);
  const copy = new RegExp(`^(.+)/${COPY}$`).exec(prefix);
  if (copy) return [`${copy[1]}/${n}`];
  return at > 1 ? [stepName(prefix, at - 1), copyName(prefix, at - 1), tidiedName(prefix)] : null;
}

async function due(env, bucket, names) {
  const needs = names.map(after);
  const there = await present(env, bucket, [...new Set(needs.flat().filter(Boolean))]);
  return names.filter((_, i) => !needs[i] || needs[i].some((n) => there.get(n)));
}

async function got(env, bucket, names) {
  const reads = env.R2 ? null : (await bucketUrls(bucket, { read: names }, 60)).read;
  const out = {};
  await Promise.all(names.map(async (name) => {
    const bytes = env.R2 ? await (await env.R2.get(name))?.arrayBuffer() : await (async () => {
      const r = await fetch(reads[name]);
      if (!r.ok && r.status !== 404) throw new Error(`the bucket said ${r.status}`);
      return r.ok ? r.arrayBuffer() : null;
    })();
    out[name] = bytes ? b64u.encode(new Uint8Array(bytes)) : null;
  }));
  return out;
}

async function urls(request, env) {
  if (!env.BUCKET) fail(503, `${env.YOURS} has no bucket yet`);
  const body = await request.json().catch(() => fail(400, "the body isn't JSON"));
  let checked;
  try {
    checked = await checkRequest(URLS, body);
  } catch (err) {
    fail(401, err.message);
  }
  const { by, fields } = checked;
  const who = await allowed(env, by);
  if (!who) fail(403, `only ${env.YOURS}'s devices reach its bucket`);
  const read = list(fields.read).flatMap(names);
  const head = list(fields.head).flatMap(names);
  const create = list(fields.create).flatMap(names);
  const replace = list(fields.replace);
  const remove = list(fields.remove);
  const pages = list(fields.list);
  const confirms = list(fields.confirm);
  const hints = list(fields.newest);
  const gets = list(fields.get).flatMap(names);
  if (gets.length > 32) fail(400, "at most 32 objects got at once");
  if (read.length + head.length + create.length + replace.length + remove.length + pages.length > MOST_URLS) fail(400, `at most ${MOST_URLS} URLs at once`);
  if (confirms.length + hints.length > CONFIRM) fail(400, `at most ${CONFIRM} objects confirmed or hints read at once`);
  if (!hints.every(isName)) fail(400, "each newest is a database's prefix");
  for (const r of [...replace, ...confirms]) if (!isName(r?.name) || typeof r.etag !== "string" || !r.etag) fail(400, "give each a name and its ETag");
  if (!replace.every((r) => r.name.endsWith(`/${NEWEST}`))) fail(403, "only a database's newest is replaced");
  for (const name of remove) if (!isName(name)) fail(400, `${name} isn't a name in the bucket`);
  if (remove.length && !who.admin) fail(403, "only an admin's device removes from the bucket");
  const client = rolesOf(who.role).every((r) => r === "client");
  if ((client || create.length + replace.length + remove.length) && behind(fields.reads)) fail(426, updateRefusal(env.YOURS));
  for (const p of pages) {
    const ok = p && (p.prefix === "" || (typeof p.prefix === "string" && p.prefix.endsWith("/") && isName(p.prefix.slice(0, -1))));
    if (!ok || ![undefined, "/"].includes(p.delimiter) || ![undefined, null].includes(p.token) && typeof p.token !== "string") fail(400, "a list is { prefix, delimiter?, token? }, its prefix a folder's, ending in /");
  }
  const bucket = JSON.parse(env.BUCKET);
  const until = Date.now() + EXPIRES * 1000;
  const creates = await due(env, bucket, create);
  if (create.length && !creates.length) fail(409, "a step is written only once the one before it is there");
  const out = await bucketUrls(bucket, { read, head, create: creates, replace, remove, list: pages }, EXPIRES);
  return { ...out, confirm: confirms.length ? await confirm(env, bucket, confirms) : {}, newest: hints.length ? await newest(env, bucket, hints) : {}, get: gets.length ? await got(env, bucket, gets) : {}, until };
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
