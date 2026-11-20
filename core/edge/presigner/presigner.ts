// The presigner: the only holder of the bucket's credentials. A server asks it to sign one request to the bucket, and
// it does if that server is on the list of whoever the blob belongs to. A read or a list it sends to the bucket
// itself, and answers with what the bucket answered: one trip for the server rather than two, to the presigner and
// then to the bucket, and no signed request to keep for later. It never signs a write without a condition
// (only if absent, or only if unchanged), so nothing in a network overwrites a blob blind: the condition is a
// signed header, which the bucket refuses to go without. It also says who the network's own servers and clients
// are, for the rest of the edge (`servers`, `admits`), an app's part (`Network`) and a client, and keeps the blobs an
// app's part writes for its app's servers (`Own`). It keeps no blob itself, only the lists of servers it has read
// lately, and runs no app code. An app's part is given only `Network` or `Own`: nothing but an ask signed by a
// server's key reaches the rest.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import { AwsClient } from "aws4fetch";
import { verifiedKey, unsigned, isKey, signedBy, NETWORK } from "../ask.ts";

/** How long a signed request works for, in seconds. */
const SIGNED_FOR = 60;
/**
 * How long a list of servers read from the bucket is believed, in milliseconds, at most: a list is changed by a write
 * the presigner signs, which says so (`Changes`), and this is only for one that a write it didn't sign changed.
 */
const KEPT = 30_000;
/**
 * A list of servers is that of a user, a group or the network, `servers` under its prefix. Whoever writes one asks the
 * presigner to sign the write (`sign`), which is how the presigner hears that it's changing.
 */
const LIST = /^(?:(?:users|groups)\/[^/]+\/|network\/)servers$/;
/**
 * A server's own blobs are under its key. It reaches them, and the network's other servers read them: that's how
 * they learn where it is.
 */
const SERVER = /^servers\/([0-9a-f]{64})\//;
/**
 * What every server of the network reads, whoever's it is: an app's code, which is open to everyone in the
 * network, what's installed for whom, which servers act for whom, who the network's first owner is, the builds its
 * machines take, and what each owner has deployed. That's how a server runs an app someone else deployed, finds
 * the server to pass a call on to, knows who may do anything where no app decides who may, updates, and lists
 * every app there is.
 */
const OPEN = /^(?:(?:(?:users|groups)\/[^/]+\/|network\/)(?:apps\/.*|installs|installed|servers)|network\/owner|network\/(?:builds|deployed)\/.*)$/;
/**
 * An app's icon, kept by its SHA-256 the first time a server gives its address (`OneCore.Apps.Icons`): what an app's
 * code says, which every server reads already. Any of the network's servers writes one, which lands only once, and
 * reads them all, a person's own as much as the network's, since each gives the icons of the apps it runs.
 */
const ICON = /^icons\/[0-9a-f]{64}$/;
/**
 * That a user, a group or the network installed an app is a blob under the app, `installed/<who>`, which a server
 * acting for them writes and removes, whoever's the app is: a deploy of it tells their servers.
 */
const INSTALLED = /^(?:(?:users|groups)\/[^/]+\/|network\/)apps\/[^/]+\/installed\/((?:users|groups)\/[^/]+|network)$/;
/**
 * That a user, a group or the network deployed an app is a blob of the network's, `deployed/<who>/<name>`, naming
 * the app, which a server acting for them writes as it deploys: so every app there is is listed in one place,
 * whichever pages it's on, as the app's `latest` describes it.
 */
const DEPLOYED = /^network\/deployed\/((?:users|groups)\/[^/]+|network)\/[^/]+$/;
/** A blob's key says whose it is by how it starts: a user's or a group's prefix, or the network's. */
const OWNER = /^(?:(?:users|groups)\/[^/]+\/|network\/)/;
const owner = (key: unknown) => OWNER.exec(String(key))?.[0];
/**
 * A key is parts joined by `/`, none empty or starting with a dot: a `..` would leave the prefix that says whose
 * it is. What's listed is the start of a key, so its last part can be empty.
 */
const isBlob = (key: unknown, start = false): key is string =>
  typeof key === "string" &&
  key.split("/").every((part, at, parts) => /^[^.]/.test(part) || (start && part === "" && at === parts.length - 1));

const refuse = (status: number, why: string) => Response.json({ error: why }, { status });

/** What a token says: a server's names a list it's on, and a device's is signed by a server for a while. */
interface Token {
  list?: string
  by?: string
  until?: number
  signature?: string
  first?: string
}

/** What a token says, or nothing if it isn't one. */
function claims(token: string): Token {
  try {
    return JSON.parse(atob(token.replace(/-/g, "+").replace(/_/g, "/"))) ?? {};
  } catch {
    return {};
  }
}

const utf8 = new TextEncoder();
/** Whether two secrets are the same, in a time that doesn't say how alike they are. */
function same(one: unknown, other: unknown) {
  if (typeof one !== "string" || typeof other !== "string") return false;
  const [a, b] = [utf8.encode(one), utf8.encode(other)];
  return a.length === b.length && crypto.subtle.timingSafeEqual(a, b);
}

export default {
  async fetch(request, env) {
    const { pathname, searchParams } = new URL(request.url);
    const body = await request.text();
    const by = await verifiedKey(request, body);
    if (!by) return unsigned(request);
    const token = request.headers.get("one-token");
    if (pathname === "/presigner/sign" && request.method === "POST") return sign(env, by, token, body);
    if (pathname === "/presigner/read" && request.method === "POST") return read(env, by, token, body);
    if (pathname === "/presigner/servers" && request.method === "GET") {
      const lists = reader(env);
      if (!(await admits(lists, by, token))) return refuse(403, "This network doesn't know you");
      // Whoever's servers are asked for, the ones acting for the network act for them too.
      const acting = await Promise.all([servers(lists, searchParams.get("of")), servers(lists, NETWORK)]);
      return Response.json({ servers: [...new Set(acting.flat())] });
    }
    return refuse(404, "The presigner signs at /presigner/sign, reads at /presigner/read and lists servers at /presigner/servers");
  },
} satisfies ExportedHandler<Env>;

/** Who the network's own are, for an app's part. */
export class Network extends WorkerEntrypoint<Env> {
  /** The keys of the servers that act for a user or group, by its prefix, or for the network, with none. */
  servers(of: string | null = NETWORK): Promise<string[]> {
    return servers(reader(this.env), of);
  }

  /** Whether `key` is one of the network's own, by the token it shows: `null`, or `{until}` for a token that ends. */
  admits(key: string, token: string | null): Promise<Admitted | null> {
    return admits(reader(this.env), key, token);
  }

  /** The network's first owner, `users/<name>`, as One Core keeps it (`OneCore.User.first_owner/0`), or null while it has none. */
  async owner(): Promise<string | null> {
    const got = await bucket(this.env).fetch(blob(this.env, `${NETWORK}owner`));
    if (got.status === 404) return null;
    if (!got.ok) throw new Error(`The bucket answered ${got.status} for ${NETWORK}owner`);
    return got.text();
  }

  /** The key that signed what was asked at `url` with `body`, by the `one-` headers it came with, or null. */
  verifiedKey(url: string, headers: HeadersInit, body: string): Promise<string | null> {
    return verifiedKey(new Request(url, { headers }), body);
  }

  /**
   * The answer to what was asked at `url` with `headers` whose signature `verifiedKey` refused, as the edge gives it
   * (`unsigned`), by which a server whose clock is off corrects it (`OneCore.Edge.ask!/2`).
   */
  unsigned(url: string, headers: HeadersInit): Response {
    return unsigned(new Request(url, { headers }));
  }
}

/**
 * An app's edge function keeps blobs in its app's own artifact for the network, which is where its
 * app's servers read them, and reaches nothing else in the bucket. Which app is said by the binding it was
 * given (`props`), which only whoever deploys the edge sets.
 */
export class Own extends WorkerEntrypoint<Env, { app: string }> {
  /** Writes the blob `name`, only if there's none. Gives whether it wrote it. */
  async write(name: string, data: BodyInit): Promise<boolean> {
    const wrote = await bucket(this.env).fetch(this.blob(name), { method: "PUT", body: data, headers: { "if-none-match": "*" } });
    if (!wrote.ok && wrote.status !== 412) throw new Error(`The bucket answered ${wrote.status} for ${name}`);
    return wrote.ok;
  }

  /**
   * Changes the blob `name` in one step, as `OneCore.Artifact.update/3` does, and gives what it wrote: `change` is
   * given what's there, or null for nothing, and gives what's to be there. If someone else wrote it meanwhile,
   * `change` is given theirs and asked again, so neither change is lost: the bucket writes only on a condition, its
   * ETag or nothing there.
   */
  async update(name: string, change: (was: string | null) => string | Promise<string>): Promise<string> {
    const at = this.blob(name);
    for (;;) {
      const got = await bucket(this.env).fetch(at);
      if (!got.ok && got.status !== 404) throw new Error(`The bucket answered ${got.status} for ${name}`);
      const was = got.ok ? await got.text() : null;
      const now = await change(was);
      if (now === was) return now;
      const condition: Record<string, string> = was === null ? { "if-none-match": "*" } : { "if-match": got.headers.get("etag")! };
      const wrote = await bucket(this.env).fetch(at, { method: "PUT", body: now, headers: condition });
      if (wrote.ok) return now;
      if (wrote.status !== 412 && wrote.status !== 404) throw new Error(`The bucket answered ${wrote.status} for ${name}`);
    }
  }

  /** Removes the blob `name`. */
  async remove(name: string): Promise<void> {
    const removed = await bucket(this.env).fetch(this.blob(name), { method: "DELETE" });
    if (!removed.ok && removed.status !== 404) throw new Error(`The bucket answered ${removed.status} for ${name}`);
  }

  blob(name: string) {
    if (!isBlob(name)) throw new Error("That isn't a blob's name");
    return blob(this.env, `${NETWORK}own/${this.ctx.props.app}/${name}`);
  }
}

const OPERATIONS = ["read", "write", "replace", "list", "remove"] as const;

/** What a server asks of the bucket: one operation, on a blob's key, or for a list the start of the keys it lists. */
interface Ask {
  operation: (typeof OPERATIONS)[number]
  key: string
  /** A replace's: the tag of the blob it replaces. */
  tag?: unknown
  /** A list's: where the keys it lists start, as the bucket said with the ones before. */
  after?: string
}

/**
 * The ask in `body`, or null if it isn't one: `{read: key}`, `{write: key}` (only if absent), `{replace: key, tag}`
 * (only if unchanged), `{list: prefix, after?}` or `{remove: key}`. It names one operation, so that what's checked
 * is what's signed.
 */
function parse(body: string): Ask | null {
  let ask;
  try {
    ask = JSON.parse(body);
  } catch {
    return null;
  }
  const named = OPERATIONS.filter((operation) => ask?.[operation] !== undefined);
  if (named.length !== 1) return null;
  const [operation] = named;
  const key = ask[operation];
  // A replace names the version it replaces: `*`, any version, would be a write that overwrites blind.
  if (operation === "replace" && !(typeof ask.tag === "string" && ask.tag !== "" && ask.tag.trim() !== "*")) return null;
  return isBlob(key, operation === "list") ? { operation, key, tag: ask.tag, after: ask.after } : null;
}

/**
 * Signs one request to the bucket for the server `by`, which shows `token`, if it may send it. Answers what to send:
 * its method, URL and headers.
 */
async function sign(env: Env, by: string, token: string | null, body: string) {
  const ask = parse(body);
  if (!ask) return refuse(400, "An ask is JSON that names one blob's key, to read, write, replace, list or remove");
  if (!(await allowed(env, by, token, ask))) return refuse(403, "You aren't on its list of servers");
  // Said before it's signed, so no list read from now on is believed until what's signed has had its time to land.
  if (LIST.test(ask.key)) await changes(env).hold(ask.operation === "write" ? "absent" : "any");
  const { method, url, headers } = request(env, ask);
  url.searchParams.set("X-Amz-Expires", String(SIGNED_FOR));
  const signed = await bucket(env).sign(url, { method, headers, aws: { signQuery: true } });
  return Response.json({ method, url: signed.url, headers });
}

/**
 * Sends a read or a list to the bucket for the server `by`, which shows `token`, if it may send it. Answers with
 * what the bucket answered: its status, the blob's tag, and the blob or the list. The bucket is asked while the
 * lists of servers are, so this takes as long as the slower of the two, and an answer the server may not have is
 * dropped unread: a refusal comes as soon as the lists say so, however large or absent the blob.
 */
async function read(env: Env, by: string, token: string | null, body: string) {
  const ask = parse(body);
  if (!ask || (ask.operation !== "read" && ask.operation !== "list")) {
    return refuse(400, "An ask to read is JSON that names one blob's key to read, or the start of the keys to list");
  }
  const { method, url, headers } = request(env, ask);
  const stop = new AbortController();
  const response = bucket(env).fetch(url, { method, headers, signal: stop.signal });
  // One dropped for a server that may not have it isn't an error; awaited below, one that failed still throws.
  response.catch(() => {});
  if (!(await allowed(env, by, token, ask))) {
    stop.abort();
    return refuse(403, "You aren't on its list of servers");
  }
  const got = await response;
  const tag = got.headers.get("etag");
  // Only what the presigner says itself is JSON, so a server tells it from the bucket's answer by its type. A blob's
  // own type isn't passed on, so nothing between here and the server takes it for a page to change.
  return new Response(got.body, { status: got.status, headers: { "content-type": "application/octet-stream", ...(tag && { etag: tag }) } });
}

/** Whether the server `by`, which shows `token`, may send the request to the bucket that `ask` is. */
async function allowed(env: Env, by: string, token: string | null, { operation, key }: Ask) {
  const server = SERVER.exec(key)?.[1];
  const reads = operation === "read" || operation === "list";
  const icon = ICON.test(key) && (operation === "read" || operation === "write");
  const opens = server === by || (server !== undefined && operation === "read") || (OPEN.test(key) && reads) || icon;
  const installer = ((operation === "write" || operation === "remove") && INSTALLED.exec(key)?.[1]) || (!reads && DEPLOYED.exec(key)?.[1]);
  // A new network has no server yet, so nobody is on a list. The first is whichever shows the secret this
  // edge was deployed with (`FIRST_SERVER`), which only whoever deployed it knows: it writes the network's
  // list. The secret opens nothing after that, since the bucket lets only the first write land.
  const first = operation === "write" && key === NETWORK + "servers" && same(claims(token ?? "").first, env.FIRST_SERVER);
  // Every list that could let it is read at once, each once: one trip to the bucket.
  const lists = reader(env);
  const [network, owners, open, installed] = await Promise.all([
    lists(NETWORK),
    lists(owner(key)),
    opens ? admits(lists, by, token) : null,
    installer ? lists(installer + "/") : null,
  ]);
  return Boolean(network?.includes(by) || owners?.includes(by) || first || open || installed?.includes(by));
}

/** The request to the bucket that `ask` is, unsigned: its method, URL and headers. */
function request(env: Env, { operation, key, tag, after }: Ask) {
  const url = new URL(operation === "list" ? env.BUCKET : blob(env, key));
  if (operation === "list") url.search = new URLSearchParams({ "list-type": "2", prefix: key, ...(after && { "continuation-token": after }) }).toString();
  const headers: Record<string, string> = {};
  if (operation === "write") headers["if-none-match"] = "*";
  if (operation === "replace") headers["if-match"] = String(tag);
  const method = operation === "remove" ? "DELETE" : operation === "write" || operation === "replace" ? "PUT" : "GET";
  return { method, url, headers };
}

/** A blob's address in the bucket, each part of its key as it is, so the bucket reads no `%2F` in one as a `/`. */
const blob = (env: Env, key: string) => `${env.BUCKET}/${key.split("/").map(encodeURIComponent).join("/")}`;

/** What a token admits: a server's, for as long as it's on its list (`{}`), and a device's, until a time. */
type Admitted = { until?: number }

/**
 * Whether `key` is one of the network's own, by the token it shows: `null`, or `{until}` for a token that ends. A
 * server's token names a list it's on (`{list}`). A device's is signed by a server on a list, for that device's
 * key, until a time (`{list, by, until, signature}`): how a device gets one is the server's to decide. A run of a
 * client on a device has a key of its own, and shows the device's token, the device's key and the device's word for
 * the run's key, joined by dots (`core/client/src/lib.rs`).
 */
export async function admits(lists: Lists, key: string, shown: string | null): Promise<Admitted | null> {
  const [token, device = key, attestation] = (shown ?? "").split(".");
  const { list, by, until, signature } = claims(token);
  if (!isKey(key) || owner(list) !== list) return null;
  if (by === undefined) return (await servers(lists, list)).includes(key) ? {} : null;
  if (!(until !== undefined && until > Date.now())) return null;
  if (device !== key && !(await signedBy(device, attestation, `one/run/1\n${key}`))) return null;
  if (!(await signedBy(by, signature, `one/token/1\n${device}\n${list}\n${until}`))) return null;
  return (await servers(lists, list)).includes(by) ? { until } : null;
}

/** The keys of the servers that act for a user or group, or for the network, by its prefix. */
export async function servers(lists: Lists, of: unknown): Promise<string[]> {
  return (owner(of) === of && (await lists(of as string))) || [];
}

/** The lists of servers, by their prefixes, as one request reads them. */
type Lists = (prefix: string | undefined) => Promise<string[] | null>;

/** A list of servers as the bucket gave it, and when it was asked for. */
const kept = new Map<string, { at: number; list: string[] | null }>();

/**
 * Says when a list of servers may have changed, for every copy of the presigner, which each keep what they've read in
 * memory that no copy can reach into. A write to a list is signed for `SIGNED_FOR` seconds, so a list read before that
 * has passed may not be the one that's there then. It's held back from belief until then, and read from the bucket
 * again once it has, so a server taken off a list is refused on its next request, not when `KEPT` ends. A Durable
 * Object is one place that every copy asks, and answers in the order things happened.
 *
 * A write that only lands if there's no list yet (`write`) can't change one that's there, and a server that starts
 * signs one each time (`OneCore.Servers.joined?/2`), so that holds back only the lists that weren't there.
 */
export class Changes extends DurableObject<Env> {
  /** When lists read at or before it aren't believed, in milliseconds: any, and the ones that weren't there. */
  async until(): Promise<{ any: number; absent: number }> {
    const held = await this.ctx.storage.get<number>(["any", "absent"]);
    return { any: held.get("any") ?? 0, absent: held.get("absent") ?? 0 };
  }

  /** A write is signed now. The clocks of the copies and this one differ a little, so it's held a few seconds past that. */
  async hold(of: "any" | "absent"): Promise<void> {
    await this.ctx.storage.put(of, Date.now() + (SIGNED_FOR + 5) * 1000);
  }
}

const changes = (env: Env) => env.CHANGES.getByName("lists");

/**
 * Reads the lists of servers for one request: each from the bucket once, however often it's asked for, and from
 * what an earlier request read if that was read since any change to it and less than `KEPT` ago. What's kept is the
 * list, never a request for it: a promise made in one request can't be settled in another that has ended.
 */
export function reader(env: Env): Lists {
  const read = new Map<string | undefined, Promise<string[] | null>>();
  let until: ReturnType<Changes["until"]> | undefined;
  const get = async (prefix: string) => {
    const [at, held] = [Date.now(), await (until ??= changes(env).until())];
    const had = kept.get(prefix);
    if (had && had.at > (had.list ? held.any : Math.max(held.any, held.absent)) && at - had.at < KEPT) return had.list;
    const found = await list(env, prefix);
    for (const [old, { at: read }] of kept) if (at - read >= KEPT) kept.delete(old);
    kept.set(prefix, { at, list: found });
    return found;
  };
  return (prefix) => read.get(prefix) ?? read.set(prefix, prefix ? get(prefix) : Promise.resolve([])).get(prefix)!;
}

/** The list of servers kept under `prefix`, `null` if there's none yet. */
async function list(env: Env, prefix: string): Promise<string[] | null> {
  const got = await bucket(env).fetch(blob(env, `${prefix}servers`));
  if (got.status === 404) return null;
  if (!got.ok) throw new Error(`The bucket answered ${got.status} for ${prefix}servers`);
  return got.json();
}

/** One client for the isolate, which keeps the signing key it derives for each day. */
let client: AwsClient | undefined;
function bucket({ ACCESS_KEY_ID: accessKeyId, SECRET_ACCESS_KEY: secretAccessKey }: Env) {
  return (client ??= new AwsClient({ accessKeyId, secretAccessKey, service: "s3", region: "auto" }));
}
