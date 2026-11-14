// The signer: the only holder of the bucket's credentials. A server asks it to sign one request to the bucket, and
// it does if that server is on the list of whoever the blob belongs to. It never signs a write without a condition
// (only if absent, or only if unchanged) and never a delete, so nothing in a network overwrites a blob blind: the
// condition is a signed header, which the bucket refuses to go without. It also says who the network's own servers
// and clients are, for the relay and for a client. It keeps nothing and runs no plugin code.
import { WorkerEntrypoint } from "cloudflare:workers";
import { AwsClient } from "aws4fetch";
import { asker, isKey, signedBy } from "../ask.js";

/** How long a signed request works for, in seconds. */
const SIGNED_FOR = 60;
/** The network's own prefix. A server on its list acts for the network, and reaches every user's and group's blobs. */
const NETWORK = "network/";
/** A blob's key says whose it is by how it starts: a user's or a group's prefix, or the network's. */
const OWNER = /^(?:(?:users|groups)\/[^/]+\/|network\/)/;
const owner = (key) => OWNER.exec(key)?.[0];
/**
 * A key is parts joined by `/`, none empty or starting with a dot: a `..` would leave the prefix that says whose
 * it is. What's listed is the start of a key, so its last part can be empty.
 */
const isBlob = (key, start) =>
  typeof key === "string" &&
  key.split("/").every((part, at, parts) => /^[^.]/.test(part) || (start && part === "" && at === parts.length - 1));

const refuse = (status, why) => Response.json({ error: why }, { status });

export default class extends WorkerEntrypoint {
  async fetch(request) {
    const { pathname, searchParams } = new URL(request.url);
    const body = await request.text();
    const by = await asker(request, body);
    if (!by) return refuse(401, "Sign what you ask with your key, by a clock within a minute of the right time");
    if (pathname === "/signer/sign" && request.method === "POST") return this.sign(by, body);
    if (pathname === "/signer/servers" && request.method === "GET") {
      if (!(await this.admits(by, request.headers.get("one-pass")))) return refuse(403, "This network doesn't know you");
      return Response.json({ servers: await this.servers(searchParams.get("of")) });
    }
    return refuse(404, "The signer signs at /signer/sign and lists servers at /signer/servers");
  }

  /**
   * Signs one request to the bucket for the server `by`: `{read: key}`, `{write: key}` (only if absent),
   * `{replace: key, tag}` (only if unchanged) or `{list: prefix, after?}`. Answers what to send: its method, URL
   * and headers.
   */
  async sign(by, body) {
    let ask;
    try {
      ask = JSON.parse(body);
    } catch {
      return refuse(400, "An ask is JSON");
    }
    const key = ask.read ?? ask.write ?? ask.replace ?? ask.list;
    if (!isBlob(key, ask.list !== undefined)) return refuse(400, "That isn't a blob's key");

    const [network, owners] = await Promise.all([this.list(NETWORK), this.list(owner(key))]);
    // A new network has no server yet. Whichever asks first writes the network's list, as its first owner is the
    // first user to sign in, and the bucket lets only one write of a blob that's absent land.
    const first = network === null && ask.write === NETWORK + "servers";
    if (!first && !network?.includes(by) && !owners?.includes(by)) return refuse(403, "You aren't on its list of servers");

    const url = new URL(this.env.BUCKET);
    const headers = {};
    if (ask.list !== undefined) {
      url.search = new URLSearchParams({ "list-type": "2", prefix: key, ...(ask.after && { "continuation-token": ask.after }) });
    } else {
      url.pathname += "/" + key.split("/").map(encodeURIComponent).join("/");
      if (ask.write !== undefined) headers["if-none-match"] = "*";
      if (ask.replace !== undefined) headers["if-match"] = String(ask.tag);
    }
    url.searchParams.set("X-Amz-Expires", SIGNED_FOR);
    const method = ask.write !== undefined || ask.replace !== undefined ? "PUT" : "GET";
    const signed = await this.bucket().sign(url, { method, headers, aws: { signQuery: true } });
    return Response.json({ method, url: signed.url, headers });
  }

  /**
   * Whether `key` is one of the network's own, by the pass it shows: `null`, or `{until}` for a pass that ends. A
   * server's pass names a list it's on (`{list}`). A client's is signed by a server on a list, for that client's
   * key, until a time (`{list, by, until, signature}`): how a client gets one is the server's to decide.
   */
  async admits(key, pass) {
    let said;
    try {
      said = JSON.parse(atob(pass.replace(/-/g, "+").replace(/_/g, "/")));
    } catch {
      return null;
    }
    const { list, by, until, signature } = said ?? {};
    if (!isKey(key) || owner(list) !== list) return null;
    if (by === undefined) return (await this.servers(list)).includes(key) ? {} : null;
    if (!(until > Date.now())) return null;
    if (!(await signedBy(by, signature, `one/pass/1\n${key}\n${list}\n${until}`))) return null;
    return (await this.servers(list)).includes(by) ? { until } : null;
  }

  /** The keys of the servers that act for a user or group, or for the network, by its prefix. */
  async servers(of) {
    return (owner(of) === of && (await this.list(of))) || [];
  }

  /** The list of servers kept under `prefix`, `null` if there's none yet, and nobody's for no prefix. */
  async list(prefix) {
    if (!prefix) return [];
    const got = await this.bucket().fetch(`${this.env.BUCKET}/${prefix}servers`);
    if (got.status === 404) return null;
    if (!got.ok) throw new Error(`The bucket answered ${got.status} for ${prefix}servers`);
    return got.json();
  }

  bucket() {
    const { ACCESS_KEY_ID: accessKeyId, SECRET_ACCESS_KEY: secretAccessKey } = this.env;
    return new AwsClient({ accessKeyId, secretAccessKey, service: "s3", region: "auto" });
  }
}
