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

/// What's in the timeline, oldest first.
export const timeline = () => kept.slice();

let sending = null;
/// Adds `event`, something that went wrong, and writes out the timeline through `send` (`/v1/insights/report`'s), soon: what
/// goes wrong together goes in one report.
export function report(send, event) {
  note(event);
  if (sending) return;
  sending = setTimeout(() => {
    sending = null;
    send(timeline()).catch(() => {});
  }, 1000);
}

/// The devices a request asked, for the timeline: a request's `trace`, which a shell gives as text.
export function tried(trace) {
  const said = typeof trace === "string" ? JSON.parse(trace || "{}") : trace;
  for (const t of Array.isArray(said?.tries) ? said.tries : []) note({ name: "client.try", node: t.node, result: t.result, ms: t.ms });
}
