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

/// The devices a request asked, for the timeline: a request's `trace`, which a shell gives as text.
export function tried(trace) {
  let said = trace;
  if (typeof trace === "string") {
    try {
      said = JSON.parse(trace);
    } catch {
      return;
    }
  }
  for (const t of Array.isArray(said?.tries) ? said.tries : []) note({ name: "client.try", node: t.node, result: t.result, ms: t.ms });
}
