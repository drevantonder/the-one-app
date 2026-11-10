// The app, as the web client's page finds it (cloudflare/web/app.js): `window.__oneNative`, in the window's own page
// alone. Screens' frames are sandboxed in it with no origin of their own: this doesn't run there, and the app's
// commands don't answer them.
(() => {
  if (window !== window.top) return;
  // A command's refusal: its text, or lib.rs's `Failure`, which says if this device is signed out (its sign-in refused).
  const fail = (err) => Object.assign(new Error(typeof err === "string" ? err : err?.message || String(err)), err?.signedOut ? { signedOut: true } : {});
  // On Android, Tauri's script and `ipc` are in every frame, a screen's too: the app's commands carry a token only this frame
  // is given, by a web message listener that answers the page's own frame alone (MainActivity.kt). Without the listener
  // (an old web view), none of it works.
  const android = /Android/.test(navigator.userAgent);
  const bridge = window.oneBridge;
  const token = bridge
    ? new Promise((resolve) => {
        bridge.onmessage = (e) => resolve(e.data);
        bridge.postMessage("token");
      })
    : null;
  // Each command's refusal, as an Error the page can say.
  const invoke = async (cmd, args) => {
    if (android && !token) throw fail("Update Android System WebView in Google Play, then open this again.");
    return window.__TAURI_INTERNALS__.invoke(cmd, args, token ? { headers: { "x-one": await token } } : undefined).catch((e) => Promise.reject(fail(e)));
  };
  const base64 = (bytes) => {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
  };

  const unbase64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

  // What goes wrong in the page, for the device's log (`one logs <device>`): an error nothing caught, a promise nothing
  // handled, a file that didn't load. Not what the page prints, which may hold anyone's data. One line, once in a row.
  let last = "";
  const log = (text) => {
    if (text === last) return;
    last = text;
    invoke("log", { text }).catch(() => {});
  };
  const where = (url) => String(url || "").split(/[?#]/)[0];
  addEventListener(
    "error",
    (e) =>
      e.target !== window
        ? log(`couldn't load ${where(e.target.src || e.target.href)}`)
        : log(`${e.message} at ${where(e.filename)}:${e.lineno}:${e.colno}${e.error?.stack ? `\n${e.error.stack}` : ""}`),
    true,
  );
  addEventListener("unhandledrejection", (e) => log(`unhandled rejection: ${e.reason?.stack || e.reason?.message || e.reason}`));

  // Tauri's `data-tauri-drag-region` always zooms a window double-clicked there; a Mac's user may have set title bars to
  // minimise or do nothing (System Settings, AppleActionOnDoubleClick), which only the app can read.
  addEventListener(
    "mouseup",
    (e) => {
      if (e.detail !== 2 || !e.target.hasAttribute?.("data-tauri-drag-region")) return;
      e.stopImmediatePropagation();
      invoke("title_bar_double_clicked").catch(() => {});
    },
    true,
  );

  window.__oneNative = {
    /// A line for this device's log (`one logs <device>`), from the page's own code.
    log,
    /// Who and where this is, and what this platform can do: lib.rs's `native` says each.
    about: () => invoke("native"),
    /// What the user typed, an address or an invite link → the address.
    address: (typed) => invoke("address", { typed }),
    signIn: () => invoke("sign_in"),
    demo: () => invoke("demo"),
    /// → `{ in: true }`, or `{ code, invited }` to show while `wait` waits.
    join: () => invoke("join"),
    wait: () => invoke("wait"),
    removed: () => invoke("removed"),
    /// This device's key, as its own client in this page asks it (`{ key, seal }`, hex), never the key itself: it signs and
    /// agrees here, in the app. What it keeps, by name; an access token, new as needed; and it's let in (`joined`).
    keys: () => invoke("device_keys"),
    sign: async (bytes) => unbase64(await invoke("device_sign", { bytes: base64(bytes) })),
    shared: async (share) => unbase64(await invoke("device_shared", { share: base64(share) })),
    keepGet: async (name) => {
      const v = await invoke("keep_get", { name });
      return v == null ? null : unbase64(v);
    },
    keepPut: (name, bytes) => invoke("keep_put", { name, bytes: base64(bytes) }),
    keepDelete: (name) => invoke("keep_delete", { name }),
    token: () => invoke("access_token"),
    waiting: (code) => invoke("waiting", { code }).catch(() => {}),
    joined: () => invoke("joined").catch(() => {}),
    /// The window in front (at a `<cli>://` link, if given), from the menu bar panel; Open at Login switched; Quit.
    main: (link) => invoke("main", { link: link ?? null }),
    login: (on) => invoke("login", { on }),
    /// A Mac's own node joins a network with a pairing code (`about().unpaired`) → its address.
    pair: (code) => invoke("pair", { code }),
    quit: () => invoke("quit"),
    /// The menu bar panel this tall (CSS pixels), its top where it is.
    fit: (height) => invoke("fit", { height }),
    /// Something changed on the user's devices: the app's own widgets are drawn again.
    changed: () => invoke("changed").catch(() => {}),
    signOut: () => invoke("sign_out"),
    /// What a call that only reads answered last on this device (`user` is the app's own) → `{ answer, at }`, or null.
    kept: (user, app, tool, input) => invoke("kept", { app, tool, input }),
    /// Renews the user's app schedules of `app`, which they just used.
    renew: (app) => invoke("renew", { app }),
    /// A phone's haptic: "selection", "medium", "success" or "error".
    haptic: (style) => invoke("haptic", { style }).catch(() => {}),
    /// The document camera, where there's one (`about().scan`) → its pages as files.
    scan: () => invoke("scan"),
    /// A screen's menu, shown with what a touch and hold lifted if it's for that (`about().lifts`) → `{ index }`; else
    /// `{ lifted: false }`, and the page shows its own.
    menu: (items) => invoke("menu", { items }),
    /// Light, dark or automatic, kept by the app too, so it starts in it.
    appearance: (value) => invoke("appearance", { value }).catch(() => {}),
    /// One of the app menu's commands ("sidebar", "join"), to `onCommand`'s function.
    onCommand: null,
    command(name) {
      if (window.__oneNative.onCommand) window.__oneNative.onCommand(name);
    },
    /// What waits for the user, counted on the app's icon (the taskbar's, the dock's): 0 for none.
    badge: (count) => invoke("badge", { count }).catch(() => {}),
    notify: (ask) => invoke("notify", { ask }),
    copy: (text) => invoke("copy", { text }),
    open: (url) => invoke("open", { url }),
    /// The hardware Back (an Android phone's) → whether the page took it (closed what's over it, or went back); if not, the
    /// app leaves to the home screen. `onBack` is the page's.
    onBack: null,
    hardwareBack: () => (window.__oneNative.onBack ? !!window.__oneNative.onBack() : false),
    /// Settings' text size changed while the app runs (a phone's): how many times usual, to `onScale`'s function.
    onScale: null,
    scale(value) {
      if (window.__oneNative.onScale) window.__oneNative.onScale(value);
    },
    /// A link the app was opened with while the page runs (`<cli>://open?…`), to `onOpen`'s function.
    onOpen: null,
    opened(url) {
      if (window.__oneNative.onOpen) window.__oneNative.onOpen(url);
    },
    save: async (name, blob, preview) => invoke("save", { name, data: base64(new Uint8Array(await blob.arrayBuffer())), preview }),

    /// A request to the user's nodes, as net.js's `fetch` → a Response whose body comes as the node sends it.
    async fetch(method, path, headers, body, signal) {
      const r = await invoke("fetch", { method, path, headers: Object.entries(headers || {}), body: body ?? null });
      const stop = () => invoke("cancel", { id: r.id }).catch(() => {});
      const stream = new ReadableStream({
        async pull(c) {
          if (signal?.aborted) return void (stop(), c.error(signal.reason));
          const part = new Uint8Array(await invoke("read", { id: r.id }));
          if (part.length) c.enqueue(part);
          else c.close();
        },
        cancel: stop,
      });
      signal?.addEventListener("abort", stop, { once: true });
      const empty = r.status === 204 || r.status === 304;
      if (empty) stop();
      return Object.assign(new Response(empty ? null : stream, { status: r.status, headers: r.headers }), { trace: r.trace });
    },
  };
})();
