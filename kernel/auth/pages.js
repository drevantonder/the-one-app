// The auth service's screens: signing in, to the install or to a client, joining with an invite link, and a person's
// passkeys. Plain HTML with a little script, served from the auth service at the install's address, so passkeys
// belong to its name.

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

const style = `
  :root { color-scheme: light dark; --accent: #0a84ff; font: 16px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }
  main { width: min(360px, calc(100vw - 48px)); }
  h1 { font-size: 22px; font-weight: 600; margin: 0 0 6px; }
  p { margin: 0 0 20px; opacity: .7; }
  button, input, a.button { box-sizing: border-box; display: block; text-align: center; text-decoration: none; width: 100%; font: inherit; padding: 12px 14px; border-radius: 12px; margin: 0 0 10px; }
  input { border: 1px solid color-mix(in srgb, CanvasText 18%, transparent); background: transparent; color: inherit; }
  button { border: 0; background: color-mix(in srgb, CanvasText 8%, transparent); color: inherit; cursor: pointer; }
  a.button { background: color-mix(in srgb, CanvasText 8%, transparent); color: inherit; }
  button:disabled { opacity: .5; cursor: default; }
  button.primary, a.button.primary { background: var(--accent); color: white; font-weight: 600; }
  .note { font-size: 14px; opacity: .6; margin-top: 18px; }
  .callout { opacity: 1; padding: 12px 14px; border-radius: 12px; background: color-mix(in srgb, var(--accent) 10%, transparent); }
  ul.passkeys { list-style: none; margin: 0 0 20px; padding: 0; }
  ul.passkeys li { display: flex; align-items: center; gap: 12px; padding: 12px 0; border-bottom: 1px solid color-mix(in srgb, CanvasText 10%, transparent); }
  ul.passkeys li:last-child { border-bottom: 0; }
  ul.passkeys span { flex: 1; } ul.passkeys small { display: block; opacity: .6; font-size: 14px; }
  ul.passkeys button { width: auto; margin: 0; padding: 6px 12px; font-size: 14px; }
  .error { color: #d70015; min-height: 1.4em; margin-top: 8px; opacity: 1; }
  [hidden] { display: none !important; }
`;

/// What a passkey without PRF is told, here and by the auth service.
export const NO_PRF = "This passkey can't open your keys, so it can't be used here. Passkeys saved in iCloud Keychain (iPhone, iPad, Mac) or Google Password Manager (Android, Chrome) work, as do most security keys on a current system. Choose one of those and try again.";

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
  /// Makes a passkey, for someone signed in, or with an invite link's secret as \`context\` (which signs them in): → its
  /// credential id, its id in the auth service, and its PRF output (asked for again if making it didn't give it).
  async function newPasskey(context) {
    const o = await api("/api/auth/passkey/generate-register-options" + (context ? "?context=" + encodeURIComponent(context) : ""));
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
    const o = await api("/api/auth/passkey/generate-authenticate-options");
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

const page = (title, body, script) => `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)}</title><style>${style}</style></head><body><main>${body}<p class="error"></p></main><script>${webauthn}${script}</script></body></html>`;

/// Which passkeys work, in plain words, for the pages that make or ask for one.
const WORKS = "Passkeys saved in iCloud Keychain, Google Password Manager or a security key work, on a current system.";

/// Signing in, to the install or, sent here by a client's authorization (\`oauth_query\`), to that client: the client
/// is the person, with everything they can use, so the passkey is all it asks for. With Google (\`google\`, the
/// install's domain), it's signing in with Google, each time.
export function signInPage({ name, google }) {
  if (google) return googlePage({ name, google });
  const body = `
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">With your passkey: it's what opens your keys.</p>
    <button class="primary" id="passkey">Sign In with a Passkey</button>
    <p class="note">No passkey here yet? Use one from another device, or ask whoever runs ${escape(name)} for an invite link.</p>`;
  const script = `${acts}
    const oauth_query = location.search.slice(1);
    const client_id = new URLSearchParams(oauth_query).get("client_id");
    const install = ${JSON.stringify(name).replaceAll("<", "\\u003c")};
    if (client_id) (async () => {
      try {
        const about = await api("/sign-in/about?" + new URLSearchParams({ client_id, redirect_uri: new URLSearchParams(oauth_query).get("redirect_uri") || "" }));
        const title = "Sign in to " + about.client + (about.person ? " as " + about.person : "");
        document.title = title;
        document.getElementById("title").textContent = title;
        if (!about.web) acts(about);
      } catch (err) { say(err.message); }
    })();
    document.getElementById("passkey").onclick = async () => {
      say();
      try {
        const { prf, credential } = await withPasskey();
        location.href = client_id ? (await api("/sign-in/accept", { oauth_query, prf, credential })).url : "/";
      } catch (err) { quietly(err); }
    };`;
  return page(`Sign in to ${name}`, body, script);
}

function googlePage({ name, google }) {
  const body = `
    <h1 id="title">Sign in to ${escape(name)}</h1>
    <p id="what">With your Google account at ${escape(google)}.</p>
    <button class="primary" id="google">Sign In with Google</button>`;
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
        const about = await api("/sign-in/about?" + new URLSearchParams({ client_id, redirect_uri: new URLSearchParams(oauth_query).get("redirect_uri") || "" }));
        const title = "Sign in to " + about.client;
        document.title = title;
        document.getElementById("title").textContent = title;
        if (!about.web) acts(about);
        // Back from Google just now: on to the client.
        if (about.fresh && !failed) location.replace((await api("/sign-in/accept", { oauth_query })).url);
      } catch (err) { say(err.message); }
    })();
    document.getElementById("google").onclick = async () => {
      say();
      try {
        const out = await api("/api/auth/sign-in/social", { provider: "google", callbackURL: back, errorCallbackURL: back });
        location.href = out.url;
      } catch (err) { say(err.message); }
    };`;
  return page(`Sign in to ${name}`, body, script);
}

export function invitePage({ name }) {
  const body = `
    <section id="join">
      <h1 id="title">Join ${escape(name)}</h1>
      <p id="what">Make a passkey to join. It's how you sign in, and the only thing that opens your keys.</p>
      <button class="primary" id="passkey" disabled>Make a Passkey</button>
      <p class="note">${escape(WORKS)}</p>
    </section>
    <section id="done" hidden>
      <h1>You're in</h1>
      <p class="callout">Add a second passkey, on another device or a security key, so losing this one doesn't lock you out.</p>
      <a class="button primary" href="/passkeys">Add Another Passkey</a>
      <a class="button" href="/">Open ${escape(name)}</a>
    </section>`;
  const script = `
    const token = location.hash.slice(1);
    (async () => {
      try {
        const about = await api("/invite/about", { token });
        document.getElementById("title").textContent = "Welcome, " + about.person;
        document.getElementById("passkey").disabled = false;
      } catch (err) { say(err.message); }
    })();
    document.getElementById("passkey").onclick = async () => {
      say();
      try {
        // Whoever was signed in here before, the passkey is the invite's person's.
        await api("/api/auth/sign-out", {}).catch(() => {});
        const { prf, credential } = await newPasskey(token);
        await api("/invite/keys", { token, prf, credential });
        history.replaceState(null, "", "/invite");
        document.getElementById("join").hidden = true;
        document.getElementById("done").hidden = false;
      } catch (err) { quietly(err); }
    };`;
  return page(`Join ${name}`, body, script);
}

export function passkeysPage({ name }) {
  const body = `
    <h1>Passkeys</h1>
    <p id="what">Each one signs you in to ${escape(name)} and opens your keys.</p>
    <button class="primary" id="signin" hidden>Sign In with a Passkey</button>
    <section id="mine" hidden>
      <ul class="passkeys" id="list"></ul>
      <p class="callout" id="suggest" hidden>Add a second passkey, on another device or a security key. If you lose every passkey, only the install's recovery key gets you back in, with the help of whoever holds it.</p>
      <button class="primary" id="add">Add a Passkey</button>
      <p class="note">${escape(WORKS)} Adding one asks first for a passkey you have, then makes the new one.</p>
    </section>`;
  const script = `
    const esc = (t) => String(t).replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";");
    const when = (d) => new Date(d).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
    async function show() {
      let list;
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
      document.getElementById("list").innerHTML = list.map((p) => '<li><span>' + esc(p.name || "Passkey") + '<small>Added ' + esc(when(p.createdAt)) + '</small></span>' +
        (list.length > 1 ? '<button data-id="' + esc(p.id) + '">Remove</button>' : '') + '</li>').join("");
    }
    show();
    document.getElementById("signin").onclick = async () => {
      say();
      try { await withPasskey(); await show(); } catch (err) { quietly(err); }
    };
    document.getElementById("add").onclick = async () => {
      say();
      let made;
      try {
        const have = await withPasskey();
        made = await newPasskey();
        await api("/passkeys/add", { from: have.prf, from_credential: have.credential, prf: made.prf, credential: made.credential });
        made = null;
        await show();
      } catch (err) {
        // A new passkey that doesn't open the person's keys isn't kept.
        if (made?.id) await api("/api/auth/passkey/delete-passkey", { id: made.id }).catch(() => {});
        quietly(err);
      }
    };
    document.getElementById("list").onclick = async (e) => {
      const id = e.target.dataset?.id;
      if (!id || !confirm("Remove this passkey? It won't sign you in or open your keys again.")) return;
      try { await api("/api/auth/passkey/delete-passkey", { id }); await show(); } catch (err) { say(err.message); }
    };`;
  return page(`Passkeys`, body, script);
}

/// Signing in with a code another device shows (the CLI on a server): typing it here goes on to signing in with a
/// passkey (or Google); `done` says how that went, once it's back.
export function codePage({ name, google, error = "", done }) {
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
    <button class="primary">Continue</button></form>`;
  const script = `
    say(${JSON.stringify(error).replaceAll("<", "\\u003c")});
    document.getElementById("code").focus();
    document.getElementById("form").onsubmit = (e) => {
      e.preventDefault();
      location.href = "/sign-in/code?c=" + encodeURIComponent(document.getElementById("code").value);
    };`;
  return page(`Sign in to ${name}`, body, script);
}
