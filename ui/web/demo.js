// Try a Demo: the app's real screens answered from a recording, with no network, sign-in or app runtime, for a store's reviewer
// (Apple's needs a way in that a work-only sign-in can't give). The recording is the build's own file (`demo` in its
// config.toml, served as /demo.json), made by using a throwaway network with sample data while `recording` is on.
//
// A recording is the calls in the order they were made. Each call that changes something starts a new stage, and a read
// answers as it was at the latest stage reached, so what a reviewer does on the recorded path shows up afterwards.
const KEY = "one.demo";
const MISSED = "That isn’t part of the demo.";

export const demoOn = () => localStorage.getItem(KEY) === "1";
export const enterDemo = () => (localStorage.setItem(KEY, "1"), location.reload());
export const leaveDemo = () => localStorage.removeItem(KEY);
/// This build's recording, if it has one.
export const demoFile = () => fetch("/demo.json").then((r) => (r.ok ? r.json() : null)).catch(() => null);

/// What identifies a call: its exact form, and the looser one a call with other input falls back to (its app and tool).
function named(method, path, body) {
  let said = body;
  try {
    const { id, client, ...rest } = JSON.parse(body);
    said = JSON.stringify(rest);
    if (path === "/api/call") return { exact: `${method} ${path} ${said}`, loose: `${rest.app}/${rest.tool}` };
  } catch {}
  return { exact: `${method} ${path} ${said ?? ""}`, loose: `${method} ${path.split("?")[0]}` };
}

const bytesOf = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));

/// `fetch` as the page's client gives it, answered from `calls`.
export function replaying(calls) {
  let stage = 0;
  const all = calls.map((c) => ({ ...c, ...named(c.method, c.path, c.body) }));
  return async (method, path, headers, body) => {
    if (path.startsWith("/api/events")) return new Response(null, { status: 204 });
    const { exact, loose } = named(method, path, body);
    let found = all.filter((c) => c.exact === exact);
    if (!found.length) found = all.filter((c) => c.loose === loose);
    if (!found.length) return new Response(JSON.stringify({ error: MISSED }), { status: 400 });
    const at = found.filter((c) => c.stage <= stage).at(-1) ?? found[0];
    const wrote = found.find((c) => c.write && c.stage > stage);
    if (wrote) stage = wrote.stage;
    const answer = wrote ?? at;
    return new Response(answer.status === 204 ? null : answer.b64 ? bytesOf(answer.b64) : answer.text, { status: answer.status, headers: answer.headers });
  };
}

/// On a throwaway network with `one.demo.record` set, every call is kept, and `oneDemo.saved()` is the file.
export const recording = {
  on: () => localStorage.getItem("one.demo.record") === "1",
  calls: [],
  async fetch(send, method, path, headers, body, ...rest) {
    const r = await send(method, path, headers, body, ...rest);
    if (path.startsWith("/api/events")) return r;
    const call = { method, path, body, status: r.status, headers: Object.fromEntries(r.headers) };
    recording.calls.push(call);
    const bytes = new Uint8Array(await r.clone().arrayBuffer());
    try {
      call.text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      call.b64 = btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join(""));
    }
    return r;
  },
  saved() {
    const apps = recording.calls.findLast((c) => c.path.split("?")[0] === "/api/apps");
    const listed = JSON.parse(apps?.text || "[]");
    const reads = (app, tool) => (Array.isArray(listed) ? listed : listed.apps || []).find((a) => a.name === app)?.tools?.some((t) => t.name === tool && t.reads === true);
    let stage = 0;
    return JSON.stringify(recording.calls.map((c) => {
      const { app, tool } = c.path === "/api/call" ? JSON.parse(c.body) : {};
      const write = !!tool && !reads(app, tool);
      if (write) stage++;
      return { ...c, write, stage };
    }));
  },
};
if (recording.on()) self.oneDemo = recording;
