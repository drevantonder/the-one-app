// The auth service's screens: signing in, to the install or to a client, joining with an invite link, and a person's
// passkeys. Plain HTML with a little script, served from the auth service at the install's address, so passkeys
// belong to its name.

import kit from "../web/screen/kit.css" with { type: "text" };

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// The look of the web client's welcome: the screens' design kit (src/screen/kit.css, which the kernel is built with),
// the white label's logo and accent, light or dark with the system. Inline, as the page's policy allows nothing else.
const style = `${kit}
  :root { --window: #f5f5f7; }
  @media (prefers-color-scheme: dark) { :root { --window: #282828; --card: #2c2c2e; } }
  body { max-width: none; min-height: 100vh; padding: 40px 24px; display: grid; place-items: center; background: var(--window); }
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
  .list .button { width: auto; font-size: 15px; padding: 6px 12px; margin: 0; background: var(--window); }
  .error { color: var(--danger); min-height: 1.4em; margin: 8px 0 0; }
  [hidden] { display: none !important; }
`;

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
  const say = (text) => (document.querySelector(".error").textContent = text || "");
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
  /// Keeps a passkey's PRF output in this tab, if it gave one.
  function keepPrf(c) {
    const first = c.getClientExtensionResults().prf?.results?.first;
    if (!first) return;
    sessionStorage.setItem("one.prf", b64u.encode(first));
    sessionStorage.setItem("one.prf.credential", c.id);
  }
  /// Makes a passkey, for someone signed in, or with an invite link's secret as \`context\` (which signs them in): →
  /// its credential id and its id in the auth service.
  async function newPasskey(context) {
    const o = await api("/api/auth/passkey/generate-register-options" + (context ? "?context=" + encodeURIComponent(context) : ""));
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), user: { ...o.user, id: b64u.decode(o.user.id) },
      excludeCredentials: (o.excludeCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })), extensions: { prf: { eval: { first: await PRF_SALT } } } };
    const c = await navigator.credentials.create({ publicKey });
    const made = await api("/api/auth/passkey/verify-registration", { response: credentialJSON(c), ...(context ? { createSession: true } : {}) });
    keepPrf(c);
    return { credential: c.id, id: made.id };
  }
  /// Signs in with a passkey: → its credential id.
  async function withPasskey() {
    const o = await api("/api/auth/passkey/generate-authenticate-options");
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), allowCredentials: (o.allowCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })),
      extensions: { prf: { eval: { first: await PRF_SALT } } } };
    const c = await navigator.credentials.get({ publicKey });
    await api("/api/auth/passkey/verify-authentication", { response: credentialJSON(c) });
    keepPrf(c);
    return { credential: c.id };
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

/// A page of `look`'s, the white label's `{ name, accent }`.
const page = (look, title, body, script) => {
  const accent = /^#[0-9a-f]{3,8}$/i.test(look.accent || "") ? look.accent : "#007aff";
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" content="${accent}"><title>${escape(title)}</title><style>:root { --accent: ${accent}; }${style}</style></head>
<body><main><img class="mark" src="/logo.png" alt="">${body}<p class="error"></p></main><script>${webauthn}${script}</script></body></html>`;
};

/// What a person joins, in their words: "your One App".
const ours = (name) => `your ${name.replace(/^The /, "")}`;

/// Which passkeys work, in plain words, for the pages that make or ask for one.
const WORKS = "Passkeys saved in iCloud Keychain, Google Password Manager or a security key work, on a current system.";

/// Signing in, to the install or, sent here by a client's authorization (\`oauth_query\`), to that client: the client
/// is the person, with everything they can use, so it's asked each time. With Google (\`google\`, the install's
/// domain), it's signing in with Google, or with a passkey for an admin.
export function signInPage(look) {
  const { name, google } = look;
  if (google) return googlePage(look);
  const body = `
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">With your passkey.</p>
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
    document.getElementById("passkey").onclick = async () => {
      say();
      try {
        await withPasskey();
        location.href = client_id ? (await api("/sign-in/accept", { oauth_query, passkey: true })).url : "/";
      } catch (err) { quietly(err); }
    };`;
  return page(look, `Sign in to ${name}`, body, script);
}

function googlePage(look) {
  const { name, google } = look;
  const body = `
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">With your Google account at ${escape(google)}.</p>
    <button class="button primary" id="google">Sign In with Google</button>
    <button class="button" id="passkey">Sign In with a Passkey</button>
    <p class="note">Admins sign in with a passkey.</p>`;
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
    if (client_id) (async () => {
      try {
        const about = await api("/sign-in/about?" + oauth_query);
        const title = "Sign in to " + about.client;
        document.title = title;
        document.getElementById("title").textContent = title;
        if (!about.web) acts(about);
        // Back from the Google sign-in started here for this client, just now: on to it.
        if (about.fresh && !failed) location.replace((await api("/sign-in/accept", { oauth_query })).url);
      } catch (err) { say(err.message); }
    })();
    document.getElementById("google").onclick = async () => {
      say();
      try {
        if (client_id) await api("/sign-in/google", { oauth_query });
        const out = await api("/api/auth/sign-in/social", { provider: "google", callbackURL: back, errorCallbackURL: back });
        location.href = out.url;
      } catch (err) { say(err.message); }
    };
    document.getElementById("passkey").onclick = async () => {
      say();
      try {
        await withPasskey();
        location.href = client_id ? (await api("/sign-in/accept", { oauth_query, passkey: true })).url : "/";
      } catch (err) { quietly(err); }
    };`;
  return page(look, `Sign in to ${name}`, body, script);
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
    const go = async () => {
      say();
      try {
        const { person } = await api("/api/auth/try/sign-in", params.get("as") ? { person: params.get("as") } : {});
        document.getElementById("go").textContent = "Sign In as " + person;
        if (params.get("as")) return void (document.getElementById("what").textContent = "This browser is signed in here as " + person + ".");
        location.href = params.get("client_id") ? (await api("/sign-in/accept", { oauth_query })).url : "/";
      } catch (err) { say(err.message); }
    };
    document.getElementById("go").onclick = go;
    go();`;
  return page(look, `Sign in to ${name}`, body, script);
}

/// Joining with an invite link, its secret in the fragment, which goes only to auth's sign-up: making a first passkey,
/// or signing in with Google (`google`) as the email it's for, or on a try install (`tried`), with nothing to make.
export function invitePage(look) {
  const { name, tried, google } = look;
  const body = `
    <section id="join">
      <h1 id="title">Join ${escape(name)}</h1>
      ${tried
        ? `<p id="what">For testing: no passkey needed.</p>
      <button class="button primary" id="go" disabled>Join</button>`
        : google
        ? `<p id="what">With your Google account.</p>
      <button class="button primary" id="go" disabled>Sign In with Google</button>`
        : `<p id="what">Make a passkey to join. It's how you sign in.</p>
      <button class="button primary" id="go" disabled>Make a Passkey</button>
      <p class="note">${escape(WORKS)}</p>`}
    </section>
    <section id="done" hidden>
      <h1>You've joined ${escape(ours(name))}</h1>
      ${tried ? "" : `<p class="callout">Add a second passkey, on another device or a security key, so losing this one doesn't lock you out.</p>
      <a class="button primary" href="/passkeys">Add Another Passkey</a>`}
      <a class="button${tried ? " primary" : ""}" href="/">Open ${escape(name)}</a>
    </section>`;
  const script = `
    const secret = location.hash.slice(1);
    const tried = ${tried ? "true" : "false"}, google = ${google ? "true" : "false"};
    // Google's answer comes back here with an error if it didn't sign them up.
    if (/(^|&)error=/.test(location.search.slice(1))) say("That Google account isn't the one this invite is for.");
    (async () => {
      try {
        const about = await api("/invite/about", { secret });
        document.getElementById("title").textContent = "Welcome, " + about.person;
        if (google && about.email) document.getElementById("what").textContent = "With your Google account, " + about.email + ".";
        document.getElementById("go").disabled = false;
      } catch (err) { say(err.message); }
    })();
    document.getElementById("go").onclick = async () => {
      say();
      try {
        // Kept in this tab for the web client, whose request to let in this person's first device proves it.
        sessionStorage.setItem("one.invite", secret);
        // Whoever was signed in here before, the invite is for its person.
        await api("/api/auth/sign-out", {}).catch(() => {});
        if (google) {
          await api("/invite/google", { secret });
          const back = "/invite#" + secret;
          location.href = (await api("/api/auth/sign-in/social", { provider: "google", callbackURL: "/", errorCallbackURL: back })).url;
          return;
        }
        if (tried) await api("/api/auth/try/join", { secret });
        else await newPasskey(secret);
        history.replaceState(null, "", "/invite");
        document.getElementById("join").hidden = true;
        document.getElementById("done").hidden = false;
      } catch (err) { quietly(err); }
    };`;
  return page(look, `Join ${name}`, body, script);
}

export function passkeysPage(look) {
  const { name } = look;
  const body = `
    <h1>Passkeys</h1>
    <p id="what">Each one signs you in to ${escape(name)}.</p>
    <button class="button primary" id="signin" hidden>Sign In with a Passkey</button>
    <section id="mine" hidden>
      <div class="list" id="list"></div>
      <p class="callout" id="suggest" hidden>Add a second passkey, on another device or a security key, so losing one doesn't lock you out.</p>
      <button class="button primary" id="add">Add a Passkey</button>
      <p class="note">${escape(WORKS)}</p>
    </section>`;
  const script = `
    const esc = (t) => String(t).replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";");
    const when = (d) => new Date(d).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
    let list = [];
    async function show() {
      try { list = await api("/api/auth/passkey/list-user-passkeys"); }
      catch (err) {
        if (err.status !== 401) return say(err.message);
        document.getElementById("signin").hidden = false;
        document.getElementById("mine").hidden = true;
        return;
      }
      document.getElementById("signin").hidden = true;
      document.getElementById("mine").hidden = false;
      document.getElementById("suggest").hidden = list.length > 1;
      document.getElementById("list").innerHTML = list.map((p) => '<div class="row plain"><div class="main"><div class="title"><span>' + esc(p.name || "Passkey") + '</span></div><div class="sub">Added ' + esc(when(p.createdAt)) + '</div></div>' +
        (list.length > 1 ? '<button class="button" data-id="' + esc(p.id) + '">Remove</button>' : '') + '</div>').join("");
    }
    show();
    document.getElementById("signin").onclick = async () => {
      say();
      try { await withPasskey(); await show(); } catch (err) { quietly(err); }
    };
    // A passkey is added just after signing in: with one they have, if they have one.
    document.getElementById("add").onclick = async () => {
      say();
      try {
        if (list.length) await withPasskey();
        await newPasskey();
        await show();
      } catch (err) { quietly(err); }
    };
    document.getElementById("list").onclick = async (e) => {
      const id = e.target.dataset?.id;
      if (!id || !confirm("Remove this passkey? It won't sign you in again.")) return;
      try { await api("/api/auth/passkey/delete-passkey", { id }); await show(); } catch (err) { say(err.message); }
    };`;
  return page(look, `Passkeys`, body, script);
}

/// Signing in with a code another device shows (the CLI on one with no screen): typing it here goes on to signing in with a
/// passkey (or Google); `done` says how that went, once it's back.
export function codePage(look) {
  const { name, google, error = "", done } = look;
  if (done !== undefined) {
    const body = done
      ? `<h1>You're signed in</h1><p>Go back to where you got the code: it carries on by itself.</p>`
      : `<h1>That didn't sign in</h1><p>Start signing in again where you got the code.</p>`;
    return page(look, `Sign in to ${name}`, body, "");
  }
  const body = `
    <h1>Sign in with a code</h1>
    <p>Type the code you were shown, then sign in with ${google ? "Google or your passkey" : "your passkey"}. Only use a code you got yourself, on your own device.</p>
    <form id="form"><input id="code" autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="BCDF-GHJK" aria-label="Code">
    <button class="button primary">Continue</button></form>`;
  const script = `
    say(${JSON.stringify(error).replaceAll("<", "\\u003c")});
    document.getElementById("code").focus();
    document.getElementById("form").onsubmit = (e) => {
      e.preventDefault();
      location.href = "/sign-in/code?c=" + encodeURIComponent(document.getElementById("code").value);
    };`;
  return page(look, `Sign in to ${name}`, body, script);
}
