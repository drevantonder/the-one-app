// Agent Devices: the devices agents use, on the person's own devices, which the agent-devices service runs: Linux with a
// browser (Podman, or a Mac's containers), with a screen or without one, and Windows, Android and iPhones where a
// device can run them (the service says which kinds each one can). Its page shows them;
// Computer Use (apps/computer) is what agents use them with. A browser profile a device starts with is a Vault item
// of type Browser profile: the service keeps it as an encrypted blob, and only it, never this app, holds its key,
// which the node fills into the calls this app makes ({{<item id>:_profile_key}}).

/// Runs one of the agent-devices service's operations, wherever it runs for the caller. When none of their devices
/// runs it, says what's needed instead of how the call failed.
async function service(ctx, op, input, on) {
  try {
    return await ctx.services["agent-devices"][op](input, on ? { device: on } : undefined);
  } catch (err) {
    if (String(err.message).includes("no device runs agent-devices for you")) throw new Error(NEEDS);
    throw err;
  }
}

/// A short hash of a string (FNV-1a), to tell whether it changed.
function fnv(s) {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return (h >>> 0).toString(36);
}

const NEEDS = "Agent Devices needs a device with Podman, or a Mac, and none of yours runs it.";

/// The service's operations Computer Use acts on agent devices with (`_act`).
const ACTS = ["list", "view", "screen", "tools", "refresh_tools", "use", "get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_dialog", "browser_pointer"];
const TYPE = "Browser profile";

/// The caller's online devices that run the service, by name.
async function running(ctx) {
  return (await ctx.devices()).filter((s) => s.online && (s.services || []).includes("agent-devices")).map((s) => s.name);
}

/// The browser profiles: the Vault items of their type this app is switched on for.
async function profiles(ctx) {
  return (await ctx.vault.items()).filter((i) => i.type === TYPE);
}

/// The browser profile named (its title, or its item's id), or the one titled default when none is named; null for "".
async function profile(ctx, name) {
  if (name === "") return null;
  const all = await profiles(ctx);
  const want = String(name ?? "default").toLowerCase();
  const found = all.find((i) => i.id === name || i.title.toLowerCase() === want);
  if (found) return found;
  if (name === undefined || name === null) return null;
  throw new Error(`No browser profile ${name}: ${all.length ? `there's ${all.map((i) => i.title).join(", ")}` : "there are none yet"}. The person adds them in the Vault.`);
}

/// A profile as the service takes it: its key is a placeholder the node fills in, for the service only.
const given = (p) => ({ item: p.id, name: p.title, key: `{{${p.id}:_profile_key}}`, blob: p.fields._blob?.value || "", salt: p.fields._salt?.value || "" });

const kindArg = { type: "string", enum: ["linux", "chrome", "windows", "android", "iphone"], description: "linux (with a browser, in a container; the default), chrome (Chrome straight on one of your devices: quicker, where its owner allows it), windows, android or iphone: kinds says which of your devices run each" };
const deviceArg = { type: "string", description: "an agent device's name, from computer_list, like pool-745f3e or windows" };
const sizeArg = { type: "string", enum: ["small", "medium", "large"], description: "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small" };

const named = { type: "string", description: "a browser profile's name, like Personal or Work: a Browser profile in the person's Vault" };

// The page: every agent device as a picture, those in use first, each with its name and one short line.
const PAGE = `
<meta name="@one/ui">
<style>
  .tile { display: block; width: 100%; min-width: 0; padding: 0; border: 0; background: none; color: inherit; font: inherit; text-align: left; }
  .tile.opens { cursor: pointer; }
  .tile:focus-visible { outline: none; }
  .tile:focus-visible .pic { outline: 3px solid color-mix(in oklab, var(--ring) 50%, transparent); }
  .pic { position: relative; aspect-ratio: 16 / 10; border-radius: calc(var(--radius) + 4px); overflow: hidden; display: grid; place-items: center; background: var(--muted); color: var(--muted-foreground); box-shadow: inset 0 0 0 1px var(--border); }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  /* A phone's whole screen, upright in the tile, like a phone. */
  .pic img.phone { position: absolute; top: 8px; left: 50%; transform: translateX(-50%); width: auto; height: calc(100% - 16px); border-radius: 12px; }
  .tile.paused .pic img { filter: grayscale(0.9) brightness(0.5); }
  .name, .line { font-size: 0.875rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name { margin-top: 0.5rem; font-weight: 500; }
  .line { min-height: 1.25rem; color: var(--muted-foreground); }
</style>
<div id="root"></div>
<script type="module">
  import { createElement as h } from "react";
  import { createRoot } from "react-dom/client";
  import { Empty, Grid, Problem } from "@one/ui";

  one.title("Agent Devices");
  const root = createRoot(document.getElementById("root"));
  const pics = {}; // by the device it's on/name: a data URL
  let devices = null; // null until first listed
  let problem = null; // what the last list failed with
  let none = false; // none of their devices runs it

  const key = (c) => \`\${c.on || ""}/\${c.name}\`;
  const at = (c) => (c.on ? { device: c.name, on: c.on } : { device: c.name });
  const host = (s) => String(s || "").split(".")[0];
  const paused = (c) => c.state !== "running";
  const screenless = (c) => c.borrowed?.no_screen;
  const capital = (s) => s.charAt(0).toUpperCase() + s.slice(1);
  const titles = { windows: "Windows", android: "Android", iphone: "iPhone" };
  const named = (c) => titles[c.name] || capital(c.name);
  const off = (c) => (c.state === "paused" ? "Paused" : "Off");
  const phone = (c) => c.kind === "android" || c.kind === "iphone";
  const glyph = (browser, c) => phone(c)
    ? '<svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="7" y="2.5" width="10" height="19" rx="2"/><path d="M11 18.5h2"/></svg>'
    : browser
    ? '<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="3" y="4.5" width="18" height="15" rx="2"/><path d="M3 8.5h18"/></svg>'
    : '<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>';

  // Its name says what it's doing, else what it's called; its line, who's using it, or that it's paused, and where,
  // when they're on more than one of the person's devices. A device with a screen opens to watch and use, here in the
  // app: a phone or Windows even when off, as it starts for whoever opens it.
  function Tile({ c, hosts }) {
    const b = c.borrowed;
    const name = b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : named(c);
    const line = [b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : paused(c) ? off(c) : "", hosts > 1 ? host(c.on) : ""].filter(Boolean).join(" · ");
    const k = key(c);
    const opens = !screenless(c) && (!paused(c) || c.kind !== "linux");
    const pic = pics[k]
      ? h("img", { src: pics[k], alt: "", className: phone(c) ? "phone" : undefined })
      : h("span", { dangerouslySetInnerHTML: { __html: glyph(screenless(c), c) } });
    return h(opens ? "button" : "div", {
      className: \`tile\${paused(c) ? " paused" : ""}\${opens ? " opens" : ""}\`,
      "data-k": k,
      onClick: opens ? () => one.open("device", at(c)) : undefined,
    }, h("div", { className: "pic" }, pic), h("div", { className: "name" }, name), h("div", { className: "line" }, line));
  }

  function Page() {
    if (none) return h(Problem, { warn: true, title: "None of your devices runs Agent Devices.", text: "It needs a Mac, or a Linux device with Podman.",
      action: { label: "Open App Store", run: () => one.open("app", { app: "agent-devices" }, "apps") } });
    // Tried again every few seconds, so no button.
    if (problem) return h(Problem, { title: problem.title, text: problem.next });
    if (!devices) return h(Empty, null, "Loading…");
    if (!devices.length) return h(Empty, null, "None yet. Agents make one when they need a browser.");
    const hosts = new Set(devices.map((c) => c.on)).size;
    const order = (c) => (c.borrowed ? 0 : paused(c) ? 2 : 1);
    const shown = [...devices].sort((a, b) => order(a) - order(b) || (Date.parse(a.borrowed?.since) || 0) - (Date.parse(b.borrowed?.since) || 0) || a.name.localeCompare(b.name));
    return h(Grid, { min: "16rem", max: 4 }, shown.map((c) => h(Tile, { key: key(c), c, hosts })));
  }
  const draw = () => root.render(h(Page));
  draw();

  // A picture of each running device with a screen that's in view, each drawn as it comes, and asked for again every
  // 20s: the service keeps the last one and takes the next in the background, and sends it only when it changed.
  const hashes = {}, asked = {}, retried = {};
  const seen = (k) => {
    const el = [...document.querySelectorAll("[data-k]")].find((t) => t.dataset.k === k);
    if (!el) return false;
    const r = el.getBoundingClientRect();
    return r.bottom > 0 && r.top < innerHeight;
  };
  function pictures() {
    for (const c of devices || []) {
      const k = key(c);
      if (paused(c) || screenless(c) || Date.now() - (asked[k] || 0) < 20000 || !seen(k)) continue;
      asked[k] = Date.now();
      one.call("_screen", { ...at(c), width: phone(c) ? 160 : 480, have: hashes[k] || "" }).then((r) => {
        // One kept from a while ago: the next, taken now, is asked for soon.
        if (r.age > 20 && !retried[k]) [retried[k], asked[k]] = [true, Date.now() - 16000];
        if (r.same) return;
        const img = (r.content || []).find((x) => x.type === "image");
        if (!img) return;
        pics[k] = \`data:\${img.mimeType};base64,\${img.data}\`;
        hashes[k] = r.hash;
        draw();
      }).catch(() => {});
    }
  }

  let listed = ""; // the list's hash: it's sent only when it changed
  async function refresh() {
    try {
      const r = await one.call("_list", { have: listed });
      if (!r.same) {
        devices = (r.devices || []).filter((c) => !c.spare || c.borrowed);
        listed = r.hash || "";
      }
      problem = null;
      draw();
    } catch (e) {
      listed = "";
      problem = one.plain(e);
      draw();
      return;
    }
    // Once drawn, so it's known which tiles are in view.
    requestAnimationFrame(pictures);
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
  addEventListener("scroll", pictures, { passive: true });
  one.call("_running").then(({ devices }) => {
    if (!devices.length) (none = true), draw();
  });
</script>`;

// One device, to watch and use: its viewer fills the screen, titled with what it's doing, under a line saying whose
// it is, with Done Signing In while it's signing in to a profile.
const DEVICE = `
<style>
  body { max-width: none; padding: 0; }
  [hidden] { display: none !important; }
  .bar { display: flex; align-items: center; gap: 12px; padding: 0 16px; height: 52px; box-sizing: border-box; }
  .bar .what { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .bar .sub { color: var(--muted); }
  iframe { position: fixed; left: 0; right: 0; bottom: 0; top: 0; width: 100%; height: 100%; border: 0; background: #000; color-scheme: dark; }
  .bar:not([hidden]) ~ iframe { top: 52px; height: calc(100% - 52px); }
  .empty { margin: 40px 16px; }
  #note:not(.empty) { margin: 16px; }
</style>
<div class="bar" id="bar" hidden><div class="what sub" id="what"></div><button class="button primary" id="done" hidden>Done Signing In</button></div>
<div class="empty" id="note">Opening…</div>
<script>
  const at = one.params.on ? { device: one.params.device, on: one.params.on } : { device: one.params.device };
  const bar = document.getElementById("bar"), what = document.getElementById("what"), done = document.getElementById("done"), note = document.getElementById("note");
  let shown = null; // the device as last listed
  let frame = null;

  function say(text, error) {
    frame?.remove();
    frame = null;
    note.hidden = false;
    note.className = error ? "" : "empty";
    if (error) note.innerHTML = one.problem({ title: text, action: { label: "Try Again", run: look } });
    else note.textContent = text;
  }

  function describe(c) {
    const b = c.borrowed;
    const name = b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : c.spare ? "Spare device" : c.name.charAt(0).toUpperCase() + c.name.slice(1);
    const line = b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : "";
    one.title(name);
    what.textContent = line;
    done.hidden = !b?.signing_in || !b.item;
    bar.hidden = !line && done.hidden;
  }

  async function look() {
    try {
      const all = (await one.call("_list", {})).devices || [];
      const c = all.find((c) => c.name === at.device && (!at.on || c.on === at.on));
      if (!c) return shown && say("It's been given back.");
      shown = c;
      describe(c);
      if (!frame) await open();
    } catch (e) {
      say(one.plain(e).title, true);
    }
  }

  async function open() {
    const r = await one.call("_view", { ...at, minutes: 240 });
    const url = typeof r === "string" ? r.split("\\n")[0] : r.url;
    if (!url || frame) return;
    note.hidden = true;
    frame = document.createElement("iframe");
    frame.allow = "clipboard-read; clipboard-write; fullscreen; microphone; keyboard-map";
    frame.src = url;
    document.body.append(frame);
    unzoom();
    frame.focus();
  }

  // The shell zooms its screens to its text size; the device's screen shows as it is, filling the window.
  function unzoom() {
    if (frame) frame.style.zoom = String(1 / (parseFloat(getComputedStyle(document.documentElement).zoom) || 1));
  }
  addEventListener("resize", unzoom);

  done.addEventListener("click", async () => {
    done.disabled = true;
    try {
      await one.call("save_profile", { item: shown.borrowed.item });
      done.hidden = true;
      say(\`Saved: the \${shown.borrowed.profile || "browser"} profile has your sign-ins now.\`);
      shown = null;
      clearInterval(timer);
    } catch (e) {
      say(one.plain(e).title, true);
    } finally {
      done.disabled = false;
    }
  });

  look();
  const timer = setInterval(look, 5000);
</script>`;

// Settings: on each of the person's machines running Agent Devices, whether agents may borrow Chrome run straight on it,
// without a container. The service keeps the switch on each machine, with its default (on where the machine runs only its
// owner's calls).
const CHROME_LOGO = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 48 48" width="28" height="28" aria-hidden="true"><defs><linearGradient id="a" x1="3.2173" y1="15" x2="44.7812" y2="15" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#d93025"/><stop offset="1" stop-color="#ea4335"/></linearGradient><linearGradient id="b" x1="20.7219" y1="47.6791" x2="41.5039" y2="11.6837" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fcc934"/><stop offset="1" stop-color="#fbbc04"/></linearGradient><linearGradient id="c" x1="26.5981" y1="46.5015" x2="5.8161" y2="10.506" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#1e8e3e"/><stop offset="1" stop-color="#34a853"/></linearGradient></defs><circle cx="24" cy="23.9947" r="12" style="fill:#fff"/><path d="M3.2154,36A24,24,0,1,0,12,3.2154,24,24,0,0,0,3.2154,36ZM34.3923,18A12,12,0,1,1,18,13.6077,12,12,0,0,1,34.3923,18Z" style="fill:none"/><path d="M24,12H44.7812a23.9939,23.9939,0,0,0-41.5639.0029L13.6079,30l.0093-.0024A11.9852,11.9852,0,0,1,24,12Z" style="fill:url(#a)"/><circle cx="24" cy="24" r="9.5" style="fill:#1a73e8"/><path d="M34.3913,30.0029,24.0007,48A23.994,23.994,0,0,0,44.78,12.0031H23.9989l-.0025.0093A11.985,11.985,0,0,1,34.3913,30.0029Z" style="fill:url(#b)"/><path d="M13.6086,30.0031,3.218,12.006A23.994,23.994,0,0,0,24.0025,48L34.3931,30.0029l-.0067-.0068a11.9852,11.9852,0,0,1-20.7778.007Z" style="fill:url(#c)"/></svg>`;
const SETTINGS = `
<style>
  .head { display: flex; align-items: center; gap: 10px; margin: 18px 4px 4px; }
  .head b { font-size: 20px; }
  .say { color: var(--muted); margin: 0 4px 14px; }
</style>
<div class="head">${CHROME_LOGO}<b>Google Chrome</b></div>
<div class="say">Chrome runs without a container. This is fine if you trust the workload.</div>
<div id="list"><div class="empty">Loading…</div></div>
<script>
  let error = "";
  const draw = () => one.render("#list", async () => {
    const { devices } = await one.call("_chrome", {});
    if (!devices.length) return one.empty("None of your devices runs Agent Devices.");
    const row = (d) => \`<label class="row plain"><div class="main"><div class="title"><span>\${one.esc(d.on)}</span></div>\${d.can ? \`<div class="sub">\${one.esc(d.can)}</div>\` : ""}</div><input type="checkbox" class="switch" data-on="\${one.esc(d.on)}"\${d.allowed ? " checked" : ""}\${d.can ? " disabled" : ""}></label>\`;
    return \`<div class="list">\${devices.map(row).join("")}</div>\${error ? one.problem({ title: error }) : ""}\`;
  });
  document.addEventListener("change", async (e) => {
    const on = e.target.dataset?.on;
    if (!on) return;
    error = "";
    try {
      await one.call("_set_chrome", { on, allowed: e.target.checked });
    } catch (err) {
      error = one.plain(err).title;
    }
    draw();
  });
  draw();
</script>`;

export default {
  name: "agent-devices",
  title: "Agent Devices",
  tagline: "Devices for AI",
  icon: "browsers",
  color: "#a2845e",
  advanced: true,
  description: "Devices your agents use: Linux with a browser, Windows, Android and iPhones",
  instructions: "Agent Devices are devices, on the person's own devices, that agents borrow (agent-devices_borrow, then agent-devices_give_back) and use through Computer Use (computer_*): Linux with a browser (the default kind), and Windows, Android and iPhone (Simulator) where one of their devices can run them, as agent-devices_kinds says. Borrow a Linux one for browsing: it's yours alone and deleted when given back. Where kinds lists chrome as available, kind chrome is quicker (under a second) and lighter: the device's own Chrome, without a container, whose borrow gives target_id and tab_id for the browser tools at once, with no browser_prepare or list_windows; its owner allows it in Agent Devices' Settings. Windows, Android and iPhone devices are the person's lasting ones (by name, like windows or android), borrowed by one agent at a time, who leaves it as they found it; for anything that changes or might break it, borrow a throwaway copy (copy: true), which is made when the lasting one is busy too if there's room. Every kind takes computer_screen, computer_view and computer_use (computer_tools lists what each kind takes), and agent-devices_open, _install and _run. Browser profiles (like Personal or Work) are Vault items of type Browser profile: a borrowed device's browser starts signed in to the sites its profile was signed in to. agent-devices_profiles lists them and the sites each was used on. When a site needs the person to sign in, agent-devices_sign_in with profile and url gives a link for them: they sign in there and it's saved when they're done (agent-devices_save_profile, or in the Vault). Never sign in for them. The rest (create, start, stop, resize, backup, restore, move, delete) is for lasting devices the person keeps.",
  uses: { services: ["agent-devices"] },
  service: { person: true, platforms: ["linux", "mac"] },
  // Browser profiles live in the Vault: the person adds one there (add_profile), signs in to sites in it (sign_in, then
  // save_profile), renames and deletes it there.
  vault: {
    profile: { title: TYPE, type: TYPE, hosts: ["agent-devices"], fields: { _profile_key: { label: "Key", secret: true } }, make: "add_profile", actions: { sign_in: "Sign In to a Site" } },
  },
  schema: {
    user: `create table saves (item text primary key, blob text not null)`,
  },


  tools: {
    borrow: {
      reads: false,
      description: "Get a device to use with computer_* for as long as you need it, and its name. Linux (the default kind): one of your own, ready at once, its browser starting with a copy of a browser profile (the one called default, unless you name one; \"\" for none). Windows, Android or iPhone: the person's lasting device, one agent at a time, or with copy, a throwaway copy of it. Give it back with agent-devices_give_back when done; a throwaway one is deleted then, or after 15 minutes without a call, with anything in it.",
      input: { type: "object", required: ["for"], properties: {
        for: { type: "string", description: "what you need it for, in a few words, e.g. Reading Messenger; the person sees it" },
        kind: kindArg,
        device: { type: "string", description: "a lasting device's name, from computer_list, like windows; default the kind's first" },
        copy: { type: "boolean", description: "a throwaway copy, not the lasting device: for anything that changes or might break it" },
        profile: named,
        screen: { type: "boolean", description: "false for a device with no screen to watch, only a browser" },
        by: { type: "string", description: "who's borrowing, as others see it, like your app's name; default the device you call from" },
      } },
      run: async (ctx, input) => {
        const other = (input.kind && input.kind !== "linux") || input.device;
        const p = other ? null : await profile(ctx, input.profile);
        const pass = Object.fromEntries(["kind", "device", "copy", "by"].filter((k) => input[k] !== undefined).map((k) => [k, input[k]]));
        return service(ctx, "borrow", { for: input.for, ...pass, ...(input.screen === false ? { screen: false } : {}), ...(p ? { profile: given(p) } : {}) });
      },
    },
    give_back: {
      reads: false,
      description: "Give back a device you borrowed: a Linux or throwaway one is deleted with everything in it, its browser's changes to the profile too; a lasting one stays as it is, for the next agent. A device signing in to a profile saves it first.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the name agent-devices_borrow gave" } } },
      run: (ctx, input) => service(ctx, "give_back", input),
    },
    profiles: {
      reads: true,
      description: "The browser profiles a borrowed device's browser can start with, like Personal and Work: each one's name, the sites it was used on and the day of each, and when it was last saved.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await profiles(ctx)).map((p) => ({ profile: p.title, sites: p.fields.sites?.value || "", saved: p.fields._saved?.value || null })),
    },
    sign_in: {
      reads: false,
      description: "For the person: opens a page in a browser on a browser profile, on a device with a screen, and gives a link for them to sign in to sites there themselves. Their sign-ins are saved when they're done (agent-devices_save_profile), or when its time is up.",
      input: { type: "object", properties: {
        profile: named,
        item: { type: "string", description: "the profile's Vault item, instead of its name" },
        url: { type: "string", description: "the page to open, like https://www.facebook.com/login" },
        minutes: { type: "integer", description: "how long until it's saved and closed; default 30, at most 240" },
      } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        const r = await service(ctx, "sign_in", { profile: given(p), ...(input.url ? { url: input.url } : {}), ...(input.minutes ? { minutes: input.minutes } : {}) });
        return { ...r, screen: { screen: "device", params: { device: r.device, on: r.on } }, then: { op: "save_profile", title: "Done Signing In" } };
      },
    },
    save_profile: {
      reads: false,
      description: "Save a browser profile's sign-ins now: from the device signing in to it, which closes, or with device, from that device's browser. Only when the person asked: borrowed devices start from saved profiles.",
      input: { type: "object", properties: { profile: named, item: { type: "string", description: "the profile's Vault item, instead of its name" }, device: deviceArg } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        return service(ctx, "save_profile", { profile: given(p), ...(input.device ? { device: input.device } : {}) });
      },
    },
    add_profile: {
      reads: false,
      description: "Add a browser profile with no sign-ins yet, as a Vault item, for the person to sign in to sites in with agent-devices_sign_in.",
      input: { type: "object", properties: { profile: { type: "string", description: "its name, like Personal or Work" }, title: { type: "string", description: "the same, as the Vault gives it" } } },
      run: async (ctx, input) => {
        const title = String(input.profile || input.title || "").trim();
        if (!title) throw new Error("Name it, like Personal or Work.");
        if ((await profiles(ctx)).some((p) => p.title.toLowerCase() === title.toLowerCase())) throw new Error(`There's already a ${title} profile.`);
        const item = await ctx.vault.create({ title, type: TYPE, hosts: ["agent-devices"], fields: { _profile_key: { generate: true } } });
        return { item, profile: title, text: `Added the ${title} profile. It has no sign-ins yet: sign in to sites in it with sign_in.` };
      },
    },
    _saved: {
      reads: false,
      description: "The service's: a browser profile was saved as a new blob, encrypted with a fresh salt, with the sites it was used on.",
      input: { type: "object", required: ["item", "blob", "salt"], properties: { item: { type: "string" }, blob: { type: "string" }, salt: { type: "string" }, sites: { type: "string" } } },
      run: async (ctx, { item, blob, salt, sites }) => {
        const all = await profiles(ctx);
        if (!all.some((p) => p.id === item)) throw new Error("That browser profile was deleted from the Vault.");
        ctx.vault.update(item, { fields: { _blob: { value: blob }, _salt: { value: salt }, _saved: { value: new Date(ctx.now()).toISOString() }, ...(sites ? { sites: { value: sites } } : {}) } });
        // The scope keeps each profile's newest save only, and none of a profile that was deleted.
        const kept = await ctx.user.query("select item, blob from saves");
        for (const s of kept) {
          if (s.item === item ? s.blob !== blob : !all.some((p) => p.id === s.item)) ctx.user.forget(s.blob);
          if (s.item !== item && !all.some((p) => p.id === s.item)) ctx.user.exec("delete from saves where item = ?", [s.item]);
        }
        // Its last save, as its item names it, even one this table doesn't hold.
        const was = all.find((p) => p.id === item).fields._blob?.value;
        if (was && was !== blob && !kept.some((s) => s.blob === was)) ctx.user.forget(was);
        ctx.user.exec("insert or replace into saves values (?, ?)", [item, blob], [blob]);
        return "Saved.";
      },
    },
    create: {
      reads: false,
      description: "Create a lasting device and start it, for the person, on the first of their devices in run priority (device-activity_order) that runs its kind and has room: Linux with a browser of its own (the default), Android or an iPhone. Takes a minute or two. Agents borrow instead.",
      input: {"properties": {"device": {"description": "its name: lowercase letters, digits and dashes, like work-browser", "type": "string"}, "kind": kindArg, "size": sizeArg}, "required": ["device"], "type": "object"},
      run: (ctx, input) => service(ctx, "create", input),
    },
    resize: {
      reads: false,
      description: "Change how much CPU and memory a lasting device may use, at once, without restarting it.",
      input: {"properties": {"device": deviceArg, "size": sizeArg}, "required": ["device", "size"], "type": "object"},
      run: (ctx, input) => service(ctx, "resize", input),
    },
    start: {
      reads: false,
      description: "Start a stopped (or paused) lasting device. It starts again after the device it's on restarts.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      run: (ctx, input) => service(ctx, "start", input),
    },
    stop: {
      reads: false,
      description: "Stop a lasting device. Its files and sign-ins stay; it stays stopped until agent-devices_start.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      run: (ctx, input) => service(ctx, "stop", input),
    },
    delete: {
      reads: false,
      confirm: true,
      description: "Delete a lasting device and everything in it, its sign-ins too, for good (agent-devices_backup first keeps a copy). Only when the person asked. A borrowed one goes with agent-devices_give_back.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      run: (ctx, input) => service(ctx, "delete", input),
    },
    backup: {
      reads: false,
      description: "Back up a device's home folder (its files and sign-ins, without caches) where any of your devices can fetch it, keeping the newest 5. agent-devices_restore makes a device from one.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      run: (ctx, input) => service(ctx, "backup", input),
    },
    backups: {
      reads: true,
      description: "Backups of agent devices, from all of your devices, newest first: each one's device, when (at, ISO 8601), its bytes, the device's size, and its blob (for agent-devices_restore).",
      input: {"properties": {"device": {"description": "only this agent device's backups, by its name", "type": "string"}, "here": {"description": "only the backups kept on the device you call from", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => service(ctx, "backups", input),
    },
    restore: {
      reads: false,
      description: "Make a new agent device, on the device you call from, from a backup: the newest of the device named, or the one with blob. It never replaces a device that's there: give as for another name.",
      input: {"properties": {"as": {"description": "the new device's name; default the backed-up one's", "type": "string"}, "blob": {"description": "a particular backup's blob, from agent-devices_backups", "type": "string"}, "device": {"description": "the backed-up device's name, from agent-devices_backups", "type": "string"}, "size": sizeArg}, "type": "object"},
      run: (ctx, input) => service(ctx, "restore", input),
    },
    move: {
      reads: false,
      description: "Move a lasting device to another of your devices: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
      input: {"properties": {"device": deviceArg, "to": {"description": "one of your devices, by name, from agent-devices_kinds", "type": "string"}}, "required": ["device", "to"], "type": "object"},
      run: (ctx, input) => service(ctx, "move", input),
    },
    kinds: {
      reads: true,
      description: "Which kinds of agent device each of your devices runs (linux, chrome, windows, android, iphone, as agent-devices_borrow's kind takes them; one it can't says why in unavailable): what input each kind takes, whether there's room for another, how many lasting ones nobody has borrowed, and the device's memory to spare (free_gb, in GB).",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "kinds", input),
    },
    open: {
      reads: false,
      description: "Open a web address in a device's browser: Chrome on Linux, Edge on Windows, the default browser on Android and an iPhone. To read and act on the page on Linux, use computer_browser_prepare and computer_get_browser_state.",
      input: { type: "object", required: ["device", "url"], properties: { device: deviceArg, url: { type: "string", description: "the address, like https://example.com" } } },
      run: (ctx, input) => service(ctx, "open", input),
    },
    install: {
      reads: false,
      description: "Install an app on a device, and say how it went: a file on the device it's on, a URL, or a package's name.",
      input: { type: "object", required: ["device", "app"], properties: { device: deviceArg, app: { type: "string", description: "a file's path on the device it's on, or a URL, to an .apk (Android), .msi, .msix or .exe (Windows), a simulator .app (iPhone) or a .deb (Linux); or a package's name: a winget id like Mozilla.Firefox, an apt package, or an Android package like com.whatsapp (it opens its Play Store page)" } } },
      run: (ctx, input) => service(ctx, "install", input),
    },
    run: {
      reads: false,
      description: "Run a command on an agent device and give its output. For a command on one of your own devices, shell_run.",
      input: { type: "object", required: ["device", "command"], properties: { device: deviceArg, command: { type: "string", description: "PowerShell on Windows, like Get-Process; a shell command on Android and Linux (as root), like ls /sdcard; on an iPhone, a program in the simulator, as simctl spawn takes it" } } },
      run: (ctx, input) => service(ctx, "run", input),
    },
    reach: {
      reads: false,
      description: "Let a Windows, Android or iPhone device reach a port on the device it's on, at the same port on its own 127.0.0.1, until it stops: for an app on it to reach a server you run there, like a throwaway network.",
      input: { type: "object", required: ["device", "port"], properties: { device: deviceArg, port: { type: "integer", description: "the port your server listens on, on the device the agent device is on, like 4791" } } },
      run: (ctx, input) => service(ctx, "reach", input),
    },
    adopt: {
      reads: false,
      description: "For the person: make something already on one of their devices a lasting device: a Windows VM's folder with its disk, an Android Virtual Device, or an iPhone simulator.",
      input: { type: "object", required: ["device", "kind", "on"], properties: {
        device: { type: "string", description: "its name: lowercase letters, digits and dashes, like windows" }, kind: kindArg, on: { type: "string", description: "one of your devices, by name, where it is" },
        dir: { type: "string", description: "windows: its folder" }, id: { type: "string", description: "android: its AVD's name; iphone: its simulator's UDID" },
        port: { type: "integer", description: "windows: its SSH port on the device it's on; android: its console port, like 5554" }, display: { type: "integer", description: "windows: its VNC display" },
        cpus: { type: "integer", description: "windows: CPUs it starts with" }, gb: { type: "integer", description: "windows: memory it starts with, in GB" },
      } },
      run: (ctx, { on, ...input }) => ctx.services["agent-devices"].adopt({ ...input, here: true }, { device: on }),
    },
    _list: {
      reads: true,
      description: "For the page: the agent devices on all of your devices, as the services last knew them: the device each is on, its kind, whether it's running, who has borrowed it and what for. None if it's the one the page has (have, its hash).",
      input: { type: "object", properties: { have: { type: "string" } } },
      // Only what the page shows, so it's the same until something it shows changes, and then the page is sent it.
      run: async (ctx, { have }) => {
        const r = await service(ctx, "list", { kept: true });
        const devices = (r.devices || []).map(({ name, on, kind, state, borrowed, spare, of }) => ({ name, on, kind, state, borrowed, spare, of }));
        const hash = fnv(JSON.stringify([devices, r.unreachable]));
        return have === hash ? { same: true, hash } : { devices, unreachable: r.unreachable, hash };
      },
    },
    _screen: {
      reads: true,
      description: "For the page: a small picture of a device, the one the device it's on last took, or none if it's the one the page has (have, its hash).",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => service(ctx, "screen", { ...input, kept: true }, input.on),
    },
    _view: {
      reads: true,
      description: "For the page: a link to watch and use a device.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => service(ctx, "view", input),
    },
    _chrome: {
      reads: true,
      description: "For Settings: each of your devices running Agent Devices, and whether agents may borrow Chrome run straight on it.",
      input: { type: "object", properties: {} },
      run: async (ctx) => {
        const r = await service(ctx, "chrome", {});
        return { devices: (r.devices || []).map(({ on, allowed, can }) => ({ on, allowed, can })).sort((a, b) => a.on.localeCompare(b.on)) };
      },
    },
    _set_chrome: {
      reads: false,
      description: "For Settings: lets agents borrow Chrome run straight on one of your devices, or stops them.",
      input: { type: "object", required: ["on", "allowed"], properties: { on: { type: "string" }, allowed: { type: "boolean" } } },
      run: (ctx, { on, allowed }) => service(ctx, "set_chrome", { on, allowed }),
    },
    _act: {
      reads: false,
      description: "For Computer Use (apps/computer), which agents act on agent devices with: runs one of the service's operations it acts with (list, view, screen, tools, refresh_tools, use, get_browser_state and the browser tools) with its input.",
      input: { type: "object", required: ["op"], properties: { op: { type: "string", enum: ACTS }, input: { type: "object" } } },
      run: (ctx, { op, input = {} }) => {
        if (!ACTS.includes(op)) throw new Error(`Computer Use doesn't act on agent devices with ${op}: only ${ACTS.join(", ")}`);
        return service(ctx, op, input);
      },
    },
    _running: {
      reads: true,
      description: "Which of your devices it runs on, for the page to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ devices: await running(ctx) }),
    },
  },

  screens: {
    main: {
      title: "Agent Devices",
      html: PAGE,
    },
    device: {
      title: "Device",
      html: DEVICE,
    },
    settings: {
      title: "Agent Devices",
      html: SETTINGS,
    },
  },
};
