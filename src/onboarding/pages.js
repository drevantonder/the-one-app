// Auth's screens: signing in, to the network or to a client, joining with an invite link, and a user's passkeys.
// Served from auth at the network's address, so passkeys belong to its name. Each is a page pages.tsx draws (@one/ui's
// Welcome, drawn to HTML when @one/ui is built, `pages` and `parts` of its dist/auth.json, with its stylesheet, `css`),
// filled in with the network's look, and a little script. Not React, as the screens are: the Worker that serves a
// browser's code before it is signed in stays small, a few KB of CSS and this script, and nothing here has to be drawn
// again after it's drawn. Inline, as the page's policy allows nothing else.

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// WebAuthn's binary fields, to and from the base64url its JSON uses.
const webauthn = `
  const b64u = {
    decode: (s) => Uint8Array.from(atob(s.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)),
    encode: (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, ""),
  };
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
    appear(problem);
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
  const FOR_DEVICE = /^[0-9a-f]{128}$/.test(JOIN || "") ? "?join=" + JOIN : "";
  const ASK = { in: "/api/auth/passkey/generate-authenticate-options" + FOR_DEVICE, new: "/api/auth/passkey/generate-register-options" };
  /// Gives the client this page's authorization names a code, for whoever just signed in: → where it goes on to.
  const consent = async (oauth_query) => (await api("/api/auth/oauth2/consent", { accept: true, oauth_query })).url;
  /// A browser joining with an invite link is signing in to the web client as it does: that sign-in's page goes to the
  /// invite, so joining is the sign-in, with nothing to do again after it.
  const invited = () => {
    const invite = sessionStorage.getItem("one.invite");
    const query = location.search.slice(1);
    if (!invite || !new URLSearchParams(query).get("client_id")) return false;
    location.replace("/invite?" + query + "#" + invite);
    return true;
  };
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
      excludeCredentials: (o.excludeCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })) };
    const c = await navigator.credentials.create({ publicKey });
    const made = await api("/api/auth/passkey/verify-registration", { response: credentialJSON(c), ...(context ? { createSession: true } : {}) });
    return { credential: c.id, id: made.id };
  }
  async function withPasskey(only) {
    const o = await options(ASK.in);
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), allowCredentials: only ? [{ type: "public-key", id: b64u.decode(only) }] : (o.allowCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })) };
    const c = await navigator.credentials.get({ publicKey });
    await api("/api/auth/passkey/verify-authentication", { response: credentialJSON(c) });
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
  const { css, pages, parts, motion } = drawn;

  /// The page `name` draws, filled in with `look` (the network's `name`, `yours` and `logo`, the first `provider`'s name, a
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
<body>${body}${templates}<script>${motion}const { rise, appear, transition } = onboardingMotion;${script ? webauthn + script : ""}
rise(document.querySelector("[data-slot=welcome-stack]"));</script></body></html>`;
  };

  /// Signing in, to the network or, sent here by a client's authorization (\`oauth_query\`), to that client: the client
  /// is the user, with everything they can use, so it's asked each time. With `providers` the network has registered
  /// with, it's a button for each and a passkey; with none, a passkey. `back` is where a sign-in with no client goes on to.
  function signInPage(look) {
    const { providers = [], domain = "", back = "/" } = look;
    if (providers.length) return providersPage(look, providers, domain, back);
    const script = `${acts}
      if (invited()) throw 0;
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
          location.href = client_id ? await consent(oauth_query) : ${json(back)};
        } catch (err) {
          back();
          ready(ASK.in);
          quietly(err);
        }
      };`;
    return page("sign-in", look, script);
  }

  function providersPage(look, providers, domain, signedIn) {
    const script = `${acts}
      if (invited()) throw 0;
      // A provider's answer comes back here, with an error added to the query if it said no.
      const raw = location.search.slice(1);
      const failed = /(^|&)error=/.test(raw);
      const oauth_query = raw.replace(/(^|&)error(_description)?=[^&]*/g, "").replace(/^&/, "");
      const client_id = new URLSearchParams(oauth_query).get("client_id");
      const network = ${json(look.name)};
      const domain = ${json(domain)};
      const providers = ${json(providers)};
      const here = location.pathname + (oauth_query ? "?" + oauth_query : "");
      const back = client_id ? here : ${json(signedIn)};
      // Why its answer didn't sign them in: no user with that account is no one invited, or an account made by a
      // passkey, which isn't linked to it.
      const why = new URLSearchParams(raw);
      if (failed) say(["unable_to_create_user", "account_not_linked"].includes(why.get("error")) ? "No one in " + network + " has that account. Ask an admin to invite you, or sign in with your passkey." : "That didn't sign you in." + (domain ? " Use your account at " + domain + "." : ""));
      const button = slot("action");
      // Back from a provider, which this tab went to a moment ago: it says so while it goes on, not offering to start again.
      const returning = !failed && Date.now() - Number(sessionStorage.getItem("provider") || 0) < 600000;
      sessionStorage.removeItem("provider");
      let waiting = returning ? busy(button, "Signing You In…") : null;
      if (client_id) (async () => {
        try {
          const about = await api("/sign-in/about?" + oauth_query);
          const title = "Sign in to " + about.client;
          document.title = title;
          slot("title").textContent = title;
          if (!about.web) acts(about);
          // Signed in since this client asked: on to it.
          if (about.fresh && !failed) {
            waiting ||= busy(button, "Signing You In…");
            return location.replace(await consent(oauth_query));
          }
        } catch (err) { say(err.message); }
        waiting?.();
      })();
      else waiting?.();
      const start = (provider) => async (e) => {
        say();
        const undo = busy(e.currentTarget, "Opening " + provider.name + "…");
        try {
          // Back to this page, with its no, also for the web client, whose sign-in goes on to its own page.
          const out = await api("/api/auth/sign-in/social" + FOR_DEVICE, { provider: provider.id, callbackURL: back, errorCallbackURL: here });
          sessionStorage.setItem("provider", String(Date.now()));
          location.href = out.url;
        } catch (err) {
          undo();
          say(err.message);
        }
      };
      button.onclick = start(providers[0]);
      for (const provider of providers.slice(1)) {
        const another = part("provider");
        another.textContent = "Sign In with " + provider.name;
        another.onclick = start(provider);
        slot("other").before(another);
      }
      slot("other").onclick = async (e) => {
        say();
        const back_ = busy(e.currentTarget, "Signing In…");
        try {
          await withPasskey();
          location.href = client_id ? await consent(oauth_query) : back;
        } catch (err) {
          back_();
          quietly(err);
        }
      };`;
    return page("sign-in-provider", look, script);
  }

  /// Signing in on a throwaway network (`one-dev throwaway`), with no passkey, straight on, so a client being tested signs in with nobody
  /// there: as whoever this browser is signed in as here (its first user, or someone who joined in it), or, with
  /// `?as=<name>` and no client, as someone who joined, for a device's browser before its app signs in. The button is for
  /// a browser that won't go on to an app by itself.
  function throwawayPage(look) {
    const script = `
      if (invited()) throw 0;
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
          location.href = params.get("client_id") ? await consent(oauth_query) : "/";
        } catch (err) {
          undo();
          say(err.message);
        }
      };
      button.onclick = go;
      go();`;
    return page("sign-in-throwaway", look, script);
  }

  /// Joining with an invite link, its secret in the fragment, which goes only to auth's sign-up. Opened as it was sent, it
  /// goes on through the web client, whose sign-in comes back here (`invited`), so what joining makes is that sign-in and
  /// nothing is asked twice. Then it makes a first passkey, or signs in with a provider (`providers`, for an invite that
  /// names the account), or on a throwaway network (`throwaway`), with nothing to make.
  function invitePage(look) {
    const { throwaway, providers = [] } = look;
    const script = `
      const secret = location.hash.slice(1);
      const throwaway = ${throwaway ? "true" : "false"}, providers = ${json(providers)};
      const oauth_query = location.search.slice(1);
      const params = new URLSearchParams(oauth_query);
      const client_id = params.get("client_id");
      if (secret && !client_id) {
        localStorage.removeItem("one.signin");
        sessionStorage.setItem("one.invite", secret);
        location.replace("/");
        throw 0;
      }
      const button = slot("action");
      let provider = null;
      button.disabled = true;
      // A provider's answer comes back here with an error if it didn't sign them up.
      if (/(^|&)error=/.test(location.search.slice(1))) say("That account isn't the one this invite is for.");
      // Whoever was signed in here before, the invite is for its user: signed out as the page opens, beside asking
      // whose invite it is, and then the passkey's options, so the prompt comes the moment Join is tapped.
      const out = api("/api/auth/sign-out", {}).catch(() => {});
      if (!throwaway && !providers.length) out.then(() => ready(register(secret)));
      const next = () => (client_id ? "/sign-in?" + oauth_query : "/");
      const withProvider = async (p, e) => {
        const undo = busy(e.currentTarget, "Opening " + p.name + "…");
        try {
          sessionStorage.setItem("one.invite", secret);
          await out;
          await api("/invite/provider", { secret });
          const back = "/invite" + location.search + "#" + secret;
          location.href = (await api("/api/auth/sign-in/social", { provider: p.id, callbackURL: next(), errorCallbackURL: back })).url;
        } catch (err) {
          undo();
          say(err.message);
        }
      };
      (async () => {
        try {
          const about = await api("/invite/about", { secret });
          slot("title").textContent = "Welcome, " + about.user;
          // An invite that names an account is taken up with it; another with a first passkey.
          if (about.email && providers.length) {
            provider = providers[0];
            slot("text").textContent = "With your account, " + about.email + ".";
            button.textContent = "Sign In with " + provider.name;
            for (const p of providers.slice(1)) {
              const another = part("provider");
              another.textContent = "Sign In with " + p.name;
              another.onclick = (e) => { say(); withProvider(p, e); };
              button.after(another);
            }
          }
          button.disabled = false;
        } catch (err) {
          // Used already, as when this tab joined: it signs in like anyone.
          if (client_id) {
            sessionStorage.removeItem("one.invite");
            return location.replace("/sign-in?" + oauth_query);
          }
          say(err.message);
        }
      })();
      button.onclick = async (e) => {
        say();
        if (provider) return withProvider(provider, e);
        const undo = busy(button, throwaway ? "Joining…" : "Making a Passkey…");
        try {
          // Kept in this tab for the web client, whose request to let in this user's first device proves it.
          sessionStorage.setItem("one.invite", secret);
          await out;
          if (throwaway) await api("/api/auth/throwaway/join", { secret, join: params.get("join") || undefined });
          else await newPasskey(secret);
          location.href = client_id ? await consent(oauth_query) : next();
        } catch (err) {
          undo();
          if (!throwaway) ready(register(secret));
          quietly(err);
        }
      };`;
    return page("invite", look, script);
  }

  /// Linking one more identity from a device (`join`, in the query), only just signed in: a provider's account, or a new
  /// passkey, then tapped once more so what it signs names the device.
  function linkPage(look) {
    const { providers = [] } = look;
    const script = `
      const providers = ${json(providers)};
      const params = new URLSearchParams(location.search);
      const here = "/sign-in/link?join=" + JOIN;
      const done = () => transition(() => {
        const linked = part("linked");
        document.querySelector("[data-slot=welcome]").replaceWith(linked);
        rise(linked.querySelector("[data-slot=welcome-stack]"));
      });
      if (params.get("error")) say("That account can't be linked. It may be someone else's, or not at this network's domain.");
      if (params.get("linked")) (async () => {
        try {
          await api("/sign-in/link/done", { join: JOIN, provider: params.get("linked") });
          history.replaceState(null, "", here);
          done();
        } catch (err) { say(err.message); }
      })();
      const link = (provider) => async (e) => {
        say();
        const undo = busy(e.currentTarget, "Opening " + provider.name + "…");
        try {
          location.href = (await api("/api/auth/link-social" + FOR_DEVICE, { provider: provider.id, callbackURL: here + "&linked=" + provider.id, errorCallbackURL: here })).url;
        } catch (err) {
          undo();
          say(err.message);
        }
      };
      for (const provider of providers) {
        const button = part("provider");
        button.textContent = "Link " + provider.name;
        button.onclick = link(provider);
        slot("action").after(button);
      }
      slot("action").onclick = async (e) => {
        say();
        const undo = busy(e.currentTarget, "Adding…");
        try {
          const { credential } = await newPasskey();
          await withPasskey(credential);
          done();
        } catch (err) {
          undo();
          quietly(err);
        }
      };`;
    return page("link", look, script, ["linked"]);
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
  /// passkey (or a provider); `done` says how that went, once it's back.
  function codePage(look) {
    const { error = "", done } = look;
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
    return page("code", look, script);
  }

  return { page, signInPage, throwawayPage, invitePage, linkPage, passkeysPage, codePage };
}
