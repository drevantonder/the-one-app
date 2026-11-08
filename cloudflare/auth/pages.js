// Auth's screens: signing in, to the network or to a client, joining with an invite link, and a user's
// passkeys. Plain HTML with a little script, served from auth at the network's address, so passkeys
// belong to its name.

import css from "../web/screen/ui/auth.css" with { type: "text" };

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// The look of these pages is @one/ui's (src/screen/ui): its theme (shadcn's neutral, light or dark with the system) and,
// of Tailwind's utilities, only those these pages use, which @one/ui's build (bun run build there) finds in this file's
// class names and writes as auth.css. Not React, as the screens are: the Worker that serves a browser's code before it
// is signed in stays small, a few KB of CSS and this script, and nothing here has to be drawn again after it's drawn.
// Inline, as the page's policy allows nothing else. The classes are shadcn's own, as the components have them (button.tsx,
// input.tsx, problem.tsx, ListRow), written out since there's no component to ask.
const BUTTON = "mx-auto mb-2.5 flex h-10 w-fit min-w-56 shrink-0 items-center justify-center gap-2 rounded-md px-6 text-sm font-medium whitespace-nowrap outline-none transition-all focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50";
const DEFAULT = `${BUTTON} bg-primary text-primary-foreground hover:bg-primary/90`;
const SECONDARY = `${BUTTON} bg-secondary text-secondary-foreground hover:bg-secondary/80`;
const GHOST = "inline-flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-md px-3 text-sm font-medium whitespace-nowrap outline-none transition-all hover:bg-accent hover:text-accent-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 dark:hover:bg-accent/50";
const INPUT = "mb-2.5 h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1 text-center text-base shadow-xs outline-none placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 md:text-sm dark:bg-input/30";
const HEADING = "mt-2.5 text-3xl font-semibold tracking-tight text-balance framed:hidden";
const TITLE = `${HEADING} mb-2`;
const TITLE_THEN_BUTTON = `${HEADING} mb-5`;
const TEXT = "mb-5 text-sm text-muted-foreground";
const NOTE = "mt-2 text-sm text-muted-foreground";
const CALLOUT = "mb-5 text-left text-sm";
const LIST = "mb-5 w-full divide-y overflow-hidden rounded-xl border bg-card text-left text-card-foreground";
const SPINNER = (size) => `<svg class="${size} animate-spin" viewBox="0 0 256 256" fill="currentColor" role="status" aria-label="Loading"><path d="M232,128a104,104,0,0,1-208,0c0-41,23.81-78.36,60.66-95.27a8,8,0,0,1,6.68,14.54C60.15,61.59,40,93.27,40,128a88,88,0,0,0,176,0c0-34.73-20.15-66.41-51.34-80.73a8,8,0,0,1,6.68-14.54C208.19,49.64,232,87,232,128Z"/></svg>`;

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
  // A problem, as @one/ui draws one (problem.tsx): its symbol, what happened, and what to do after the first sentence.
  const say = (text) => {
    const [title, ...rest] = String(text || "").split(/(?<=[.!?])\\s+/);
    document.querySelector(".problems").innerHTML = text ? '<div class="relative grid w-full grid-cols-[1rem_1fr] items-start gap-x-3 gap-y-0.5 rounded-lg border bg-card px-4 py-3 text-left text-sm text-destructive" role="alert"><svg class="size-4 translate-y-0.5" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d="M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm-8,56a8,8,0,0,1,16,0v56a8,8,0,0,1-16,0Zm8,104a12,12,0,1,1,12-12A12,12,0,0,1,128,184Z"/></svg><div class="problem-title col-start-2 font-medium tracking-tight"></div><div class="problem-text col-start-2 text-muted-foreground"></div></div>' : "";
    if (!text) return;
    document.querySelector(".problem-title").textContent = title;
    document.querySelector(".problem-text").textContent = rest.join(" ");
    document.querySelector(".problem-text").hidden = !rest.length;
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
    const was = b.innerHTML;
    b.disabled = true;
    b.innerHTML = ${JSON.stringify(SPINNER("size-4"))} + label;
    const undo = () => { b.innerHTML = was; b.disabled = false; };
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
    const what = document.getElementById("what");
    what.textContent = about.client + " will act as you, with everything you can use in " + network + ", until you sign it out.";
    const where = document.createElement("p");
    where.className = ${JSON.stringify(CALLOUT)};
    where.textContent = about.goes === "an app on this device" ? "It's an app on this device." : about.goes === "where you got your code" ? "It's where you got your code. Only go on if you got it yourself, on your own device." : "It goes to " + about.goes + ". Only go on if that's who you meant to sign in to.";
    what.after(where);
  }
`;

/// A page, near the top, not centred: text that comes in later (a client's name, a note) grows the page down, never
/// moving what's already there. In the web client's frame (the passkeys' page, beside its sidebar, whose bar already says
/// Passkeys) it has no mark or title.
const page = (title, body, script) => `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" media="(prefers-color-scheme: light)" content="#ffffff">
<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#141414"><title>${escape(title)}</title><link rel="icon" href="/logo.png"><style>${css}</style></head>
<body class="grid min-h-screen items-start justify-items-center px-6 pt-[max(40px,14vh)] pb-10 framed:min-h-0 framed:pt-6"><main class="w-[min(380px,100%)] text-center"><img class="mx-auto block size-28 framed:hidden" src="/logo.png" alt="">${body}<div class="problems empty:hidden mt-3.5" role="status"></div></main><script>${webauthn}${script}</script></body></html>`;

const WORKS = "Passkeys saved in iCloud Keychain, Google Password Manager or a security key work, on a current system.";

/// Signing in, to the network or, sent here by a client's authorization (\`oauth_query\`), to that client: the client
/// is the user, with everything they can use, so it's asked each time. With Google (\`google\`, the network's
/// domain), it's signing in with Google, or with a passkey for an admin.
export function signInPage(look) {
  const { name, google } = look;
  if (google) return googlePage(look);
  const body = `
    <h1 class="${TITLE}" id="title">Sign in to ${escape(name)}</h1>
    <p class="${TEXT}" id="what">With your passkey.</p>
    <button class="${DEFAULT}" id="passkey">Sign In with a Passkey</button>
    <p class="${NOTE}">No passkey here yet? Use one from another device, or ask whoever runs ${escape(name)} for an invite link.</p>`;
  const script = `${acts}
    const oauth_query = location.search.slice(1);
    const client_id = new URLSearchParams(oauth_query).get("client_id");
    const network = ${JSON.stringify(name).replaceAll("<", "\\u003c")};
    if (client_id) (async () => {
      try {
        const about = await api("/sign-in/about?" + oauth_query);
        const title = "Sign in to " + about.client + (about.user ? " as " + about.user : "");
        document.title = title;
        document.getElementById("title").textContent = title;
        if (!about.web) acts(about);
      } catch (err) { say(err.message); }
    })();
    ready(ASK.in);
    document.getElementById("passkey").onclick = async (e) => {
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
  return page(`Sign in to ${name}`, body, script);
}

function googlePage(look) {
  const { name, google } = look;
  const body = `
    <h1 class="${TITLE}" id="title">Sign in to ${escape(name)}</h1>
    <p class="${TEXT}" id="what">With your Google account at ${escape(google)}.</p>
    <button class="${DEFAULT}" id="google">Sign In with Google</button>
    <button class="${SECONDARY}" id="passkey">Sign In with a Passkey</button>
    <p class="${NOTE}">Admins sign in with a passkey.</p>`;
  const script = `${acts}
    // Google's answer comes back here, with an error added to the query if it said no.
    const raw = location.search.slice(1);
    const failed = /(^|&)error=/.test(raw);
    const oauth_query = raw.replace(/(^|&)error(_description)?=[^&]*/g, "").replace(/^&/, "");
    const client_id = new URLSearchParams(oauth_query).get("client_id");
    const network = ${JSON.stringify(name).replaceAll("<", "\\u003c")};
    const domain = ${JSON.stringify(google).replaceAll("<", "\\u003c")};
    const back = client_id ? "/sign-in?" + oauth_query : "/";
    if (failed) say("That didn't sign you in. Use your Google account at " + domain + ".");
    const button = document.getElementById("google");
    // Back from Google, which this tab went to a moment ago: it says so while it goes on, not offering to start again.
    const returning = !failed && Date.now() - Number(sessionStorage.getItem("google") || 0) < 600000;
    sessionStorage.removeItem("google");
    let waiting = returning ? busy(button, "Signing You In…") : null;
    if (client_id) (async () => {
      try {
        const about = await api("/sign-in/about?" + oauth_query);
        const title = "Sign in to " + about.client;
        document.title = title;
        document.getElementById("title").textContent = title;
        if (!about.web) acts(about);
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
        if (client_id) await api("/sign-in/google", { oauth_query });
        const out = await api("/api/auth/sign-in/social", { provider: "google", callbackURL: back, errorCallbackURL: back });
        sessionStorage.setItem("google", String(Date.now()));
        location.href = out.url;
      } catch (err) {
        undo();
        say(err.message);
      }
    };
    document.getElementById("passkey").onclick = async (e) => {
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
  return page(`Sign in to ${name}`, body, script);
}

/// Signing in on a throwaway network (`one throwaway`), with no passkey, straight on, so a client being tested signs in with nobody
/// there: as whoever this browser is signed in as here (its first user, or someone who joined in it), or, with
/// `?as=<name>` and no client, as someone who joined, for a device's browser before its app signs in. The button is for
/// a browser that won't go on to an app by itself.
export function throwawayPage(look) {
  const { name, user } = look;
  const body = `
    <h1 class="${TITLE}" id="title">Sign in to ${escape(name)}</h1>
    <p class="${TEXT}" id="what">For testing: no passkey needed.</p>
    <button class="${DEFAULT}" id="go">Sign In as ${escape(user)}</button>`;
  const script = `
    const oauth_query = location.search.slice(1);
    const params = new URLSearchParams(oauth_query);
    const button = document.getElementById("go");
    const go = async () => {
      say();
      const undo = busy(button, "Signing In…");
      try {
        const { user } = await api("/api/auth/throwaway/sign-in", params.get("as") ? { user: params.get("as") } : { join: params.get("join") || undefined });
        if (params.get("as")) {
          undo();
          button.textContent = "Sign In as " + user;
          return void (document.getElementById("what").textContent = "This browser is signed in here as " + user + ".");
        }
        location.href = params.get("client_id") ? (await api("/sign-in/accept", { oauth_query })).url : "/";
      } catch (err) {
        undo();
        say(err.message);
      }
    };
    document.getElementById("go").onclick = go;
    go();`;
  return page(`Sign in to ${name}`, body, script);
}

/// Joining with an invite link, its secret in the fragment, which goes only to auth's sign-up: making a first passkey,
/// or signing in with Google (`google`) as the email it's for, or on a throwaway network (`throwaway`), with nothing to make. With
/// `?code=`, the CLI's (`sign-in --invite`), it goes on to sign that in, as the user who just joined.
export function invitePage(look) {
  const { name, yours, throwaway, google } = look;
  const body = `
    <section id="join">
      <h1 class="${TITLE}" id="title">Join ${escape(name)}</h1>
      ${throwaway
        ? `<p class="${TEXT}" id="what">For testing: no passkey needed.</p>
      <button class="${DEFAULT}" id="go" disabled>Join</button>`
        : google
        ? `<p class="${TEXT}" id="what">With your Google account.</p>
      <button class="${DEFAULT}" id="go" disabled>Sign In with Google</button>`
        : `<p class="${TEXT}" id="what">Make a passkey to join. It's how you sign in.</p>
      <button class="${DEFAULT}" id="go" disabled>Make a Passkey</button>
      <p class="${NOTE}">${escape(WORKS)}</p>`}
    </section>
    <section id="done" hidden>
      <h1 class="${throwaway ? TITLE_THEN_BUTTON : TITLE}">You've joined ${escape(yours)}</h1>
      ${throwaway ? "" : `<p class="${CALLOUT}">Add a second passkey, on another device or a security key, so losing this one doesn't lock you out.</p>
      <a class="${DEFAULT}" href="/passkeys">Add Another Passkey</a>`}
      <a class="${throwaway ? DEFAULT : SECONDARY}" href="/">Open ${escape(name)}</a>
    </section>`;
  const script = `
    const secret = location.hash.slice(1);
    const throwaway = ${throwaway ? "true" : "false"}, google = ${google ? "true" : "false"};
    const code = new URLSearchParams(location.search).get("code");
    const button = document.getElementById("go");
    // Google's answer comes back here with an error if it didn't sign them up.
    if (/(^|&)error=/.test(location.search.slice(1))) say("That Google account isn't the one this invite is for.");
    // Whoever was signed in here before, the invite is for its user: signed out as the page opens, beside asking
    // whose invite it is, and then the passkey's options, so the prompt comes the moment Make a Passkey is tapped.
    const out = api("/api/auth/sign-out", {}).catch(() => {});
    if (!throwaway && !google) out.then(() => ready(register(secret)));
    (async () => {
      try {
        const about = await api("/invite/about", { secret });
        document.getElementById("title").textContent = "Welcome, " + about.user;
        if (google && about.email) document.getElementById("what").textContent = "With your Google account, " + about.email + ".";
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
        document.getElementById("join").hidden = true;
        document.getElementById("done").hidden = false;
      } catch (err) {
        undo();
        if (!throwaway && !google) ready(register(secret));
        quietly(err);
      }
    };`;
  return page(`Join ${name}`, body, script);
}

export function passkeysPage(look) {
  const { name } = look;
  const body = `
    <h1 class="${TITLE}">Passkeys</h1>
    <p class="${TEXT}" id="what">Each one signs you in to ${escape(name)}.</p>
    <div class="mb-5 flex justify-center text-muted-foreground" id="wait">${SPINNER("size-5")}</div>
    <button class="${DEFAULT}" id="signin" hidden>Sign In with a Passkey</button>
    <section id="mine" hidden>
      <div class="${LIST}" id="list"></div>
      <p class="${CALLOUT}" id="suggest" hidden>Add a second passkey, on another device or a security key, so losing one doesn't lock you out.</p>
      <button class="${DEFAULT}" id="add">Add a Passkey</button>
      <p class="${NOTE}" id="step">${escape(WORKS)}</p>
    </section>`;
  const script = `
    if (window.top !== window) document.documentElement.classList.add("framed");
    const esc = (t) => String(t).replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";");
    const when = (d) => new Date(d).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
    let list = [];
    async function show() {
      try { list = await api("/api/auth/passkey/list-user-passkeys"); }
      catch (err) {
        document.getElementById("wait").hidden = true;
        if (err.status !== 401) return say(err.message);
        document.getElementById("signin").hidden = false;
        document.getElementById("mine").hidden = true;
        return;
      }
      document.getElementById("wait").hidden = true;
      document.getElementById("signin").hidden = true;
      document.getElementById("mine").hidden = false;
      document.getElementById("suggest").hidden = list.length > 1;
      document.getElementById("list").innerHTML = list.map((p) => '<div class="flex w-full min-w-0 items-center gap-3 px-3 py-2.5 text-left text-sm"><span class="min-w-0 flex-1"><span class="block truncate font-medium">' + esc(p.name || "Passkey") + '</span><span class="block truncate text-muted-foreground">Added ' + esc(when(p.createdAt)) + '</span></span>' +
        (list.length > 1 ? '<button class="${GHOST}" data-id="' + esc(p.id) + '">Remove</button>' : '') + '</div>').join("");
    }
    show();
    ready(ASK.in);
    document.getElementById("signin").onclick = async (e) => {
      say();
      const undo = busy(e.currentTarget, "Signing In…");
      try { await withPasskey(); await show(); } catch (err) { quietly(err); }
      ready(ASK.in);
      undo();
    };
    const step = document.getElementById("step"), steps = step.textContent;
    document.getElementById("add").onclick = async (e) => {
      say();
      const b = e.currentTarget, undo = busy(b, list.length ? "Signing In…" : "Adding…");
      try {
        if (list.length) {
          await withPasskey();
          b.lastChild.textContent = "Adding…";
          step.textContent = "Now make the new passkey.";
        }
        await newPasskey();
        await show();
      } catch (err) { quietly(err); }
      undo();
      step.textContent = steps;
      ready(ASK.in);
    };
    document.getElementById("list").onclick = async (e) => {
      const id = e.target.dataset?.id;
      if (!id || !confirm("Remove this passkey? It won't sign you in again.")) return;
      busy(e.target, "Removing…");
      try { await api("/api/auth/passkey/delete-passkey", { id }); } catch (err) { say(err.message); }
      await show();
    };`;
  return page(`Passkeys`, body, script);
}

/// Signing in with a code another device shows (the CLI on one with no screen): typing it here goes on to signing in with a
/// passkey (or Google); `done` says how that went, once it's back.
export function codePage(look) {
  const { name, google, error = "", done } = look;
  if (done !== undefined) {
    const body = done
      ? `<h1 class="${TITLE}">You're signed in</h1><p class="${TEXT}">Go back to where you got the code: it carries on by itself.</p>`
      : `<h1 class="${TITLE}">That didn't sign in</h1><p class="${TEXT}">Start signing in again where you got the code.</p>`;
    return page(`Sign in to ${name}`, body, "");
  }
  const body = `
    <h1 class="${TITLE}">Sign in with a code</h1>
    <p class="${TEXT}">Type the code you were shown, then sign in with ${google ? "Google or your passkey" : "your passkey"}. Only use a code you got yourself, on your own device.</p>
    <form id="form"><input class="${INPUT}" id="code" autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="BCDF-GHJK" aria-label="Code">
    <button class="${DEFAULT}">Continue</button></form>`;
  const script = `
    say(${JSON.stringify(error).replaceAll("<", "\\u003c")});
    document.getElementById("code").focus();
    document.getElementById("form").onsubmit = (e) => {
      e.preventDefault();
      busy(e.target.querySelector("button"), "Continuing…");
      location.href = "/sign-in/code?c=" + encodeURIComponent(document.getElementById("code").value);
    };`;
  return page(`Sign in to ${name}`, body, script);
}
