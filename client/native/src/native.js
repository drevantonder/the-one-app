// The app, as the web client's page (cloudflare/web/web.js) finds it: `window.__oneNative`, in the window's own page
// alone. Screens' frames are sandboxed in it with no origin of their own: this doesn't run there, and the app's
// commands don't answer them.
(() => {
  if (window !== window.top) return;
  // Signed out (its refresh token refused) is said as "Sign in again", which the page takes as signed out.
  const fail = (err) => {
    const text = typeof err === "string" ? err : err?.message || String(err);
    return Object.assign(new Error(text), text.startsWith("Sign in again") ? { signedOut: true } : {});
  };
  // Each command's refusal, as an Error the page can say.
  const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args).catch((e) => Promise.reject(fail(e)));
  const base64 = (bytes) => {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
  };

  window.__oneNative = {
    /// `{ name, cli, platform, device, client, address, signedIn, user, opened }`: `opened`, a link it was opened with.
    about: () => invoke("native"),
    /// What the person typed, an address or an invite link → the address.
    address: (typed) => invoke("address", { typed }),
    signIn: () => invoke("sign_in"),
    /// → `{ in: true }`, or `{ code, invited }` to show while `wait` waits.
    join: () => invoke("join"),
    wait: () => invoke("wait"),
    removed: () => invoke("removed"),
    /// The window in front (at a `<cli>://` link, if given), from the menu bar panel; Open at Login switched; Quit.
    main: (link) => invoke("main", { link: link ?? null }),
    login: (on) => invoke("login", { on }),
    quit: () => invoke("quit"),
    /// The menu bar panel this tall (CSS pixels), its top where it is.
    fit: (height) => invoke("fit", { height }),
    /// Something changed on the person's devices: the app's own widgets are drawn again.
    changed: () => invoke("changed").catch(() => {}),
    signOut: () => invoke("sign_out"),
    notify: (ask) => invoke("notify", { ask }),
    copy: (text) => invoke("copy", { text }),
    open: (url) => invoke("open", { url }),
    /// A link the app was opened with while the page runs (`<cli>://open?…`), to `onOpen`'s function.
    onOpen: null,
    opened(url) {
      if (window.__oneNative.onOpen) window.__oneNative.onOpen(url);
    },
    save: async (name, blob, preview) => invoke("save", { name, data: base64(new Uint8Array(await blob.arrayBuffer())), preview }),

    /// A request to the person's nodes, as net.js's `fetch` → a Response whose body comes as the node sends it.
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
