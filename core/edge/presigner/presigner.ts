// The presigner: the only holder of the bucket's credentials. A server asks it to sign one request to the bucket, and
// it does if that server is on the list of whoever the blob belongs to. It never signs a write without a condition
// (only if absent, or only if unchanged), so nothing in a network overwrites a blob blind: the condition is a
// signed header, which the bucket refuses to go without. It also says who the network's own servers and clients
// are, for the other pieces here and for a client (`Network`), and keeps the blobs a plugin's edge function
// writes for its plugin's servers (`Own`). It keeps nothing itself and runs no plugin code. Another Worker is
// given only `Network` or `Own`: nothing but an ask signed by a server's key reaches the rest.
import { WorkerEntrypoint } from "cloudflare:workers";
import { AwsClient } from "aws4fetch";
import { verifiedKey, isKey, signedBy } from "../ask.ts";

/** How long a signed request works for, in seconds. */
const SIGNED_FOR = 60;
/** The network's own prefix. A server on its list acts for the network, and reaches every user's and group's blobs. */
const NETWORK = "network/";
/**
 * A server's own blobs are under its key. It reaches them, and the network's other servers read them: that's how
 * they learn where it is.
 */
const SERVER = /^servers\/([0-9a-f]{64})\//;
/**
 * What every server of the network reads, whoever's it is: a plugin's code, which is open to everyone in the
 * network, what's installed for whom, which servers act for whom, who the network's first owner is, the builds its
 * machines take, and what each owner has deployed. That's how a server runs a plugin someone else deployed, finds
 * the server to pass a call on to, knows who may do anything where no plugin decides who may, updates, and lists
 * every app there is.
 */
const OPEN = /^(?:(?:(?:users|groups)\/[^/]+\/|network\/)(?:plugins\/.*|installed|servers)|network\/owner|network\/(?:builds|deployed)\/.*)$/;
/**
 * That a user, a group or the network installed a plugin is a blob under the plugin, `installed/<who>`, which a server
 * acting for them writes and removes, whoever's the plugin is: a deploy of it tells their servers.
 */
const INSTALLED = /^(?:(?:users|groups)\/[^/]+\/|network\/)plugins\/[^/]+\/installed\/((?:users|groups)\/[^/]+|network)$/;
/**
 * That a user, a group or the network deployed a plugin is a blob of the network's, `deployed/<who>/<name>`, with
 * the plugin's apps, which a server acting for them writes as it deploys: so every app there is is listed in one
 * place, whichever pages it's on.
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
function said(token: string): Token {
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
    if (!by) return refuse(401, "Sign what you ask with your key, by a clock within a minute of the right time");
    if (pathname === "/presigner/sign" && request.method === "POST") return sign(env, by, request.headers.get("one-token"), body);
    if (pathname === "/presigner/servers" && request.method === "GET") {
      if (!(await admits(env, by, request.headers.get("one-token")))) return refuse(403, "This network doesn't know you");
      // Whoever's servers are asked for, the ones acting for the network act for them too.
      const acting = [...(await servers(env, searchParams.get("of"))), ...(await servers(env, NETWORK))];
      return Response.json({ servers: [...new Set(acting)] });
    }
    return refuse(404, "The presigner signs at /presigner/sign and lists servers at /presigner/servers");
  },
} satisfies ExportedHandler<PresignerEnv>;

/** Who the network's own are, for another piece of the edge. */
export class Network extends WorkerEntrypoint<PresignerEnv> {
  /** The keys of the servers that act for a user or group, or for the network, by its prefix. */
  servers(of: string | null): Promise<string[]> {
    return servers(this.env, of);
  }

  /** Whether `key` is one of the network's own, by the token it shows: `null`, or `{until}` for a token that ends. */
  admits(key: string, token: string | null): Promise<Admitted | null> {
    return admits(this.env, key, token);
  }

  /** The key that signed what was asked at `url` with `body`, by the `one-` headers it came with, or null. */
  verifiedKey(url: string, headers: HeadersInit, body: string): Promise<string | null> {
    return verifiedKey(new Request(url, { headers }), body);
  }
}

/**
 * A plugin's edge function keeps blobs in its plugin's own artifact for the network, which is where its
 * plugin's servers read them, and reaches nothing else in the bucket. Which plugin is said by the binding it was
 * given (`props`), which only whoever deploys the edge sets.
 */
export class Own extends WorkerEntrypoint<PresignerEnv, { plugin: string }> {
  /** Writes the blob `name`, only if there's none. Gives whether it wrote it. */
  async write(name: string, data: BodyInit): Promise<boolean> {
    const wrote = await bucket(this.env).fetch(this.blob(name), { method: "PUT", body: data, headers: { "if-none-match": "*" } });
    if (!wrote.ok && wrote.status !== 412) throw new Error(`The bucket answered ${wrote.status} for ${name}`);
    return wrote.ok;
  }

  /** Removes the blob `name`. */
  async remove(name: string): Promise<void> {
    const removed = await bucket(this.env).fetch(this.blob(name), { method: "DELETE" });
    if (!removed.ok && removed.status !== 404) throw new Error(`The bucket answered ${removed.status} for ${name}`);
  }

  blob(name: string) {
    if (!isBlob(name)) throw new Error("That isn't a blob's name");
    return `${this.env.BUCKET}/${NETWORK}own/${this.ctx.props.plugin}/${name.split("/").map(encodeURIComponent).join("/")}`;
  }
}

/**
 * Signs one request to the bucket for the server `by`, which shows `token`: `{read: key}`, `{write: key}` (only if
 * absent), `{replace: key, tag}` (only if unchanged), `{list: prefix, after?}` or `{remove: key}`. Answers what to
 * send: its method, URL and headers.
 */
/** What a server asks the presigner to sign. */
interface Ask {
  read?: string
  write?: string
  replace?: string
  tag?: unknown
  list?: string
  after?: string
  remove?: string
}

async function sign(env: PresignerEnv, by: string, token: string | null, body: string) {
  let ask: Ask;
  try {
    ask = JSON.parse(body);
  } catch {
    return refuse(400, "An ask is JSON");
  }
  const key = ask.read ?? ask.write ?? ask.replace ?? ask.list ?? ask.remove;
  if (!isBlob(key, ask.list !== undefined)) return refuse(400, "That isn't a blob's key");

  const [network, owners] = await Promise.all([list(env, NETWORK), list(env, owner(key))]);
  const listed = network?.includes(by) || owners?.includes(by);
  // A new network has no server yet, so nobody is on a list. The first is whichever shows the secret this
  // edge was deployed with (`FIRST_SERVER`), which only whoever deployed it knows: it writes the network's
  // list. The secret opens nothing after that, since the bucket lets only the first write land.
  const first = ask.write === NETWORK + "servers" && same(said(token ?? "").first, env.FIRST_SERVER);
  const server = SERVER.exec(key)?.[1];
  const reads = ask.read !== undefined || ask.list !== undefined;
  const open = (server === by || (server && ask.read !== undefined) || (OPEN.test(key) && reads)) && (await admits(env, by, token));
  const installer = ((ask.write !== undefined || ask.remove !== undefined) && INSTALLED.exec(key)?.[1]) || (!reads && DEPLOYED.exec(key)?.[1]);
  const installed = installer && (await list(env, installer + "/"))?.includes(by);
  if (!listed && !first && !open && !installed) return refuse(403, "You aren't on its list of servers");

  const url = new URL(env.BUCKET);
  const headers: Record<string, string> = {};
  if (ask.list !== undefined) {
    url.search = new URLSearchParams({ "list-type": "2", prefix: key, ...(ask.after && { "continuation-token": ask.after }) }).toString();
  } else {
    url.pathname += "/" + key.split("/").map(encodeURIComponent).join("/");
    if (ask.write !== undefined) headers["if-none-match"] = "*";
    if (ask.replace !== undefined) headers["if-match"] = String(ask.tag);
  }
  url.searchParams.set("X-Amz-Expires", String(SIGNED_FOR));
  const method = ask.remove !== undefined ? "DELETE" : ask.write !== undefined || ask.replace !== undefined ? "PUT" : "GET";
  const signed = await bucket(env).sign(url, { method, headers, aws: { signQuery: true } });
  return Response.json({ method, url: signed.url, headers });
}

/**
 * Whether `key` is one of the network's own, by the token it shows: `null`, or `{until}` for a token that ends. A
 * server's token names a list it's on (`{list}`). A device's is signed by a server on a list, for that device's
 * key, until a time (`{list, by, until, signature}`): how a device gets one is the server's to decide. A run of a
 * client on a device has a key of its own, and shows the device's token, the device's key and the device's word for
 * the run's key, joined by dots (`core/client/src/lib.rs`).
 */
/** What a token admits: a server's, for as long as it's on its list (`{}`), and a device's, until a time. */
type Admitted = { until?: number }

async function admits(env: PresignerEnv, key: string, shown: string | null): Promise<Admitted | null> {
  const [token, device = key, attestation] = (shown ?? "").split(".");
  const { list, by, until, signature } = said(token);
  if (!isKey(key) || owner(list) !== list) return null;
  if (by === undefined) return (await servers(env, list)).includes(key) ? {} : null;
  if (!(until !== undefined && until > Date.now())) return null;
  if (device !== key && !(await signedBy(device, attestation, `one/run/1\n${key}`))) return null;
  if (!(await signedBy(by, signature, `one/token/1\n${device}\n${list}\n${until}`))) return null;
  return (await servers(env, list)).includes(by) ? { until } : null;
}

/** The keys of the servers that act for a user or group, or for the network, by its prefix. */
async function servers(env: PresignerEnv, of: unknown): Promise<string[]> {
  return (owner(of) === of && (await list(env, of as string))) || [];
}

/** The list of servers kept under `prefix`, `null` if there's none yet, and nobody's for no prefix. */
async function list(env: PresignerEnv, prefix: string | undefined): Promise<string[] | null> {
  if (!prefix) return [];
  const got = await bucket(env).fetch(`${env.BUCKET}/${prefix}servers`);
  if (got.status === 404) return null;
  if (!got.ok) throw new Error(`The bucket answered ${got.status} for ${prefix}servers`);
  return got.json();
}

function bucket({ ACCESS_KEY_ID: accessKeyId, SECRET_ACCESS_KEY: secretAccessKey }: PresignerEnv) {
  return new AwsClient({ accessKeyId, secretAccessKey, service: "s3", region: "auto" });
}
