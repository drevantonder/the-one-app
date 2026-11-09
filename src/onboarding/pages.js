// Auth's screens: signing in, to the network or to a client, joining with an invite link, and a user's passkeys.
// Served from auth at the network's address, so passkeys belong to its name. Each is a page pages.tsx draws (@one/ui's
// Welcome, drawn to HTML when @one/ui is built, `pages` and `parts` of its dist/auth.json, with its stylesheet, `css`),
// filled in with the network's look, and a little script. Not React, as the screens are: the Worker that serves a
// browser's code before it is signed in stays small, a few KB of CSS and this script, and nothing here has to be drawn
// again after it's drawn. Inline, as the page's policy allows nothing else.

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// WebAuthn's binary fields, to and from the base64url its JSON uses; and asking a passkey for its PRF output, where it
// has PRF, under the salt sha256("one/passkey-device"), from which the web client derives the passkey's device keys
// (keys::passkey_keys). The output is kept only in this tab's sessionStorage (`one.prf`, with the passkey's credential id
// as `one.prf.credential`), which the web client at the same address reads: it's never sent anywhere.
const webauthn = `
  const b64u = {
    decode: (s) => Uint8Array.from(atob(s.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)),
    encode: (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, ""),
  };
  const PRF_SALT = crypto.subtle.digest("SHA-256", new TextEncoder().encode("one/passkey-device")).then((h) => new Uint8Array(h));
  // The page's parts (pages.tsx's): Welcome's, and those drawn for this script to copy in.
  const slot = (name) => document.querySelector("[data-slot=welcome-" + name + "]");
  const part = (name) => document.querySelector("template[data-part=" + name + "]").content.firstElementChild.cloneNode(true);
  // A problem: what happened, and what to do after the first sentence.
  const say = (text) => {
    const [title, ...rest] = String(text || "").split(/(?<=[.!?])\\s+/);
    slot("problem").replaceChildren();
    if (!text) return;
    const problem = part("problem");
    problem.querySelector("[data-slot=alert-title]").textContent = title;
    if (rest.length) problem.querySelector("[data-slot=alert-description] p").textContent = rest.join(" ");
    else problem.querySelector("[data-slot=alert-description]").remove();
    slot("problem").append(problem);
  };
  const quietly = (err) => say(err.name === "NotAllowedError" || err.name === "AbortError" ? "" : err.message);
  async function api(path, body) {
    const resp = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error || out.message || "That didn't work."), { status: resp.status });
    return out;
  }
  function credentialJSON(c) {
    const r = c.response;
    const response = { clientDataJSON: b64u.encode(r.clientDataJSON) };
    if (r.attestationObject) Object.assign(response, { attestationObject: b64u.encode(r.attestationObject), transports: r.getTransports?.() || [] });
    if (r.authenticatorData) Object.assign(response, { authenticatorData: b64u.encode(r.authenticatorData), signature: b64u.encode(r.signature), userHandle: r.userHandle && b64u.encode(r.userHandle) });
    return { id: c.id, rawId: b64u.encode(c.rawId), type: c.type, response, clientExtensionResults: {}, authenticatorAttachment: c.authenticatorAttachment };
  }
  function keepPrf(c) {
    const first = c.getClientExtensionResults().prf?.results?.first;
    if (!first) return;
    sessionStorage.setItem("one.prf", b64u.encode(first));
    sessionStorage.setItem("one.prf.credential", c.id);
  }
  function busy(b, label) {
    const was = [...b.childNodes];
    b.disabled = true;
    b.replaceChildren(part("spinner"), label);
    const undo = () => { b.replaceChildren(...was); b.disabled = false; };
    // Come back to with Back, after it went on, it's ready again.
    addEventListener("pageshow", (e) => e.persisted && undo(), { once: true });
    return undo;
  }
  /// A passkey prompt's options are asked for before the tap (\`ready\`), so the prompt comes the moment it's tapped. Each
  /// is used once, and only while it's fresh: auth keeps one challenge, for five minutes.
  // A sign-in for a device (\`join\`, in its client's authorization) has the passkey sign a challenge naming it.
  const JOIN = new URLSearchParams(location.search).get("join");
  const ASK = { in: "/api/auth/passkey/generate-authenticate-options" + (/^[0-9a-f]{128}$/.test(JOIN || "") ? "?join=" + JOIN : ""), new: "/api/auth/passkey/generate-register-options" };
  let early = null;
  function ready(path) {
    const asked = api(path);
    asked.catch(() => {});
    early = { path, at: Date.now(), asked };
  }
  function options(path) {
    const e = early;
    early = null;
    return e && e.path === path && Date.now() - e.at < 240000 ? e.asked.catch(() => api(path)) : api(path);
  }
  const register = (context) => ASK.new + (context ? "?context=" + encodeURIComponent(context) : "");
  async function newPasskey(context) {
    const o = await options(register(context));
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), user: { ...o.user, id: b64u.decode(o.user.id) },
      excludeCredentials: (o.excludeCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })), extensions: { prf: { eval: { first: await PRF_SALT } } } };
    const c = await navigator.credentials.create({ publicKey });
    const made = await api("/api/auth/passkey/verify-registration", { response: credentialJSON(c), ...(context ? { createSession: true } : {}) });
    keepPrf(c);
    return { credential: c.id, id: made.id };
  }
  async function withPasskey() {
    const o = await options(ASK.in);
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), allowCredentials: (o.allowCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })),
      extensions: { prf: { eval: { first: await PRF_SALT } } } };
    const c = await navigator.credentials.get({ publicKey });
    await api("/api/auth/passkey/verify-authentication", { response: credentialJSON(c) });
    keepPrf(c);
    return { credential: c.id };
  }
`;

/// A client's name is only what it calls itself, so a client other than the web client is told what it will do and where it goes.
const acts = `
  function acts(about) {
    slot("text").textContent = about.client + " will act as you, with everything you can use in " + network + ", until you sign it out.";
    slot("callout").textContent = about.goes === "an app on this device" ? "It's an app on this device." : about.goes === "where you got your code" ? "It's where you got your code. Only go on if you got it yourself, on your own device." : "It goes to " + about.goes + ". Only go on if that's who you meant to sign in to.";
  }
`;

const json = (value) => JSON.stringify(value).replaceAll("<", "\\u003c");

/// Auth's pages, from what @one/ui's build drew (`drawn`).
export function pages(drawn) {
  const { css, pages, parts } = drawn;

  /// The page `name` draws, filled in with `look` (the network's `name`, `yours` and `logo`, the domain of `google`, a
  /// throwaway's `user`), and `script`, with the parts it copies in (pages.tsx's `parts`, and the pages named in `more`).
  /// Its title is its heading.
  const page = (name, look, script = "", more = []) => {
    const fill = (html) => html.replace(/\{\{(\w+)\}\}/g, (_, key) => escape(look[key] ?? ""));
    const body = fill(pages[name]);
    const title = /data-slot="welcome-title"[^>]*>([^<]*)</.exec(body)?.[1] ?? "";
    const copied = { ...parts, ...Object.fromEntries(more.map((m) => [m, pages[m]])) };
    const templates = script ? Object.entries(copied).map(([n, html]) => `<template data-part="${n}">${fill(html)}</template>`).join("") : "";
    return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" media="(prefers-color-scheme: light)" content="#ffffff">
<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#141414"><title>${title}</title><link rel="icon" href="${escape(look.logo)}"><style>${css}</style></head>
<body>${body}${templates}${script ? `<script>${webauthn}${script}</script>` : ""}</body></html>`;
  };

  /// Signing in, to the network or, sent here by a client's authorization (\`oauth_query\`), to that client: the client
  /// is the user, with everything they can use, so it's asked each time. With Google (`google`, the network's
  /// domain), it's signing in with Google, or with a passkey for an admin.
  function signInPage(look) {
    if (look.google) return googlePage(look);
    const script = `${acts}
      const oauth_query = location.search.slice(1);
      const client_id = new URLSearchParams(oauth_query).get("client_id");
      const network = ${json(look.name)};
      if (client_id) (async () => {
        try {
          const about = await api("/sign-in/about?" + oauth_query);
          const title = "Sign in to " + about.client + (about.user ? " as " + about.user : "");
          document.title = title;
          slot("title").textContent = title;
          if (!about.web) acts(about);
        } catch (err) { say(err.message); }
      })();
      ready(ASK.in);
      slot("action").onclick = async (e) => {
        say();
        const back = busy(e.currentTarget, "Signing In…");
        try {
          await withPasskey();
          location.href = client_id ? (await api("/sign-in/accept", { oauth_query, passkey: true })).url : "/";
        } catch (err) {
          back();
          ready(ASK.in);
          quietly(err);
        }
      };`;
    return page("sign-in", look, script);
  }

  function googlePage(look) {
    const script = `${acts}
      // Google's answer comes back here, with an error added to the query if it said no.
      const raw = location.search.slice(1);
      const failed = /(^|&)error=/.test(raw);
      const oauth_query = raw.replace(/(^|&)error(_description)?=[^&]*/g, "").replace(/^&/, "");
      const client_id = new URLSearchParams(oauth_query).get("client_id");
      const network = ${json(look.name)};
      const domain = ${json(look.google)};
      const back = client_id ? "/sign-in?" + oauth_query : "/";
      if (failed) say("That didn't sign you in. Use your Google account at " + domain + ".");
      const button = slot("action");
      // Back from Google, which this tab went to a moment ago: it says so while it goes on, not offering to start again.
      const returning = !failed && Date.now() - Number(sessionStorage.getItem("google") || 0) < 600000;
      sessionStorage.removeItem("google");
      let waiting = returning ? busy(button, "Signing You In…") : null;
      // Google said no because they're an admin: the way in is the passkey, which is why it's the other button.
      const admins = (about) => about.admin && !about.fresh && say("You're an admin, so you sign in with your passkey, not Google.");
      if (!client_id && failed) api("/sign-in/about?").then(admins, () => {});
      if (client_id) (async () => {
        try {
          const about = await api("/sign-in/about?" + oauth_query);
          const title = "Sign in to " + about.client;
          document.title = title;
          slot("title").textContent = title;
          if (!about.web) acts(about);
          admins(about);
          // Back from the Google sign-in started here for this client, just now: on to it.
          if (about.fresh && !failed) {
            waiting ||= busy(button, "Signing You In…");
            return location.replace((await api("/sign-in/accept", { oauth_query })).url);
          }
        } catch (err) { say(err.message); }
        waiting?.();
      })();
      else waiting?.();
      button.onclick = async () => {
        say();
        const undo = busy(button, "Opening Google…");
        try {
          await api("/sign-in/google", { oauth_query });
          // Back to this page, with Google's no, also for the web client, whose sign-in goes on to its own page.
          const out = await api("/api/auth/sign-in/social", { provider: "google", callbackURL: back, errorCallbackURL: client_id ? back : "/sign-in" });
          sessionStorage.setItem("google", String(Date.now()));
          location.href = out.url;
        } catch (err) {
          undo();
          say(err.message);
        }
      };
      slot("other").onclick = async (e) => {
        say();
        const back = busy(e.currentTarget, "Signing In…");
        try {
          await withPasskey();
          location.href = client_id ? (await api("/sign-in/accept", { oauth_query, passkey: true })).url : "/";
        } catch (err) {
          back();
          quietly(err);
        }
      };`;
    return page("sign-in-google", look, script);
  }

  /// Signing in on a throwaway network (`one-dev throwaway`), with no passkey, straight on, so a client being tested signs in with nobody
  /// there: as whoever this browser is signed in as here (its first user, or someone who joined in it), or, with
  /// `?as=<name>` and no client, as someone who joined, for a device's browser before its app signs in. The button is for
  /// a browser that won't go on to an app by itself.
  function throwawayPage(look) {
    const script = `
      const oauth_query = location.search.slice(1);
      const params = new URLSearchParams(oauth_query);
      const button = slot("action");
      const go = async () => {
        say();
        const undo = busy(button, "Signing In…");
        try {
          const { user } = await api("/api/auth/throwaway/sign-in", params.get("as") ? { user: params.get("as") } : { join: params.get("join") || undefined });
          if (params.get("as")) {
            undo();
            button.textContent = "Sign In as " + user;
            return void (slot("text").textContent = "This browser is signed in here as " + user + ".");
          }
          location.href = params.get("client_id") ? (await api("/sign-in/accept", { oauth_query })).url : "/";
        } catch (err) {
          undo();
          say(err.message);
        }
      };
      button.onclick = go;
      go();`;
    return page("sign-in-throwaway", look, script);
  }

  /// Joining with an invite link, its secret in the fragment, which goes only to auth's sign-up: making a first passkey,
  /// or signing in with Google (`google`) as the email it's for, or on a throwaway network (`throwaway`), with nothing to make. With
  /// `?code=`, the CLI's (`sign-in --invite`), it goes on to sign that in, as the user who just joined.
  function invitePage(look) {
    const { throwaway, google } = look;
    const script = `
      const secret = location.hash.slice(1);
      const throwaway = ${throwaway ? "true" : "false"}, google = ${google ? "true" : "false"};
      const code = new URLSearchParams(location.search).get("code");
      const button = slot("action");
      button.disabled = true;
      // Google's answer comes back here with an error if it didn't sign them up.
      if (/(^|&)error=/.test(location.search.slice(1))) say("That Google account isn't the one this invite is for.");
      // Whoever was signed in here before, the invite is for its user: signed out as the page opens, beside asking
      // whose invite it is, and then the passkey's options, so the prompt comes the moment Make a Passkey is tapped.
      const out = api("/api/auth/sign-out", {}).catch(() => {});
      if (!throwaway && !google) out.then(() => ready(register(secret)));
      (async () => {
        try {
          const about = await api("/invite/about", { secret });
          slot("title").textContent = "Welcome, " + about.user;
          if (google && about.email) slot("text").textContent = "With your Google account, " + about.email + ".";
          button.disabled = false;
        } catch (err) { say(err.message); }
      })();
      button.onclick = async () => {
        say();
        const undo = busy(button, throwaway ? "Joining…" : google ? "Opening Google…" : "Making a Passkey…");
        try {
          // Kept in this tab for the web client, whose request to let in this user's first device proves it.
          sessionStorage.setItem("one.invite", secret);
          await out;
          if (google) {
            await api("/invite/google", { secret });
            const back = "/invite" + location.search + "#" + secret;
            location.href = (await api("/api/auth/sign-in/social", { provider: "google", callbackURL: code ? "/sign-in/code?c=" + encodeURIComponent(code) : "/", errorCallbackURL: back })).url;
            return;
          }
          if (throwaway) await api("/api/auth/throwaway/join", { secret });
          else await newPasskey(secret);
          if (code) return void (location.href = "/sign-in/code?c=" + encodeURIComponent(code));
          history.replaceState(null, "", "/invite");
          document.querySelector("[data-slot=welcome]").replaceWith(part(throwaway ? "joined-throwaway" : "joined"));
        } catch (err) {
          undo();
          if (!throwaway && !google) ready(register(secret));
          quietly(err);
        }
      };`;
    return page(throwaway ? "invite-throwaway" : google ? "invite-google" : "invite", look, script, google ? [] : [throwaway ? "joined-throwaway" : "joined"]);
  }

  /// A user's passkeys: each one, with Remove while there's another, and Add a Passkey; or, signed out, signing in.
  function passkeysPage(look) {
    const script = `
      if (window.top !== window) document.documentElement.classList.add("framed");
      const when = (d) => new Date(d).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
      const button = slot("action"), wait = document.querySelector("[data-slot=welcome] [data-slot=spinner]"), rows = document.querySelector("[data-slot=passkeys]");
      const steps = slot("note").textContent;
      let list = [], signedOut = false;
      const shown = (signedIn) => {
        wait.remove();
        button.hidden = false;
        slot("note").hidden = !signedIn;
        slot("callout").hidden = !signedIn || list.length > 1;
        button.textContent = signedIn ? "Add a Passkey" : "Sign In with a Passkey";
      };
      for (const s of ["action", "note", "callout"]) slot(s).hidden = true;
      async function show() {
        try { list = await api("/api/auth/passkey/list-user-passkeys"); }
        catch (err) {
          wait.remove();
          if (err.status !== 401) return say(err.message);
          signedOut = true;
          rows.replaceChildren();
          return shown(false);
        }
        signedOut = false;
        rows.replaceChildren(...list.map((p) => {
          const row = part("passkey"), [name, added] = row.querySelectorAll("span > span"), remove = row.querySelector("button");
          name.textContent = p.name || "Passkey";
          added.textContent = "Added " + when(p.createdAt);
          if (list.length > 1) remove.dataset.id = p.id;
          else remove.remove();
          return row;
        }));
        shown(true);
      }
      show();
      ready(ASK.in);
      button.onclick = async () => {
        say();
        if (signedOut) {
          const undo = busy(button, "Signing In…");
          try { await withPasskey(); await show(); } catch (err) { quietly(err); }
          ready(ASK.in);
          return undo();
        }
        const undo = busy(button, list.length ? "Signing In…" : "Adding…");
        try {
          if (list.length) {
            await withPasskey();
            button.lastChild.textContent = "Adding…";
            slot("note").textContent = "Now make the new passkey.";
          }
          await newPasskey();
          await show();
        } catch (err) { quietly(err); }
        undo();
        slot("note").textContent = steps;
        ready(ASK.in);
      };
      rows.onclick = async (e) => {
        const id = e.target.dataset?.id;
        if (!id || !confirm("Remove this passkey? It won't sign you in again.")) return;
        busy(e.target, "Removing…");
        try { await api("/api/auth/passkey/delete-passkey", { id }); } catch (err) { say(err.message); }
        await show();
      };`;
    return page("passkeys", look, script);
  }

  /// Signing in with a code another device shows (the CLI on one with no screen): typing it here goes on to signing in with a
  /// passkey (or Google); `done` says how that went, once it's back.
  function codePage(look) {
    const { google, error = "", done } = look;
    if (done !== undefined) return page(done ? "code-signed-in" : "code-not-signed-in", look);
    const script = `
      say(${json(error)});
      const field = slot("field"), button = slot("action");
      field.focus();
      field.oninput = () => (button.disabled = !field.value.trim());
      const go = (e) => {
        e.preventDefault();
        if (!field.value.trim()) return;
        busy(button, "Continuing…");
        location.href = "/sign-in/code?c=" + encodeURIComponent(field.value);
      };
      field.form.onsubmit = go;
      button.onclick = go;`;
    return page(google ? "code-google" : "code", look, script);
  }

  return { page, signInPage, throwawayPage, invitePage, passkeysPage, codePage };
}
