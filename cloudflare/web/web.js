// The web client: the window of every client, in a browser and in the app (platform.js says which, once). It asks the
// user's nodes as their device (request.js), and draws the window (window.js) around screens it frames (screens.js),
// at the place the URL names (routes.js), from what the nodes say (store.js). App screens are untrusted: each runs in a
// sandboxed frame, and this page answers its bridge, making each call itself; the client's own screens (Settings) are
// answered as the user (settings.js). The modules import only downward, as each is named by its content with its
// imports' names in it (src/web.rs): view, platform, request, store, routes, offline, settings, moves, screens, window, web.
//
// The bridge (src/screen/), loaded after this, gives `one`; this page answers it for its own use (`__one.host`).
import { late, offline } from "./offline.js";
import { platform } from "./platform.js";
import { api, retryWait, sleep } from "./request.js";
import { go, href, own, route } from "./routes.js";
import { ask, iconPaths, openRequest } from "./screens.js";
import { appearance, schemed } from "./settings.js";
import { config, forgetScreens, joined, lastAt, lastWindow, me, network, refreshApps, refreshState, setJoining, signOut, whySignedOut } from "./store.js";
import { h, logo, message, show, ui } from "./view.js";
import { hardwareBack, listen, render, showBack, showJoin, showSignedOut, showUnreachable, watchWaiting, windowShowing } from "./window.js";

window.__one = { host: async (m) => (m.type === "copy" ? platform.copy(m.text) : m.type === "icons" ? iconPaths(m.names) : null) };

const signedIn = () => message("You’re Signed In", `Go back to ${config().name}.`);

async function start() {
  schemed(document, appearance());
  document.documentElement.style.visibility = "";
  document.documentElement.style.background = "";
  const problem = await platform.start({
    look: appearance(),
    route,
    go: (to) => joined.then(() => go(to)),
    request: (scope, id) => joined.then(() => openRequest(scope, id)),
    signedOut: showSignedOut,
    back: hardwareBack,
  });
  if (problem) return showSignedOut(problem);
  // A new device's QR: Approve, with its code.
  if (location.pathname === "/approve") history.replaceState(null, "", href(own("devices", { approve: location.hash.slice(1) })));
  // `/back?state=<app>/<screen>[/<the app's own>]&…`: where a sign-in at another site sends the user back (an app's
  // `ctx.address`).
  // It opens that app's screen with the whole address as `params.back`, which finishes it. A browser that isn't
  // signed in here passes it on to the app on this phone or computer, which opens the same screen from `<cli>://open`.
  if (location.pathname === "/back") {
    const [, app, screen = "main"] = /^([a-z0-9][a-z0-9-]*)(?:\/([a-z0-9_-]+)(?:\/[A-Za-z0-9_-]*)?)?$/.exec(new URLSearchParams(location.search).get("state") || "") || [];
    const params = JSON.stringify({ back: location.href });
    if (!app) history.replaceState(null, "", "/");
    else if (platform.signedIn()) history.replaceState(null, "", href({ kind: "screen", app, screen, params }));
    else return showBack(app, screen, params);
  }
  // `/signed-in#<code>`: where a native app sends the browser the user signed in with, once it has asked to join. Not signed
  // in here, or the app's request not one this device can answer, it's only told to go back.
  const handedOver = location.pathname === "/signed-in";
  if (handedOver && !platform.signedIn()) return show(signedIn());
  if (!platform.signedIn()) {
    // However the sign-in ended (it ran out, another tab signed out), nothing of who it was stays for the next person.
    forgetScreens().catch(() => {});
    // Opened from an invite link (`/invite`): on to signing in, which is joining.
    if (sessionStorage.getItem("one.invite")) return platform.signIn().catch((err) => showSignedOut(err.message));
    return showSignedOut(whySignedOut());
  }
  // `/vault?id=…&scope=…`: an agent's link to a request for an item, answered over You.
  let request = null;
  if (location.pathname === "/vault") {
    const q = new URLSearchParams(location.search);
    request = { scope: q.get("scope"), id: q.get("id") };
    history.replaceState(null, "", "/");
  }
  // Everything the window opens on, asked for at once: its screen, beside the node's state and apps; meanwhile the
  // window as it was last. Its screens' asks wait until this is its user's device (`joined`).
  ask(route());
  if (lastWindow()) render();
  else show(h(ui.Starting, { logo }));
  setJoining(true);
  for (let misses = 1; ; misses++) {
    try {
      // A device from the start, as a node may need the keys it hands (the Cloudflare node, even for the state).
      let waited = false;
      await platform.join((code, invited) => ((waited = true), showJoin(code, invited)));
      if (waited) show(h(ui.Starting, { logo }));
      break;
    } catch (err) {
      if (err.unreachable) {
        if (!windowShowing()) showUnreachable();
        await sleep(retryWait(misses));
        continue;
      }
      if (!err.signedOut) return void show(message("Can’t Let This Device In", err.message));
      if (err.removed) return;
      return signOut(err.message);
    }
  }
  if (handedOver) {
    const code = location.hash.slice(1);
    const asked = await api("/v1/devices/requests", { method: "POST", body: {} }).catch(() => []);
    // A request names its device by as much of its code as its sign-in leaves to check (`JOIN_CODE_SHORT`), the app by all of it.
    if (!asked.some((r) => r.code && code.startsWith(r.code.replaceAll(" ", "")))) return show(signedIn());
    history.replaceState(null, "", href(own("devices", { approve: code })));
  }
  // A device that takes the request and never answers (asleep, stuck) is one that can't be reached, while it's still asked.
  const away = setTimeout(() => windowShowing() || showUnreachable(), 15000);
  // What it showed last, with the devices still not answering: said so, as a failed ask is.
  const slow = setTimeout(() => windowShowing() && offline.add("state", () => refreshState(), lastAt()), late());
  for (let misses = 1; ; misses++) {
    try {
      await refreshState();
      offline.done("state");
      break;
    } catch (err) {
      if (err.signedOut) return clearTimeout(away), clearTimeout(slow), err.removed ? undefined : void signOut(err.message);
      // What it showed last stays, as a screen whose node is restarting does, saying so.
      if (!windowShowing()) (console.warn("starting:", err.message), showUnreachable());
      else offline.add("state", () => refreshState(), lastAt());
      await sleep(retryWait(misses));
    }
  }
  clearTimeout(away);
  clearTimeout(slow);
  if (network() && me()) {
    await platform.settle();
    await refreshState().catch(() => {});
  }
  setJoining(false);
  joined.done();
  await refreshApps().catch((err) => windowShowing() || (console.warn("starting:", err.message), showUnreachable()));
  if (request?.id && request.scope) openRequest(request.scope, request.id);
  listen();
  if (platform.panel) return;
  watchWaiting();
  platform.notify(false).catch((err) => console.warn("notifications:", err));
}

document.addEventListener("DOMContentLoaded", start);
