import { KEPT_SPANS } from "../shared/protocol.js";

// What this client keeps of what it just did, in memory: the last minute, a few hundred events, each a closed-set name and
// number, never anything a screen or a call said. It's written out only when something goes wrong, with what led to it.
const LAST_MINUTE = 60_000;
const MOST = 300;
const kept = [];

/// Adds an event to the timeline.
export function note(event) {
  kept.push({ time: Date.now(), ...event });
  const from = Date.now() - LAST_MINUTE;
  while (kept.length > MOST || (kept.length && kept[0].time < from)) kept.shift();
}

/// A node can be restarting, or out of reach, for a minute or two, and takes an event back from the last five minutes: a
/// report no node took goes again then, with whatever's newer.
const AGAIN_AFTER = 30_000;
const KEPT_FOR = 300_000;
let sending = null;
let unsent = [];

function flush(send) {
  sending = null;
  const from = Date.now() - KEPT_FOR;
  const events = [...new Set([...unsent, ...kept])].filter((e) => e.time >= from).slice(-MOST);
  unsent = [];
  send(events).catch(() => {
    unsent = events;
    sending ||= setTimeout(() => flush(send), AGAIN_AFTER);
  });
}

/// Adds `event`, something that went wrong, and writes out the timeline through `send` (`/api/insights/report`'s), soon: what
/// goes wrong together goes in one report.
export function report(send, event) {
  note(event);
  sending ||= setTimeout(() => flush(send), 1000);
}

/// A request's `trace` (`{ tries }`), which a shell gives as text.
function triesOf(trace) {
  let said = trace;
  if (typeof trace === "string") {
    try {
      said = JSON.parse(trace);
    } catch {
      return [];
    }
  }
  return Array.isArray(said?.tries) ? said.tries : [];
}

/// The devices a request asked, for the timeline: → the one that answered it, if one did, and how the last try went.
export function tried(trace) {
  const tries = triesOf(trace);
  for (const t of tries) note({ name: "client.try", node: t.node, result: t.result, ms: t.ms });
  return { node: tries.findLast((t) => t.result === "answered")?.node, last: tries.at(-1)?.result };
}

/// What this client asked of its nodes, as OTEL's client spans (`client.call`), each the parent of what the node that took
/// it did: only what failed, was slow, or is one trace in 20, as a node keeps its own (`KEPT_SPANS`), so a phone's battery and
/// the bucket aren't spent on what went well. Sent about once a minute, and, as a report is, again while no node takes
/// them, for as long as a node takes them back (a day).
const CALLS_EVERY = 60_000;
const CALLS_KEPT_FOR = 86_400_000;
const sampled = (traceparent) => parseInt(traceparent.split("-")[1].slice(-8), 16) % KEPT_SPANS.sampledOneIn === 0;
let calls = [];
let callsSending = null;

function sendCalls(send) {
  callsSending = null;
  const from = Date.now() - CALLS_KEPT_FOR;
  const events = calls.filter((e) => e.time >= from).slice(-MOST);
  calls = [];
  if (!events.length) return;
  send(events).catch(() => {
    calls = [...events, ...calls].slice(-MOST);
    callsSending ||= setTimeout(() => sendCalls(send), AGAIN_AFTER);
  });
}

/// Adds a request that ended, if it's kept: `{ traceparent, ms }`, a call's `app` and `tool` or else its `path`, the
/// `node` that answered it, and `failed` with its `code` if what it answered was a failure, or, if no node answered,
/// why (`error`) and what that said (`message`).
export function called(send, call) {
  if (!call.failed && !call.error && call.ms < KEPT_SPANS.slow && !sampled(call.traceparent)) return;
  calls.push({ name: "client.call", time: Date.now() - call.ms, ...call });
  if (calls.length > MOST) calls.shift();
  callsSending ||= setTimeout(() => sendCalls(send), CALLS_EVERY);
}
