// The request path: a request to the user's nodes, as this device (the platform's `fetch`: in a browser, one-client's
// keyring in the SharedWorker, net.js; in the app, its own client), asked again while no node answers, watched for
// this device being taken out, and traced in Developer mode.
import { HEADER } from "../shared/protocol.js";
import { platform } from "./platform.js";

export class NodeError extends Error {}
export const UNANSWERED = "Your devices aren’t answering. Make sure one of them is on and online.";

/// How long to wait before asking again the `n`th time running (from 0) after the node didn't answer: at once, then
/// a quarter second, doubling, up to five. A node restarting for an update is back within a moment.
export const retryWait = (n) => (n === 0 ? 0 : Math.min(5000, 125 << Math.min(n, 6)));
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// A request to the node, as this user: JSON in and out, or with `raw` the response itself. Only a device of theirs
/// asks, of their nodes, itself (the keyring says whose it is, `x-one-user`, which a node checks against the device its
/// connection proves). Asked again for a few seconds while no node answers, as one restarting doesn't; a call carries an
/// id, so it runs once however often it's sent.
export async function api(path, { method = "GET", body, query, raw, signal, since } = {}) {
  const url = new URL(path, location.origin);
  for (const [k, v] of Object.entries(query || {})) if (v != null) url.searchParams.set(k, typeof v === "string" ? v : JSON.stringify(v));
  if (url.pathname === "/v1/call" && body && typeof body === "object" && !body.id) body = { ...body, id: callId() };
  const trace = developer.on() ? developer.start(url.pathname, body, since) : null;
  try {
    const out = await sending(url, method, body, raw, signal, trace);
    if (trace) trace.status = raw ? out.status : 200;
    return out;
  } catch (err) {
    if (trace) trace.failed = true;
    throw err;
  } finally {
    if (trace) developer.keep(trace);
  }
}

/// `api`'s request, each send's trace kept in `trace` if it has one. The device sends it again with the keys a node
/// asks for (one-client's keyring); a request that doesn't reach a node is asked again here.
async function sending(url, method, body, raw, signal, trace) {
  const user = platform.user();
  const joining = performance.now();
  if (!user || !(await platform.reaches(user))) throw Object.assign(new NodeError("This device hasn't been approved yet."), { status: 403 });
  if (trace && performance.now() - joining >= 1) trace.network = Math.round(performance.now() - joining);
  if (trace && (trace.network || trace.first)) trace.joined = await platform.took();
  const traced = (json) => {
    const said = developer.parse(json);
    for (const f of Array.isArray(said) ? said : [said]) {
      trace.fetches.push(f);
      if (f.client) trace.keys++;
    }
  };
  for (let misses = 0; ; ) {
    const headers = {};
    if (trace) headers[HEADER.trace] = "1";
    if (body !== undefined) headers["content-type"] = "application/json";
    const sent = body === undefined ? undefined : JSON.stringify(body);
    let r;
    const watch = watching();
    try {
      r = await Promise.race([platform.fetch(method, url.pathname + url.search, headers, sent, signal, platform.until(), !!trace), watch.out]);
      if (trace) traced(r.trace);
    } catch (err) {
      if (trace) traced(err.trace);
      if (err.notLetIn) {
        if (await takenOut()) throw gone();
        throw Object.assign(new NodeError(err.message), { status: 403 });
      }
      if (err.removed || signal?.aborted || misses >= 5) throw err;
      // A request that fails may be one a device taken out of its user's devices can't make.
      if (await takenOut()) throw gone();
      await sleep(retryWait(misses++));
      continue;
    } finally {
      watch.stop();
    }
    if (raw && r.ok) return r;
    const out = await r.json().catch(() => null);
    if (!r.ok || out?.error) throw Object.assign(new NodeError(out?.error || (r.status >= 500 ? UNANSWERED : "That didn’t work. Try again.")), { status: r.status, code: out?.code });
    return out;
  }
}

/// Developer mode, on this device alone (Settings › Developer, for admins): each request asks its nodes for a trace
/// (`x-one-trace`), kept here, the latest `KEPT`, for screens to show (`traces`, Developer's Timing).
export const developer = {
  KEY: "one.developer",
  KEPT: 100,
  kept: [],
  on: () => localStorage.getItem(developer.KEY) === "1",
  set(on) {
    if (on) localStorage.setItem(developer.KEY, "1");
    else (localStorage.removeItem(developer.KEY), (developer.kept = []));
  },
  /// What a request asked, as a trace names it: a call's app and tool, a screen's app and name, else its path.
  /// Never its input or query. `since` (`{ at, perf, first }`) is when what it's for began, if earlier: a screen's
  /// page waits for this device to be let in, and the screen the window opened on from the page's start.
  start(path, body, since) {
    const what = path === "/v1/call" ? { what: "call", app: body?.app, tool: body?.tool }
      : path.startsWith("/v1/screens/") ? { what: "page", app: decodeURIComponent(path.split("/")[3] || ""), screen: decodeURIComponent(path.split("/")[4] || "") }
      : { what: "request", path };
    const now = performance.now();
    return { ...what, at: Math.round(since ? since.at : Date.now()), started: since ? since.perf : now, waited: since ? Math.round(now - since.perf) : 0, first: !!since?.first, keys: 0, fetches: [] };
  },
  /// When a screen's page was asked for: the one the window opened on, from the page's start; any other (one asked
  /// after the client's own, like Settings, where Developer mode is turned on) from when it was asked.
  firstScreen: true,
  since() {
    const first = developer.firstScreen;
    developer.firstScreen = false;
    return first ? { at: performance.timeOrigin, perf: 0, first } : { at: Date.now(), perf: performance.now() };
  },
  parse: (json) => {
    try {
      return json ? JSON.parse(json) : {};
    } catch {
      return {};
    }
  },
  keep(trace) {
    if (trace.started !== undefined) trace.ms = Math.round(performance.now() - trace.started);
    delete trace.started;
    developer.kept.push(trace);
    if (developer.kept.length > developer.KEPT) developer.kept.shift();
  },
};

const REMOVED = "This device was removed. Sign in again.";
let removed = false;

/// What's done once this device is taken out (`whenRemoved`): it's signed out then, not an app's problem.
const onRemoved = [];
export const whenRemoved = (fn) => onRemoved.push(fn);

/// Whether this device isn't one of its user's devices any more, which a request can wait on for a long time
/// without an answer.
async function takenOut() {
  if (removed) return true;
  if (!(await platform.removed())) return false;
  removed = true;
  await Promise.all(onRemoved.map((fn) => fn(REMOVED)));
  return true;
}

const gone = () => Object.assign(new NodeError(REMOVED), { signedOut: true, removed: true });

/// Fails, once this device is taken out, while a request it waits on is still going: asked after a few seconds, then
/// every few more. `stop` once the request has answered.
function watching() {
  let done = false;
  const out = new Promise((_, fail) => {
    (async () => {
      for (let wait = 3000; !done; wait = 5000) {
        await sleep(wait);
        if (!done && (await takenOut())) fail(gone());
      }
    })();
  });
  out.catch(() => {});
  return { out, stop: () => (done = true) };
}

/// Names a call wherever it's sent, so sent again it runs once.
const callId = () => Array.from(crypto.getRandomValues(new Uint8Array(8)), (b) => b.toString(16).padStart(2, "0")).join("");

/// The node's state or apps, asked for once a second at most: the window and its own screens (Home, Settings) ask
/// together as it opens, and share the answer. `fresh` forgets them, once something changed.
const recent = {};
export function shared(path) {
  if (recent[path] && Date.now() - recent[path].at < 1000) return recent[path].answer;
  const answer = api(path);
  recent[path] = { at: Date.now(), answer };
  answer.catch(() => delete recent[path]);
  return answer;
}
export const fresh = () => {
  for (const path in recent) delete recent[path];
};

/// An app's tool, as its screen calls it: the same call asked again while one runs is that one, if it only `reads`.
export async function call(app, tool, input = {}, reads = false) {
  const same = reads ? JSON.stringify([app, tool, input]) : null;
  if (same && reading.has(same)) return reading.get(same);
  const asked = api("/v1/call", { method: "POST", body: { app, tool, input, client: platform.client() } }).then((r) => r.ok);
  if (same) (reading.set(same, asked), asked.catch(() => {}).finally(() => reading.delete(same)));
  const out = await asked;
  platform.renew(app).catch((err) => console.warn(`renewing ${app}'s schedules:`, err));
  return out;
}
const reading = new Map();
