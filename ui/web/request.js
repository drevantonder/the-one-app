// The request path: a request to the user's devices, as this device (the platform's `fetch`: in a browser, onet's
// keyring in the SharedWorker, net.js; in the app, its own client), asked again while no device answers, watched for
// this device being taken out.
import { CODES, HEADER } from "../shared/protocol.js";
import { platform } from "./platform.js";
import { recording } from "./demo.js";
import { called, tried } from "./insights.js";

export class NodeError extends Error {}

/// Whether an app runtime has refused this client as too old to read what the network has moved on to, and who to tell.
export const tooOld = {
  now: false,
  set() {
    if (this.now) return;
    this.now = true;
    this.changed?.();
  },
};
export const UNANSWERED = "Your devices aren’t answering. Check that one is on and online.";
/// How long a request is asked again after no device answered. It doesn't limit one a device is working on.
const ASKED_AGAIN_FOR = 20_000;

/// How long to wait before asking again the `n`th time running (from 0) after no device answered: at once, then
/// a quarter second, doubling, up to five. An app runtime restarting for an update is back within a moment.
export const retryWait = (n) => (n === 0 ? 0 : Math.min(5000, 125 << Math.min(n, 6)));
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// A request to the network, as this user: JSON in and out, or with `raw` the response itself. Only a device of theirs
/// asks, of their devices, itself (the keyring says whose it is, `x-one-user`, which an app runtime checks against the device its
/// connection proves). Asked again for a few seconds while no device answers, as one restarting doesn't; a call carries an
/// id, so it runs once however often it's sent.
export async function api(path, { method = "GET", body, query, raw, signal } = {}) {
  const url = new URL(path, location.origin);
  for (const [k, v] of Object.entries(query || {})) if (v != null) url.searchParams.set(k, typeof v === "string" ? v : JSON.stringify(v));
  if (url.pathname === "/api/call" && body && typeof body === "object" && !body.id) body = { ...body, id: callId() };
  // Its client span (`insights.called`): what the devices do of it is under it. Sending what's recorded isn't recorded
  // itself, as each would be the next thing sent.
  const call = { traceparent: traceparent(), ...(url.pathname === "/api/call" ? { app: body?.app, tool: body?.tool } : { path: url.pathname }) };
  const began = performance.now();
  const ended = (more) => call.sent && url.pathname !== REPORT && called(sendReport, { ...call, sent: undefined, last: undefined, ms: Math.round(performance.now() - began), ...more });
  try {
    const out = await sending(url, method, body, raw, signal, call);
    ended({});
    return out;
  } catch (err) {
    // A device answered what has a status; anything else reached none, and says why as the last try did.
    if (!signal?.aborted && !err.removed) ended(err.status ? { failed: true, code: err.code } : { error: UNREACHED[call.last] || "unreachable", message: err.message });
    throw err;
  }
}

const hexOf = (n) => [...crypto.getRandomValues(new Uint8Array(n))].map((b) => b.toString(16).padStart(2, "0")).join("");
/// W3C's `traceparent` for a request made here.
const traceparent = () => `00-${hexOf(16)}-${hexOf(8)}-01`;

const REPORT = "/api/insights/report";
/// Sends what this client recorded (`insights.js`).
export const sendReport = (events) => api(REPORT, { method: "POST", body: { platform: platform.client(), events } });

/// Why no device answered, from how the last one tried went (onet's results), as insights names it.
const UNREACHED = { "no answer": "timeout", lost: "lost" };

/// `api`'s request, and in `call` which device answered, or how the last it tried went. The device sends it again with the keys an app runtime asks for (onet's keyring); a request that doesn't
/// reach a device is asked again here.
async function sending(url, method, body, raw, signal, call) {
  const user = platform.user();
  if (!user || !(await platform.reaches(user))) throw Object.assign(new NodeError("This device hasn’t been approved yet."), { status: 403 });
  const began = performance.now();
  call.sent = true;
  for (let misses = 0; ; ) {
    // The same for each send, so what the devices do of this one request is one trace.
    const headers = { [HEADER.traceparent]: call.traceparent };
    if (body !== undefined) headers["content-type"] = "application/json";
    const sent = body === undefined ? undefined : JSON.stringify(body);
    let r;
    const watch = watching();
    try {
      const asked = [method, url.pathname + url.search, headers, sent, signal, platform.until()];
      r = await Promise.race([recording.on() ? recording.fetch(platform.fetch, ...asked) : platform.fetch(...asked), watch.out]);
      call.node = tried(r.trace).node;
    } catch (err) {
      call.last = tried(err.trace).last;
      if (err.notLetIn) {
        if (await takenOut()) throw gone();
        throw Object.assign(new NodeError(err.message), { status: 403 });
      }
      if (err.removed || signal?.aborted) throw err;
      // A request that fails may be one a device taken out of its user's devices can't make.
      if (await takenOut()) throw gone();
      // A failure with a code is the keyring's own, said in its words; any other is the connection.
      if (misses >= 5 || performance.now() - began > ASKED_AGAIN_FOR) throw err.code ? err : Object.assign(new NodeError(UNANSWERED), { unanswered: true });
      await sleep(retryWait(misses++));
      continue;
    } finally {
      watch.stop();
    }
    if (raw && r.ok) return r;
    const out = await r.json().catch(() => null);
    if (out?.code === CODES.update) tooOld.set();
    if (!r.ok || out?.error) throw Object.assign(new NodeError(out?.error || (r.status >= 500 ? UNANSWERED : "That didn’t work. Try again.")), { status: r.status, code: out?.code, trace: out?.trace });
    return out;
  }
}

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

/// The network's state or apps, asked for once a second at most: the window and its own screens (Home, Settings) ask
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
  const same = JSON.stringify([app, tool, input]);
  if (reads && reading.has(same)) return reading.get(same);
  // A write no device answered may have run: asked again, it's the same call, so it runs once.
  const id = reads ? callId() : unanswered.get(same) || callId();
  const asked = api("/api/call", { method: "POST", body: { id, app, tool, input, client: platform.client() } }).then((r) => r.ok);
  if (reads) (reading.set(same, asked), asked.catch(() => {}).finally(() => reading.delete(same)));
  else asked.then(() => unanswered.delete(same), (err) => (err.unanswered ? unanswered.set(same, id) : unanswered.delete(same)));
  const out = await asked;
  platform.renew(app).catch((err) => console.warn(`renewing ${app}'s schedules:`, err));
  return out;
}
const reading = new Map();
const unanswered = new Map();
