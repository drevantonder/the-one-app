// Computer Use: the computers agents can use, and the tools they use them with. Agent computers are Linux computers with a
// browser on the person's servers, which the agent-computers sidecar runs (Incus, or a Mac's containers); the person's own
// devices are computers here too once they let agents use them, through the computer sidecar the Mac app hosts. Use,
// tools and screen go to a device when one is named; this app keeps who used which device when, as `session`
// entries in the person's own scope. Agent Computers (apps/agent-computers) is the person's page for them.

/// Runs one of the agent-computers sidecar's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars["agent-computers"][op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs agent-computers for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Computer Use needs a Linux server with Incus, or a Mac, and none of yours runs it.";

/// The caller's online servers that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.servers()).filter((s) => s.online && (s.sidecars || []).includes("agent-computers")).map((s) => s.name);
}

/// The caller's devices whose computer sidecar serves them, by server name: `{ server, on, title, kind, using,
/// permissions, missing }`, or `{ server, why }` for one that didn't answer.
async function devices(ctx) {
  const mine = (s) => {
    const d = (s.installed_sidecars || []).find((d) => d.name === "computer");
    return d ? !d.users || d.users.includes(ctx.me.user_name) : (s.sidecars || []).includes("computer");
  };
  const servers = (await ctx.servers()).filter((s) => s.online && mine(s));
  return Promise.all(servers.map(async (s) => {
    try {
      return { server: s.name, ...(await ctx.sidecars.computer._status({}, { server: s.name })) };
    } catch (err) {
      return { server: s.name, on: false, why: err.message };
    }
  }));
}

/// The device a computer's name means, if it's one: a server of the caller's running the computer sidecar for them.
async function device(ctx, name) {
  if (!name) return null;
  const n = String(name).toLowerCase();
  const s = (await ctx.servers()).find((s) => s.name.toLowerCase() === n && (s.sidecars || []).includes("computer"));
  return s ? s.name : null;
}

/// A device as the list shows it among the computers.
const asComputer = (d) => ({ name: d.server, server: d.server, device: true, kind: d.kind, title: d.title, state: d.on ? "running" : "off", using: d.using || null });

// The page: the person's own devices agents may use, each with a switch, a live picture while an agent uses it, and the
// recent sessions when there are any.
const PAGE = `
<style>
  body { max-width: 1120px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(240px, 40vw), 1fr)); gap: 22px 14px; margin-top: 6px; }
  .pic { aspect-ratio: 16 / 10; border-radius: 10px; overflow: hidden; background: var(--card); border: 0.5px solid var(--line); display: grid; place-items: center; color: var(--faint); }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .head { display: flex; align-items: center; gap: 8px; margin-top: 8px; min-width: 0; }
  .name { font-weight: 600; flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .switch { transform: scale(0.78); transform-origin: right center; }
  .sub { color: var(--muted); font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-height: 1.3em; }
</style>
<div class="grid" id="grid"><div class="empty" style="grid-column:1/-1">Loading…</div></div>
<div id="sessions-section" hidden>
<h2 class="section">Recent Sessions</h2>
<div class="list" id="sessions"></div>
</div>
<script>
  one.title("Computer Use");
  const grid = document.getElementById("grid");
  const pics = {}; // by server: a data URL
  const asking = new Set(); // devices this page is asking to let agents in
  let devs = [], sessions = [];
  const host = (s) => String(s || "").split(".")[0];
  const laptop = '<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>';

  // A device: its picture while an agent uses it, its name with a switch, and one short line when there's something to say.
  function tile(d) {
    const allowed = d.permissions ? d.permissions.accessibility && d.permissions.screen_recording : true;
    const line = d.why ? "Not answering"
      : asking.has(d.server) || d.asking ? "Asking on the device…"
      : !d.on ? (d.missing || "")
      : d.using ? \`\${d.using.agent} is using it\`
      : allowed ? "" : "Allow it in System Settings on the device";
    const pic = d.on && d.using && pics[d.server] ? \`<img src="\${pics[d.server]}" alt="">\` : laptop;
    const off = d.why || d.missing || asking.has(d.server);
    return \`<div><div class="pic">\${pic}</div><div class="head"><div class="name">\${one.esc(d.title || d.server)}</div>\` +
      \`<input type="checkbox" class="switch" data-device="\${one.esc(d.server)}" aria-label="Let agents use it"\${d.on ? " checked" : ""}\${off ? " disabled" : ""}></div>\` +
      \`<div class="sub">\${one.esc(line)}</div></div>\`;
  }

  function draw() {
    grid.innerHTML = devs.length ? devs.map(tile).join("")
      : \`<div class="empty" style="grid-column:1/-1">Agents use your Mac here once you let them. Install The One App on it first.</div>\`;
    for (const s of grid.querySelectorAll("[data-device]")) s.onchange = () => allow(s.dataset.device, s.checked);
    document.getElementById("sessions-section").hidden = !sessions.length;
    document.getElementById("sessions").innerHTML = sessions.slice(0, 10).map((s) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(s.agent)}</span></div>\` +
      \`<div class="sub small">\${one.esc(s.title || s.device)} · \${one.esc(one.ago(s.time, { long: true }))}\${s.came_from ? " · asked from " + one.esc(host(s.came_from)) : ""}</div></div></div>\`).join("");
  }

  async function allow(server, on) {
    if (on) asking.add(server);
    draw();
    try {
      await one.call("_allow", { device: server, on });
    } catch (err) {
      alert(err.message);
    } finally {
      asking.delete(server);
      await refresh();
    }
  }

  async function refresh() {
    try {
      ({ devices: devs, sessions } = await one.call("_devices", {}));
    } catch (e) {
      grid.innerHTML = \`<div class="empty error" style="grid-column:1/-1">\${one.esc(e.message)}</div>\`;
      return;
    }
    draw();
    await Promise.all(devs.filter((d) => d.on && d.using).map(async (d) => {
      try {
        const r = await one.call("screen", { computer: d.server, width: 480 });
        const img = (r.content || []).find((c) => c.type === "image");
        if (img) pics[d.server] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
    }));
    draw();
  }

  let busy = false;
  async function tick() {
    if (busy || document.visibilityState === "hidden") return;
    busy = true;
    try { await refresh(); } finally { busy = false; }
  }
  tick();
  setInterval(tick, 5000);
  document.addEventListener("visibilitychange", tick);
</script>`;

export default {
  name: "computer",
  title: "Computer Use",
  icon: "cursorarrow.rays",
  color: "#5e5ce6",
  description: "Computers your agents can use: agent computers with a browser, and your own devices when you let them",
  connector: true,
  needs: "Needs a Linux server with Incus, or a Mac",
  instructions: "Computer Use is the computers agents can use: agent computers (Linux, on the person's servers, each with a browser) and the person's own devices once they let agents use them. To browse, borrow an agent computer of your own: computer_borrow with for (what it's for, in a few words: the person sees it) gives you one whose browser starts with a copy of a browser profile, signed in to its sites (default unless you pass profile; agent-computers_profiles lists them, like Personal or PTC, with the sites each was used on); pass its name as computer to the other tools, and give it back with computer_give_back when done: it's deleted, with anything it did to the profile. One left 15 minutes without a call is given back for you. In it: computer_browser_prepare with allow_launch true and profile {\"mode\":\"isolated_named\",\"name\":\"default\"} (the profile's name, as computer_borrow gives it) launches the browser; computer_use with tool list_windows gives its window_id; computer_get_browser_state with that pid and window_id gives target_id and tab_id; then computer_browser_navigate, computer_browser_click and computer_browser_type act, and computer_get_browser_state reads the page. A browser left idle for 5 minutes closes; prepare it again. computer_screen shows the computer. When a site needs the person to sign in, agent-computers_sign_in with the profile and the site's url gives them a link to sign in there themselves. For a challenge in your own computer, give them computer_view's link. Never sign in or submit forms for them unless they asked. Everything else on a computer (apps, windows, keyboard, mouse): computer_tools lists the tools and computer_use runs one. computer_list shows every server's agent computers, who has borrowed which and what for; to act on one on another server, pass server too. A lasting computer nobody calls or watches for 30 minutes is paused, and unpaused by the next call. computer_list also lists the person's own devices they let agents use (device: true, like their Mac), by their server's name: computer_tools, computer_use and computer_screen work on one as on an agent computer, acting in the background where they can, but there's nothing to borrow and no view link. The person sees in their menu bar while you use their device and can stop you. Use one only for what they asked of that device, and never their messages, mail, browser sessions or files unless they asked.",
  uses: { apps: ["agent-computers"], sidecars: ["agent-computers", "computer"] },

  schema: {
    user: `create table sessions (id text primary key, device text not null, title text, agent text not null, came_from text, time integer not null)`,
  },


  ops: {
    accounts: {
      reads: true,
      description: "The servers whose computers agents can use: each { account, ok }, the server's name.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.servers()).filter((s) => (s.sidecars || []).some((d) => d === "agent-computers" || d === "computer")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      confirm: true,
      description: "Stop agents using a server's computers: removes Computer Use from that server, for one of its owners. Its computers and backups stay there.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the server, by name" } } },
      run: async (ctx, { account }) => {
        for (const d of ["agent-computers", "computer"]) if ((await ctx.servers()).some((s) => s.name === account && (s.sidecars || []).includes(d))) ctx.sidecar(d, account, false);
        return `Computer Use is removed from ${account}.`;
      },
    },
    list: {
      description: "Every server's computers: each one's server, whether it's running, paused or stopped, its size, the memory and CPU it uses now, and who has borrowed it and what for (or that it's the spare kept for borrowing).",
      input: {"properties": {"here": {"description": "only this server's", "type": "boolean"}}, "type": "object"},
      run: async (ctx, input) => {
        const [listed, devs] = await Promise.all([sidecar(ctx, "list", input).catch((err) => err), input.here ? [] : devices(ctx)]);
        const on = devs.filter((d) => d.on).map(asComputer);
        if (listed instanceof Error) {
          if (on.length && listed.message === NEEDS) return { computers: on };
          throw listed;
        }
        return on.length && Array.isArray(listed?.computers) ? { ...listed, computers: [...on, ...listed.computers] } : listed;
      },
    },
    borrow: {
      description: "Borrow an agent computer of your own for as long as you need a browser, its browser starting with a copy of a browser profile (default unless you name one; \"\" for none). Quick: a spare is kept ready. Give it back with give_back when done; it's deleted then, or after 15 minutes without a call, with anything in it.",
      input: {"properties": {"by": {"description": "who's borrowing, e.g. your app's name; default the device you call from", "type": "string"}, "for": {"description": "what you need it for, in a few words, e.g. Reading Messenger; the person sees it", "type": "string"}, "profile": {"description": "a browser profile's name, like Personal or PTC (agent-computers_profiles lists them)", "type": "string"}, "screen": {"description": "false for one with no screen to watch, only a browser", "type": "boolean"}}, "required": ["for"], "type": "object"},
      run: (ctx, input) => ctx.apps["agent-computers"].borrow(input),
    },
    give_back: {
      description: "Give back a borrowed computer: it's deleted with everything in it, its browser's changes to the profile too.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}}, "required": ["computer"], "type": "object"},
      run: (ctx, input) => ctx.apps["agent-computers"].give_back(input),
    },
    view: {
      description: "A link to watch and use a computer in a browser, for the person: to solve a challenge, or to see what an agent is doing. Works on their own devices.",
      input: {"properties": {"computer": {"description": "which computer, by name; default the only one", "type": "string"}, "minutes": {"description": "how long the link works; default 60, at most 1440", "type": "integer"}}, "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.computer);
        if (d) throw new Error(`${d} is the person's own device: they watch it on the Computer Use page and on the device itself, and stop agents there.`);
        return sidecar(ctx, "view", input);
      },
    },
    screen: {
      description: "A small picture of a computer as it is now.",
      input: {"properties": {"computer": {"description": "which computer, by name; default the only one", "type": "string"}, "width": {"description": "the picture's width in pixels; default 640, at most 1920", "type": "integer"}}, "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.computer);
        if (d) return ctx.sidecars.computer._screen({ width: input.width }, { server: d });
        return sidecar(ctx, "screen", input);
      },
    },
    tools: {
      description: "The computer's other tools, for computer_use; with name, that tool's full description and input.",
      input: {"properties": {"computer": {"description": "which computer, by name; default the only one", "type": "string"}, "name": {"description": "a tool's name", "type": "string"}}, "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.computer);
        if (d) return ctx.sidecars.computer._tools(input.name ? { name: input.name } : {}, { server: d });
        return sidecar(ctx, "tools", input);
      },
    },
    refresh_tools: {
      description: "Read the computers' tools again from a running computer, as after its image was updated.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "refresh_tools", input),
    },
    use: {
      description: "Run one of the computer's tools, as computer_tools lists them, e.g. list_windows, launch_app, click, type_text, get_guide. On a device of the person's (computer_list says device: true), the same, on that device.",
      input: {"properties": {"args": {"description": "the tool's input", "type": "object"}, "computer": {"description": "which computer, by name; default the only one", "type": "string"}, "tool": {"description": "the tool's name", "type": "string"}}, "required": ["tool"], "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.computer);
        if (!d) return sidecar(ctx, "use", input);
        const r = await ctx.sidecars.computer._use({ tool: input.tool, args: input.args || {} }, { server: d });
        if (r.started) {
          const id = [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");
          ctx.user.exec("insert or replace into sessions values (?, ?, ?, ?, ?, ?)", [id, d, r.title || null, ctx.me.agent || ctx.me.name, ctx.me.name, ctx.now()]);
        }
        return r.result;
      },
    },
    get_browser_state: {
      description: "Read-only browser inspection. Mode 1 (bind): pass pid + window_id of a native browser window to classify it, correlate it to a CDP target (exact-or-refuse), and mint a session-scoped target id plus tab ids. Mode 2 (snapshot): pass target_id + tab_id. The dom_refs_v1 compatibility format returns composed DOM refs. semantic_v2 joins accessibility, DOM, layout, and viewport state; ranks visible content before retained/offscreen state; and returns a semantic outline, typed action refs, content refs, scoped reads, and opaque continuation. Never performs setup: a browser that is not attached yet refuses with browser_consent_required or browser_requires_setup, and detail.next_call names the browser_prepare call that attaches it. Use this route for any web page in Chrome, Edge or Brave; computer pointer tools move the user's pointer. Setup and recovery: `get_guide({topic:\"browser\"})`.",
      input: {"additionalProperties": true, "properties": {"continuation": {"description": "Opaque continuation minted by an earlier semantic_v2 response.", "type": "string"}, "computer": {"description": "which computer, by name; default the only one", "type": "string"}, "include_screenshot": {"default": false, "description": "Capture the exact tab viewport as PNG through CDP without selecting the tab or foregrounding its native window. The request refuses if capture cannot be completed.", "type": "boolean"}, "pid": {"description": "Native browser process id (bind mode).", "type": "integer"}, "query": {"description": "Read-only semantic match over role, accessible name, and visible text.", "type": "string"}, "scope_ref": {"description": "Current semantic/content ref whose subtree should be observed.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "snapshot_format": {"description": "Versioned snapshot contract. dom_refs_v1 remains the compatibility default.", "enum": ["dom_refs_v1", "semantic_v2"], "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "window_id": {"description": "Native window id owned by pid (bind mode).", "type": "integer"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "get_browser_state", input),
    },
    browser_prepare: {
      description: "Attach the browser tools to a browser. For a page in the user's running Chrome, Edge or Brave, call it once with pid, window_id, session and strategy {\"kind\":\"existing_profile\"} when get_browser_state refuses with browser_consent_required or browser_requires_setup; then call get_browser_state again and act with browser_click, browser_type or browser_pointer. It attaches, or refuses with nothing changed and says why. Explicitly prepare an owned DevTools endpoint for a browser. pid is required for an existing process or existing-profile attachment, and optional only for allow_launch=true with an isolated profile. Existing endpoints are detected without side effects. Acting setup for an isolated profile follows the runtime permission mode and optional capability manifest. It requires allow_launch=true, launches a separate browser, and never copies, modifies, or terminates the requested user profile. Without pid, only a platform-attested system Chrome/Edge installation (or a root-owned package payload on Linux) is eligible; redirects and user-controlled locations fail closed. Existing-profile attachment is explicit and follows the runtime's immutable permission mode: standard requires an explicit --grant existing-profile launch grant or an embedding authorization host, bounded requires a launch-approved exact resource manifest, and unrestricted requires explicit trusted startup risk acceptance. Ordinary MCP transport approval never proves profile authorization. On proven platforms, an authorized request also permits one bounded exact-window setup: open the recognized browser product's fixed remote-debugging page, toggle its uniquely matched per-instance checkbox, prove the PID-owned loopback endpoint, and close the temporary tab. Every visible effect is reported; ambiguity is refused. Where attaching would bring the browser in front of the user's app (macOS Chrome shows its own remote-debugging prompt in front whenever the sidecar opens a new browser connection; sessions for the same browser share one approved connection, kept for up to 30 minutes idle and 8 hours in total, so later sessions do not prompt again), existing-profile attachment refuses with background_unavailable unless delivery_mode is \"foreground\" (the app that was in front is brought back afterwards) or the browser window is on another Space than the user's, where it attaches in the background while the sidecar reverts the browser's own brief activations. A sidecar-owned isolated launch result carries an advisory lifecycle object: the browser process ends with the calling session after idle_expires_in_seconds without session activity, an isolated_named profile persists after the session and an isolated_new profile does not, and explicit end, disconnect, revocation or shutdown end it earlier. Setup and recovery: `get_guide({topic:\"browser\"})`.",
      input: {"additionalProperties": true, "properties": {"allow_launch": {"description": "Allow a separate driver-owned isolated Chromium process to be launched (default false).", "type": "boolean"}, "delivery_mode": {"default": "background", "description": "Existing-profile attachment only. background (default) attaches in the background when the browser window is on another Space than the user's, and otherwise refuses with background_unavailable, before any visible effect, where attaching would bring the browser in front of the user's app. foreground accepts that, for a screen nobody else is using or when the user agreed; the app that was in front is brought back afterwards.", "enum": ["background", "foreground"], "type": "string"}, "computer": {"description": "which computer, by name; default the only one", "type": "string"}, "pid": {"description": "Browser process id to prepare. Required except for a sidecar-owned isolated_new/isolated_named launch with allow_launch=true.", "type": "integer"}, "profile": {"additionalProperties": false, "description": "Sidecar-owned isolated Chromium profile to launch with allow_launch=true. mode=isolated_new creates a fresh throwaway profile; mode=isolated_named reuses the named sidecar-owned profile. Never an existing user profile.", "properties": {"mode": {"enum": ["isolated_new", "isolated_named"], "type": "string"}, "name": {"description": "Required only for isolated_named; 1-64 path-safe ASCII characters.", "type": "string"}}, "required": ["mode"], "type": "object"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "strategy": {"additionalProperties": false, "description": "Attach to an already-running browser instead of launching one. Pass an object, {\"kind\":\"existing_profile\"}, not a string. kind=existing_profile attaches to the user's running profile at pid/window_id under the runtime's profile authorization.", "properties": {"kind": {"enum": ["existing_profile"], "type": "string"}}, "required": ["kind"], "type": "object"}, "window_id": {"description": "Exact native window approval anchor; required for strategy.kind=existing_profile.", "type": "integer"}}, "required": [], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_prepare", input),
    },
    browser_navigate: {
      description: "Navigate one tab of an exactly-bound browser target to a new URL (http/https/about only). Refused for heuristic bindings. Navigation invalidates all p<snapshot>:<index> refs for the tab.",
      input: {"additionalProperties": true, "properties": {"computer": {"description": "which computer, by name; default the only one", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "url": {"description": "Destination URL (http:, https:, or about:).", "type": "string"}}, "required": ["target_id", "tab_id", "url"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_navigate", input),
    },
    browser_click: {
      description: "Click a page element (by ref) or viewport coordinates in an exactly-bound tab. Default route is trusted hardware-like input (Input.dispatchMouseEvent), and refuses where that route cannot preserve standalone-browser background posture unless delivery_mode=\"foreground\" accepts that the browser window may activate (including Linux Chromium configurations whose trusted route cannot stay in the background; for example a browser inside a sandbox). input_route=\"dom_event\" (synthetic pointer and mouse down/up/click sequence at the element's center, ref required) is used only when explicitly requested; it proves dispatch, not control activation, because trust-gated controls may ignore synthetic events. Refused for heuristic bindings.",
      input: {"additionalProperties": true, "properties": {"delivery_mode": {"default": "background", "description": "background (default) refuses trusted input where it would activate the browser window, including Linux Chromium and macOS Chromium configurations whose trusted route cannot stay in the background. foreground accepts that activation, for a browser whose window nobody else is using (for example inside a sandbox).", "enum": ["background", "foreground"], "type": "string"}, "computer": {"description": "which computer, by name; default the only one", "type": "string"}, "input_route": {"description": "\"trusted\" (default): Input.dispatchMouseEvent. It refuses rather than foregrounding a standalone browser. \"dom_event\": synthetic full-background DOM click, only when explicitly requested. Dispatch does not prove the control activated; refresh page state and verify the expected postcondition.", "enum": ["trusted", "dom_event"], "type": "string"}, "ref": {"description": "Page element ref in the p<snapshot>:<index> namespace from get_browser_state. Refs are invalidated by navigation and by newer snapshots of the same tab.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "x": {"description": "Viewport x (CSS px) — alternative to ref.", "type": "number"}, "y": {"description": "Viewport y (CSS px) — alternative to ref.", "type": "number"}}, "required": ["target_id", "tab_id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_click", input),
    },
    browser_type: {
      description: "Type text into an exactly-bound tab via the Input domain. mode=\"insert_text\" (default) uses Input.insertText; mode=\"keystrokes\" dispatches per-character key events. Both insert at the caret, so typing into a field that already holds text appends to it; pass replace=true to set the field instead, or to clear it by typing an empty string. Pass a ref to an editable element from the latest snapshot. A ref is required; heuristic bindings are refused.",
      input: {"additionalProperties": true, "properties": {"computer": {"description": "which computer, by name; default the only one", "type": "string"}, "mode": {"description": "insert_text (default): bulk Input.insertText. keystrokes: per-character Input.dispatchKeyEvent.", "enum": ["insert_text", "keystrokes"], "type": "string"}, "ref": {"description": "Page element ref in the p<snapshot>:<index> namespace from get_browser_state. Refs are invalidated by navigation and by newer snapshots of the same tab.", "type": "string"}, "replace": {"description": "false (default): insert at the caret, appending to whatever the field already holds. true: select the element's whole content first so the text replaces it — with an empty text this clears the field. Replacement goes through the selection, so beforeinput/input still fire and framework state stays consistent.", "type": "boolean"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "text": {"description": "Text to type.", "type": "string"}}, "required": ["target_id", "tab_id", "ref", "text"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_type", input),
    },
    browser_select: {
      description: "Select one option of a native HTML <select> ref on an exactly-bound tab without opening its popup. Pass exactly one of label or value; exactly one option must equal it. Refuses non-native, multiple-selection, and disabled selects, disabled options, and zero or ambiguous matches before changing anything. A change dispatches bubbling input and change events; they are synthetic, so verify the page accepted the selection. Selecting the option that is already selected dispatches no events.",
      input: {"additionalProperties": false, "properties": {"computer": {"description": "which computer, by name; default the only one", "type": "string"}, "label": {"description": "Exact visible label of the option to select.", "type": "string"}, "ref": {"description": "Page element ref in the p<snapshot>:<index> namespace from get_browser_state. Refs are invalidated by navigation and by newer snapshots of the same tab.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "value": {"description": "Exact value attribute of the option to select.", "type": "string"}}, "required": ["target_id", "tab_id", "ref"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_select", input),
    },
    browser_dialog: {
      description: "Inspect or resolve a page-owned JavaScript alert, confirm, prompt, or beforeunload dialog on one exactly-bound tab. This never handles browser permission UI, extension UI, native dialogs, or file pickers. Inspect returns an opaque dialog_id; accept/dismiss require that exact current id. Resolution defaults to background delivery; Linux callers must explicitly request foreground delivery because Chromium's native modal cannot be resolved there without changing foreground posture.",
      input: {"additionalProperties": true, "properties": {"action": {"description": "inspect returns the current dialog and its dialog_id; accept or dismiss resolves that exact dialog.", "enum": ["inspect", "accept", "dismiss"], "type": "string"}, "delivery_mode": {"default": "background", "description": "Requested foreground posture for accept/dismiss. Linux Chromium requires foreground; inspect is read-only.", "enum": ["background", "foreground"], "type": "string"}, "computer": {"description": "which computer, by name; default the only one", "type": "string"}, "dialog_id": {"description": "Opaque current dialog generation returned by action=inspect.", "type": "string"}, "prompt_text": {"description": "Sensitive response text, valid only when accepting a prompt dialog.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}}, "required": ["target_id", "tab_id", "action"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_dialog", input),
    },
    browser_pointer: {
      description: "Perform hover, right-click, double-click, scroll, or drag in an exactly-bound browser tab. Semantic refs must declare pointer for hover, right-click, double-click, and drag; scroll accepts a scroll or pointer capability. The trusted route uses CDP Input events and refuses if standalone background posture cannot be preserved. The explicit dom_event route requires a page ref and synthesizes full-background DOM events. Never activates or brings a tab to the foreground.",
      input: {"additionalProperties": true, "properties": {"action": {"description": "Pointer gesture. scroll needs delta_x or delta_y; drag needs destination_ref or to_x/to_y.", "enum": ["hover", "right_click", "double_click", "scroll", "drag"], "type": "string"}, "delivery_mode": {"default": "background", "description": "background (default) refuses trusted input where it would activate the browser window, including Linux Chromium and macOS Chromium configurations whose trusted route cannot stay in the background. foreground accepts that activation, for a browser whose window nobody else is using (for example inside a sandbox).", "enum": ["background", "foreground"], "type": "string"}, "delta_x": {"description": "Horizontal scroll delta in CSS pixels.", "type": "number"}, "delta_y": {"description": "Vertical scroll delta in CSS pixels.", "type": "number"}, "computer": {"description": "which computer, by name; default the only one", "type": "string"}, "destination_ref": {"description": "Drag destination page ref in the exact same frame.", "type": "string"}, "input_route": {"default": "trusted", "description": "trusted sends CDP Input events; dom_event synthesizes DOM events in the page and requires ref.", "enum": ["trusted", "dom_event"], "type": "string"}, "ref": {"description": "Origin page ref. Alternative to x/y.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. This tool requires the label that owns its browser target, tab, and refs.", "type": "string"}, "tab_id": {"description": "Opaque tab id minted by get_browser_state.", "type": "string"}, "target_id": {"description": "Opaque target id minted by get_browser_state.", "type": "string"}, "to_x": {"description": "Drag destination viewport x in CSS pixels.", "type": "number"}, "to_y": {"description": "Drag destination viewport y in CSS pixels.", "type": "number"}, "x": {"description": "Origin viewport x in CSS pixels.", "type": "number"}, "y": {"description": "Origin viewport y in CSS pixels.", "type": "number"}}, "required": ["target_id", "tab_id", "session", "action"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_pointer", input),
    },
    _devices: {
      description: "The person's devices that agents may use once they let them, each with whether they do and who is using it now, and the recent sessions.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({
        devices: await devices(ctx),
        sessions: ctx.user ? await ctx.user.query("select * from sessions order by time desc limit 30") : [],
      }),
    },
    _allow: {
      description: "Lets agents use one of the person's devices, or stops them. Turning it on asks the person at the device first.",
      input: { type: "object", properties: { device: { type: "string" }, on: { type: "boolean" } }, required: ["device", "on"] },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`${input.device} can't be used by agents: no sidecar for it runs there.`);
        return ctx.sidecars.computer._allow({ on: !!input.on }, { server: d });
      },
    },
  },

  screens: {
    main: {
      title: "Computer Use",
      html: PAGE,
    },
  },
};
