// The auth service's two screens: signing in, and letting a client in with a passkey. Plain HTML with a little script,
// served from the auth service itself, so passkeys belong to its name.

const escape = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

const style = `
  :root { color-scheme: light dark; --accent: #0a84ff; font: 16px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }
  main { width: min(360px, calc(100vw - 48px)); }
  h1 { font-size: 22px; font-weight: 600; margin: 0 0 6px; }
  p { margin: 0 0 20px; opacity: .7; }
  button, input { box-sizing: border-box; width: 100%; font: inherit; padding: 12px 14px; border-radius: 12px; margin: 0 0 10px; }
  input { border: 1px solid color-mix(in srgb, CanvasText 18%, transparent); background: transparent; color: inherit; }
  button { border: 0; background: color-mix(in srgb, CanvasText 8%, transparent); color: inherit; cursor: pointer; }
  button.primary { background: var(--accent); color: white; font-weight: 600; }
  label.scope { display: flex; gap: 10px; align-items: center; padding: 10px 2px; }
  label.scope input { width: auto; margin: 0; }
  details { margin-top: 14px; } summary { cursor: pointer; opacity: .7; margin-bottom: 12px; }
  .error { color: #d70015; min-height: 1.4em; margin-top: 8px; opacity: 1; }
  [hidden] { display: none !important; }
`;

// WebAuthn's binary fields, to and from the base64url its JSON uses; and asking for a passkey with its PRF output.
const webauthn = `
  const b64u = {
    decode: (s) => Uint8Array.from(atob(s.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)),
    encode: (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, ""),
  };
  const PRF_SALT = new TextEncoder().encode("one/passkey-prf");
  const say = (text) => (document.querySelector(".error").textContent = text || "");
  async function api(path, body) {
    const resp = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw new Error(out.error || out.message || "That didn't work.");
    return out;
  }
  function credentialJSON(c) {
    const r = c.response;
    const response = { clientDataJSON: b64u.encode(r.clientDataJSON) };
    if (r.attestationObject) Object.assign(response, { attestationObject: b64u.encode(r.attestationObject), transports: r.getTransports?.() || [] });
    if (r.authenticatorData) Object.assign(response, { authenticatorData: b64u.encode(r.authenticatorData), signature: b64u.encode(r.signature), userHandle: r.userHandle && b64u.encode(r.userHandle) });
    return { id: c.id, rawId: b64u.encode(c.rawId), type: c.type, response, clientExtensionResults: {}, authenticatorAttachment: c.authenticatorAttachment };
  }
  async function newPasskey() {
    const o = await api("/api/auth/passkey/generate-register-options");
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), user: { ...o.user, id: b64u.decode(o.user.id) },
      excludeCredentials: (o.excludeCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })), extensions: { prf: {} } };
    const c = await navigator.credentials.create({ publicKey });
    await api("/api/auth/passkey/verify-registration", { response: credentialJSON(c), name: navigator.platform || "Passkey" });
  }
  /// Signs in with a passkey: → its credential id and PRF output (null if the passkey has none). With oauth_query,
  /// the sign-in carries on with the client's authorization.
  async function withPasskey(oauth_query) {
    const o = await api("/api/auth/passkey/generate-authenticate-options");
    const publicKey = { ...o, challenge: b64u.decode(o.challenge), allowCredentials: (o.allowCredentials || []).map((c) => ({ ...c, id: b64u.decode(c.id) })),
      extensions: { prf: { eval: { first: PRF_SALT } } } };
    const c = await navigator.credentials.get({ publicKey });
    const prf = c.getClientExtensionResults().prf?.results?.first;
    const out = await api("/api/auth/passkey/verify-authentication", { response: credentialJSON(c), ...(oauth_query ? { oauth_query } : {}) });
    return { credential: c.id, prf: prf ? b64u.encode(prf) : null, out };
  }
`;

const page = (title, body, script) => `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)}</title><style>${style}</style></head><body><main>${body}<p class="error"></p></main><script>${webauthn}${script}</script></body></html>`;

export function signInPage({ name, google, first }) {
  const body = `
    <h1>Sign in to ${escape(name)}</h1>
    <p>Your passkey opens your keys; nothing else can.</p>
    <button class="primary" id="passkey">Sign In with a Passkey</button>
    ${google ? `<button id="google">Sign In with Google</button>` : ""}
    <details${first ? " open" : ""}><summary>${first ? "Make the first account" : "Email and password"}</summary>
      <form id="email">
        ${first ? `<input name="name" placeholder="Your name" autocomplete="name" required>` : ""}
        <input name="email" type="email" placeholder="Email" autocomplete="email" required>
        <input name="password" type="password" placeholder="Password" autocomplete="${first ? "new-password" : "current-password"}" minlength="8" required>
        <button type="submit">${first ? "Make Account" : "Sign In"}</button>
      </form>
    </details>`;
  const script = `
    const oauth_query = location.search.slice(1) || undefined;
    const go = (out) => { if (out?.url) location.href = out.url; else if (out?.redirect && out?.url) location.href = out.url; else say("Signed in. Go back to the app you came from."); };
    document.getElementById("passkey").onclick = async () => {
      say();
      try {
        const { prf, credential, out } = await withPasskey(oauth_query);
        if (prf) sessionStorage.setItem("passkey", JSON.stringify({ prf, credential }));
        go(out);
      } catch (err) { say(err.name === "NotAllowedError" ? "" : err.message); }
    };
    document.getElementById("google")?.addEventListener("click", async () => {
      try { go(await api("/api/auth/sign-in/social", { provider: "google", callbackURL: "/sign-in", ...(oauth_query ? { oauth_query } : {}) })); }
      catch (err) { say(err.message); }
    });
    document.getElementById("email").onsubmit = async (e) => {
      e.preventDefault();
      const f = Object.fromEntries(new FormData(e.target));
      try { go(await api(${first ? '"/api/auth/sign-up/email"' : '"/api/auth/sign-in/email"'}, { ...f, ...(oauth_query ? { oauth_query } : {}) })); }
      catch (err) { say(err.message); }
    };`;
  return page(`Sign in to ${name}`, body, script);
}

export function consentPage({ name }) {
  const body = `
    <h1 id="title">Let an app in</h1>
    <p id="what">Use your passkey to give it your keys.</p>
    <button class="primary" id="passkey">Continue with Passkey</button>
    <form id="choose" hidden>
      <div id="scopes"></div>
      <button class="primary" type="submit">Allow</button>
    </form>
    <button id="deny">Don't Allow</button>`;
  const script = `
    const oauth_query = location.search.slice(1);
    const client_id = new URLSearchParams(oauth_query).get("client_id");
    const label = (s) => s === "global" ? "Everyone in ${escape(name)}" : s.startsWith("user:") ? "Your own" : "A group (" + s.slice(6) + ")";
    let opened = null;
    (async () => {
      try {
        const about = await api("/consent/about?client_id=" + encodeURIComponent(client_id));
        document.getElementById("title").textContent = "Let " + about.client + " in?";
        document.getElementById("what").textContent = about.passkeys ? "It can use what you choose, as you, " + about.person + "." : "First, make a passkey: it's what opens your keys.";
        const kept = JSON.parse(sessionStorage.getItem("passkey") || "null");
        sessionStorage.removeItem("passkey");
        if (kept) await open(kept);
      } catch (err) { say(err.message); }
    })();
    async function open(passkey) {
      const { scopes } = await api("/consent/scopes", passkey);
      opened = passkey;
      document.getElementById("scopes").innerHTML = scopes.map((s) => '<label class="scope"><input type="checkbox" checked value="' + s + '"> ' + label(s) + '</label>').join("");
      document.getElementById("passkey").hidden = true;
      document.getElementById("choose").hidden = false;
    }
    document.getElementById("passkey").onclick = async () => {
      say();
      try {
        const about = await api("/consent/about?client_id=" + encodeURIComponent(client_id));
        if (!about.passkeys) await newPasskey();
        const { prf, credential } = await withPasskey();
        if (!prf) return say("This passkey can't open keys: use one saved in iCloud Keychain, Google Password Manager or a security key that supports it.");
        await open({ prf, credential });
      } catch (err) { say(err.name === "NotAllowedError" ? "" : err.message); }
    };
    document.getElementById("choose").onsubmit = async (e) => {
      e.preventDefault();
      const scopes = [...document.querySelectorAll("#scopes input:checked")].map((i) => i.value);
      try { location.href = (await api("/consent/accept", { oauth_query, accept: true, prf: opened.prf, scopes })).url; }
      catch (err) { say(err.message); }
    };
    document.getElementById("deny").onclick = async () => {
      try { location.href = (await api("/consent/accept", { oauth_query, accept: false })).url; } catch (err) { say(err.message); }
    };`;
  return page(`Let an app in`, body, script);
}
