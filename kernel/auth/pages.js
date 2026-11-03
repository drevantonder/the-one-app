// The auth service's screens: signing in, to the install or to a client, joining with an invite link, and a person's
// passkeys. Plain HTML with a little script, served from the auth service at the install's address, so passkeys
// belong to its name.

import kit from "../web/screen/kit.css" with { type: "text" };

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// The look of the web client's welcome: the screens' design kit (src/screen/kit.css, which the kernel is built with),
// the white label's logo, light or dark with the system. Inline, as the page's policy allows nothing else.
const style = `${kit}
  /* Near the top, not centred: text that comes in later (a client's name, a note) grows the page down, never moving
     what's already there. */
  body { max-width: none; min-height: 100vh; padding: max(40px, 14vh) 24px 40px; display: grid; place-items: start center; background: var(--bg); }
  main { width: min(380px, 100%); text-align: center; }
  .mark { display: block; width: 88px; height: 88px; margin: 0 auto 18px; }
  h1 { font-size: 26px; font-weight: 600; margin: 0 0 8px; text-wrap: balance; }
  p { margin: 0 0 22px; color: var(--muted); }
  main .button { display: flex; width: 100%; font-size: 17px; padding: 11px 16px; margin: 0 0 10px; border-radius: 12px; }
  main .button:not(.primary) { background: var(--card); }
  input { display: block; width: 100%; font: inherit; text-align: center; padding: 11px 16px; margin: 0 0 10px; border-radius: 12px; border: 0.5px solid var(--line); background: var(--card); color: var(--text); }
  .note { font-size: 13px; margin: 8px 0 0; }
  .callout { color: var(--text); text-align: left; padding: 12px 16px; border-radius: 12px; background: var(--card); }
  .list { text-align: left; margin: 0 0 22px; }
  .list .button { width: auto; font-size: 15px; padding: 6px 12px; margin: 0; background: var(--bg); }
  .error { color: var(--danger); min-height: 1.4em; margin: 8px 0 0; }
  [hidden] { display: none !important; }
  .framed body { min-height: 0; place-items: start center; padding-top: 24px; }
  .framed .mark, .framed h1 { display: none; }
`;

/// What a passkey without PRF is told, here and by the auth service.
export const NO_PRF = "This passkey can't unlock your data, so it can't be used here. Passkeys saved in iCloud Keychain (iPhone, iPad, Mac) or Google Password Manager (Android, Chrome) work, as do most security keys on a current system. Choose one of those and try again.";

// WebAuthn's binary fields, to and from the base64url its JSON uses; and asking for a passkey with its PRF output,
// which opens the person's keys at the vault. A passkey without PRF opens nothing, so it's refused here and by the
// auth service. Only whether a passkey has PRF goes to the auth service when it's made, never its output.
const webauthn = `
  const b64u = {
    decode: (s) => Uint8Array.from(atob(s.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)),
    encode: (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, ""),
  };
  const PRF_SALT = new TextEncoder().encode("one/passkey-prf");
  const NO_PRF = ${JSON.stringify(NO_PRF)};
  const say = (text) => (document.querySelector(".error").textContent = text || "");
  const quietly = (err) => say(err.name === "NotAllowedError" || err.name === "AbortError" ? "" : err.message);
  async function api(path, body) {
    const resp = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error || out.message || "That didn't work."), { status: resp.status });
    return out;
  }
  function credentialJSON(c, extensions = {}) {
    const r = c.response;
    const response = { clientDataJSON: b64u.encode(r.clientDataJSON) };
    if (r.attestationObject) Object.assign(response, { attestationObject: b64u.encode(r.attestationObject), transports: r.getTransports?.() || [] });
    if (r.authenticatorData) Object.assign(response, { authenticatorData: b64u.encode(r.authenticatorData), signature: b64u.encode(r.signature), userHandle: r.userHandle && b64u.encode(r.userHandle) });
    return { id: c.id, rawId: b64u.encode(c.rawId), type: c.type, response, clientExtensionResults: extensions, authenticatorAttachment: c.authenticatorAttachment };
  }
  const prfOf = (c) => { const first = c.getClientExtensionResults().prf?.results?.first; return first ? b64u.encode(first) : null; };
  /// A button busy with what its tap started: disabled at once, a spinner and \`label\` in it. → what puts it back.
  function busy(b, label) {
    const was = b.innerHTML;
    b.disabled = true;
    b.innerHTML = '<span class="spinner small" aria-hidden="true"></span>' + label;
    const undo = () => { b.innerHTML = was; b.disabled = false; };
    // Come back to with Back, after it went on, it's ready again.
    addEventListener("pageshow", (e) => e.persisted && undo(), { once: true });
    return undo;
  }
  /// A passkey prompt's options, asked for before the tap (\`ready\`), so the prompt comes the moment it's tapped. Each
  /// is used once, and only while it's fresh: the auth service keeps one challenge, for five minutes.
  const ASK = { in: "/api/auth/passkey/generate-authenticate-options", new: "/api/auth/passkey/generate-register-options" };
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
  /// Makes a passkey, for someone signed in, or with an invite link's secret as \`context\` (which signs them in): → its
  /// credential id, its id in the auth service, and its PRF output (asked for again if making it didn't give it).
  async function newPasskey(context) {
    const o = await options(register(context));
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), user: { ...o.user, id: b64u.decode(o.user.id) },
      excludeCredentials: (o.excludeCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })), extensions: { prf: { eval: { first: PRF_SALT } } } };
    const c = await navigator.credentials.create({ publicKey });
    let prf = prfOf(c);
    if (!prf && !c.getClientExtensionResults().prf?.enabled) throw new Error(NO_PRF);
    const made = await api("/api/auth/passkey/verify-registration", { response: credentialJSON(c, { prf: { enabled: true } }), ...(context ? { createSession: true } : {}) });
    if (!prf) prf = (await withPasskey([c.id])).prf;
    return { credential: c.id, id: made.id, prf };
  }
  /// Signs in with a passkey (one of \`only\`'s credential ids, if given), refusing one without PRF: → its credential
  /// id and PRF output.
  async function withPasskey(only) {
    const o = await options(ASK.in);
    const allow = only ? only.map((id) => ({ type: "public-key", id })) : o.allowCredentials || [];
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), allowCredentials: allow.map((c) => ({ ...c, id: b64u.decode(c.id) })),
      extensions: { prf: { eval: { first: PRF_SALT } } } };
    const c = await navigator.credentials.get({ publicKey });
    const prf = prfOf(c);
    if (!prf) throw new Error(NO_PRF);
    await api("/api/auth/passkey/verify-authentication", { response: credentialJSON(c) });
    return { credential: c.id, prf };
  }
`;

/// What a client other than the web client is told: what it will do, and where it goes, since its name is only what it
/// calls itself. Uses the page's install.
const acts = `
  function acts(about) {
    const what = document.getElementById("what");
    what.textContent = about.client + " will act as you, with everything you can use in " + install + ", until you sign it out.";
    const where = document.createElement("p");
    where.className = "callout";
    where.textContent = about.goes === "an app on this device" ? "It's an app on this device." : about.goes === "where you got your code" ? "It's where you got your code. Only go on if you got it yourself, on your own device." : "It goes to " + about.goes + ". Only go on if that's who you meant to sign in to.";
    what.after(where);
  }
`;

/// A page, its colours for the browser's bar the kit's page colour (tests/contracts.rs).
const page = (title, body, script) => `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" media="(prefers-color-scheme: light)" content="#ffffff">
<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#141414"><title>${escape(title)}</title><style>${style}</style></head>
<body><main><img class="mark" src="/logo.png" alt="">${body}<p class="error"></p></main><script>${webauthn}${script}</script></body></html>`;

/// Which passkeys work, in plain words, for the pages that make or ask for one.
const WORKS = "Passkeys saved in iCloud Keychain, Google Password Manager or a security key work, on a current system.";

/// Signing in, to the install or, sent here by a client's authorization (\`oauth_query\`), to that client: the client
/// is the person, with everything they can use, so the passkey is all it asks for. With Google (\`google\`, the
/// install's domain), it's signing in with Google, each time.
export function signInPage(look) {
  const { name, google } = look;
  if (google) return googlePage(look);
  const body = `
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">With your passkey: it's what unlocks your data.</p>
    <button class="button primary" id="passkey">Sign In with a Passkey</button>
    <p class="note">No passkey here yet? Use one from another device, or ask whoever runs ${escape(name)} for an invite link.</p>`;
  const script = `${acts}
    const oauth_query = location.search.slice(1);
    const client_id = new URLSearchParams(oauth_query).get("client_id");
    const install = ${JSON.stringify(name).replaceAll("<", "\\u003c")};
    if (client_id) (async () => {
      try {
        const about = await api("/sign-in/about?" + oauth_query);
        const title = "Sign in to " + about.client + (about.person ? " as " + about.person : "");
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
        const { prf, credential } = await withPasskey();
        location.href = client_id ? (await api("/sign-in/accept", { oauth_query, prf, credential })).url : "/";
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
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">With your Google account at ${escape(google)}.</p>
    <button class="button primary" id="google">Sign In with Google</button>`;
  const script = `${acts}
    // Google's answer comes back here, with an error added to the query if it said no.
    const raw = location.search.slice(1);
    const failed = /(^|&)error=/.test(raw);
    const oauth_query = raw.replace(/(^|&)error(_description)?=[^&]*/g, "").replace(/^&/, "");
    const client_id = new URLSearchParams(oauth_query).get("client_id");
    const install = ${JSON.stringify(name).replaceAll("<", "\\u003c")};
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
    };`;
  return page(`Sign in to ${name}`, body, script);
}

/// Signing in on a try install (`one try`), with no passkey, straight on, so a client being tested signs in with nobody
/// there: as whoever this browser is signed in as here (its first person, or someone who joined in it), or, with
/// `?as=<name>` and no client, as someone who joined, for a device's browser before its app signs in. The button is for
/// a browser that won't go on to an app by itself.
export function tryPage(look) {
  const { name, person } = look;
  const body = `
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">For testing: no passkey needed.</p>
    <button class="button primary" id="go">Sign In as ${escape(person)}</button>`;
  const script = `
    const oauth_query = location.search.slice(1);
    const params = new URLSearchParams(oauth_query);
    const button = document.getElementById("go");
    const go = async () => {
      say();
      const undo = busy(button, "Signing In…");
      try {
        const { person } = await api("/api/auth/try/sign-in", params.get("as") ? { person: params.get("as") } : {});
        if (params.get("as")) {
          undo();
          button.textContent = "Sign In as " + person;
          return void (document.getElementById("what").textContent = "This browser is signed in here as " + person + ".");
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

/// Joining with an invite link: making a first passkey, or on a try install (`tried`), with nothing to make.
export function invitePage(look) {
  const { name, yours, tried } = look;
  const body = `
    <section id="join">
      <h1 id="title">Join ${escape(name)}</h1>
      ${tried
        ? `<p id="what">For testing: no passkey needed.</p>
      <button class="button primary" id="passkey" disabled>Join</button>`
        : `<p id="what">Make a passkey to join. It's how you sign in, and the only thing that unlocks your data.</p>
      <button class="button primary" id="passkey" disabled>Make a Passkey</button>
      <p class="note">${escape(WORKS)}</p>`}
    </section>
    <section id="done" hidden>
      <h1>You've joined ${escape(yours)}</h1>
      ${tried ? "" : `<p class="callout">Add a second passkey, on another device or a security key, so losing this one doesn't lock you out.</p>
      <a class="button primary" href="/passkeys">Add Another Passkey</a>`}
      <a class="button${tried ? " primary" : ""}" href="/">Open ${escape(name)}</a>
    </section>`;
  const script = `
    const token = location.hash.slice(1);
    const tried = ${tried ? "true" : "false"};
    const button = document.getElementById("passkey");
    // Whoever was signed in here before, the passkey is the invite's person's: signed out as the page opens, beside
    // asking whose invite it is, and then the passkey's options, so the prompt comes the moment Make a Passkey is tapped.
    const out = api("/api/auth/sign-out", {}).catch(() => {});
    if (!tried) out.then(() => ready(register(token)));
    (async () => {
      try {
        const about = await api("/invite/about", { token });
        document.getElementById("title").textContent = "Welcome, " + about.person;
        button.disabled = false;
      } catch (err) { say(err.message); }
    })();
    button.onclick = async () => {
      say();
      const undo = busy(button, tried ? "Joining…" : "Making a Passkey…");
      try {
        await out;
        if (tried) await api("/api/auth/try/join", { token });
        else {
          const { prf, credential } = await newPasskey(token);
          await api("/invite/keys", { token, prf, credential });
        }
        history.replaceState(null, "", "/invite");
        document.getElementById("join").hidden = true;
        document.getElementById("done").hidden = false;
      } catch (err) {
        undo();
        if (!tried) ready(register(token));
        quietly(err);
      }
    };`;
  return page(`Join ${name}`, body, script);
}

export function passkeysPage(look) {
  const { name } = look;
  const body = `
    <h1>Passkeys</h1>
    <p id="what">Each one signs you in to ${escape(name)} and unlocks your data.</p>
    <div class="loading" id="wait" role="status" aria-label="Loading"><span class="spinner" aria-hidden="true"></span></div>
    <button class="button primary" id="signin" hidden>Sign In with a Passkey</button>
    <section id="mine" hidden>
      <div class="list" id="list"></div>
      <p class="callout" id="suggest" hidden>Add a second passkey on another device or a security key. Lose them all, and only whoever holds the recovery key can get you back in.</p>
      <button class="button primary" id="add">Add a Passkey</button>
      <p class="note" id="step">${escape(WORKS)} Adding one asks first for a passkey you have, then makes the new one.</p>
    </section>`;
  const script = `
    // In the web client, beside its sidebar, whose bar already says Passkeys.
    if (window.top !== window) document.documentElement.classList.add("framed");
    const esc = (t) => String(t).replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";");
    const when = (d) => new Date(d).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
    async function show() {
      let list;
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
      document.getElementById("list").innerHTML = list.map((p) => '<div class="row plain"><div class="main"><div class="title"><span>' + esc(p.name || "Passkey") + '</span></div><div class="sub">Added ' + esc(when(p.createdAt)) + '</div></div>' +
        (list.length > 1 ? '<button class="button" data-id="' + esc(p.id) + '">Remove</button>' : '') + '</div>').join("");
    }
    // Both buttons start with a passkey they have: its prompt's options are asked for now, so it comes at once.
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
      const b = e.currentTarget, undo = busy(b, "Signing In…");
      let made;
      try {
        const have = await withPasskey();
        b.lastChild.textContent = "Adding…";
        step.textContent = "Now make the new passkey.";
        made = await newPasskey();
        await api("/passkeys/add", { from: have.prf, from_credential: have.credential, prf: made.prf, credential: made.credential });
        made = null;
        await show();
      } catch (err) {
        // A new passkey that doesn't open the person's keys isn't kept.
        if (made?.id) await api("/api/auth/passkey/delete-passkey", { id: made.id }).catch(() => {});
        quietly(err);
      }
      undo();
      step.textContent = steps;
      ready(ASK.in);
    };
    document.getElementById("list").onclick = async (e) => {
      const id = e.target.dataset?.id;
      if (!id || !confirm("Remove this passkey? It won't sign you in or unlock your data again.")) return;
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
      ? `<h1>You're signed in</h1><p>Go back to where you got the code: it carries on by itself.</p>`
      : `<h1>That didn't sign in</h1><p>Start signing in again where you got the code.</p>`;
    return page(`Sign in to ${name}`, body, "");
  }
  const body = `
    <h1>Sign in with a code</h1>
    <p>Type the code you were shown, then sign in with ${google ? "Google" : "your passkey"}. Only use a code you got yourself, on your own device.</p>
    <form id="form"><input id="code" autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="BCDF-GHJK" aria-label="Code">
    <button class="button primary">Continue</button></form>`;
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
