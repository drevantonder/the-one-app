// What the Google apps (gmail, docs, drive, sheets, google-calendar) share: signing in, the accounts and their tokens,
// and the four tools every one has. Each app's `google/` is a link to this folder, so a deploy of it holds these files
// (the same go in its bundle's `src/`), and `import { google } from "./google/google.js"` is all it takes.
//
// The Vault keeps what signing in needs, the same for every Google app: one "Google sign-in" item, the OAuth client
// (its id, and its secret), and a "Google account" item for each account (its address, its refresh token and the
// scopes it allowed), sent only to Google's token address. Signing in to an account another Google app already uses
// adds this app's scopes to it (incremental consent). Google sends the browser back to the network's address, which
// opens this app's settings screen (./settings.jsx) to finish; the verifier waits for it in this app's own data.

const OAUTH = "oauth2.googleapis.com";

const WAITS = 30 * 60_000;

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

// Where Google sends the browser back: the network's address, which opens this app's settings screen to finish. The
// Google sign-in (a Web application client) lists it as an authorized redirect URI.
function redirect(ctx) {
  if (!ctx.address) throw new Error("This device is out of date: update it, then sign in again");
  return `${ctx.address}/back`;
}

const vault = {
  client: {
    title: "Google sign-in", type: "Google sign-in", domain: "google.com", hosts: [OAUTH],
    why: "Make one in Google Cloud Console: APIs & Services › Credentials › Create OAuth client ID, type Web application. It's set once, for everyone's Google apps. Its authorized redirect URI: {address}/back",
    // Set once, by an admin in Everyone's Vault, for every app that declares a shared Google sign-in: anyone else is
    // told which admin to ask.
    shared: true,
    fields: { client_id: { label: "Client ID" }, client_secret: { label: "Client secret", secret: true } },
  },
  account: {
    title: "Google account", type: "Google account", domain: "google.com", hosts: [OAUTH], accounts: true,
    fields: { email: { label: "Email" }, refresh_token: { label: "Refresh token", secret: true } },
  },
};

// Each database of an app that has signed in, as the changes that make it, oldest first: the first is kept because
// every database that ran it has it in its history.
const schema = { user: [
  "create table pending (verifier text not null, client text);",
  "drop table pending; create table pending (state text primary key, verifier text not null, client text, at integer not null);",
] };

async function googleClient(ctx, id) {
  const all = (await ctx.vault.items()).filter((i) => i.fields.client_id?.value && i.fields.client_secret);
  return all.find((i) => i.fields.client_id.value === id) || all.find((i) => i.scope.startsWith("user:")) || all[0] || null;
}

async function accountItems(ctx) {
  return (await ctx.vault.items()).filter((i) => i.type === "Google account" && i.fields.email?.value && i.fields.refresh_token).sort((a, b) => a.time - b.time);
}

async function accounts(ctx) {
  if (!ctx.user) throw new Error("this device doesn't keep your data, so it can't reach your accounts: use one of your own devices");
  return (await accountItems(ctx)).map((i) => i.fields.email.value);
}

function emailOf(idToken) {
  try {
    const payload = String(idToken).split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
    return JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(payload), (c) => c.charCodeAt(0)))).email || null;
  } catch {
    return null;
  }
}

const tokens = new Map(); // `${user}/${email}` → { value, expires }, for this runtime only

/// An app's Google sign-in: `scopes` what it asks for (space separated), `app` and `title` as it's named, `base` the
/// address of the Google API it calls (ending in a slash). Gives what its `export default` takes (`vault`, `schema`,
/// `tools`), `account(ctx, name)` (the address a call's account names, the first signed in when none) and
/// `api(ctx, email, method, path, options)` (a call to Google's API), with the `str`, `accountInput` and `withAccount`
/// its tools' inputs are made of.
export function google({ app, title, scopes, base }) {
  const noClient = () => Object.assign(new Error(`No Google sign-in is set up yet: vault_request (app ${app}, need client) asks for it. An admin sets it once, for everyone; for anyone else it says which admin to ask, which is all to tell them`), { code: "no_client" });

  async function account(ctx, name) {
    const all = await accounts(ctx);
    if (!all.length) throw Object.assign(new Error(`Sign in to a Google account first: ${app}_sign_in signs one in`), { code: "not_signed_in" });
    if (!name) return all[0];
    const n = String(name).toLowerCase();
    const found = all.find((e) => e.toLowerCase() === n) || (all.filter((e) => e.toLowerCase().includes(n)).length === 1 && all.find((e) => e.toLowerCase().includes(n)));
    if (!found) throw new Error(`no single account matches "${name}"; they are: ${all.join(", ")}`);
    return found;
  }

  const allowed = (item) => scopes.split(" ").every((s) => s === "openid" || s === "email" || item.fields.scopes?.value.split(" ").includes(s));

  const forget = (ctx, email) => tokens.delete(`${ctx.user.id}/${email}`);

  async function token(ctx, email) {
    const item = (await accountItems(ctx)).find((i) => i.fields.email.value === email);
    if (!item) throw Object.assign(new Error(`Sign in to ${email} first: ${app}_sign_in signs it in`), { code: "not_signed_in" });
    if (!allowed(item)) throw new Error(`Needs signing in again, to allow ${title}: ${app}_sign_in with account ${email} signs it in`);
    const key = `${ctx.user.id}/${email}`;
    const t = tokens.get(key);
    if (t && t.expires > Date.now() + 60_000) return t.value;
    const client = await googleClient(ctx, item.fields.client_id?.value);
    if (!client) throw noClient();
    const r = await fetch(`https://${OAUTH}/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: `grant_type=refresh_token&client_id=${encodeURIComponent(client.fields.client_id.value)}&client_secret={{${client.id}:client_secret}}&refresh_token={{${item.id}:refresh_token}}`,
    });
    const body = await r.json().catch(() => ({}));
    if (!r.ok || !body.access_token) throw new Error(`Needs signing in again (${email}: ${body.error_description || body.error || r.status}): ${app}_sign_in signs it in again`);
    tokens.set(key, { value: body.access_token, expires: Date.now() + (body.expires_in || 3600) * 1000 });
    return body.access_token;
  }

  /// A link to sign in to Google with, `hint` the account to suggest. Each sign-in waits in this app's own data as a row
  /// of its own, keyed by a random state Google passes back, so two at once (two accounts, devices or agents) each
  /// finish with their own verifier.
  async function startSignIn(ctx, hint) {
    const client = await googleClient(ctx);
    if (!client) throw noClient();
    const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
    const state = b64url(crypto.getRandomValues(new Uint8Array(16)));
    const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
    ctx.user.exec("delete from pending where at < ?", [Date.now() - WAITS]);
    ctx.user.exec("insert into pending values (?, ?, ?, ?)", [state, verifier, client.id ?? null, Date.now()]);
    const q = new URLSearchParams({
      client_id: client.fields.client_id.value, redirect_uri: redirect(ctx), response_type: "code", scope: scopes, access_type: "offline",
      // `<app>/<screen>/<state>`: the network's address opens this app's settings screen, which finishes this sign-in.
      prompt: "consent", include_granted_scopes: "true", state: `${app}/settings/${state}`, code_challenge: challenge, code_challenge_method: "S256", ...(hint ? { login_hint: hint } : {}),
    });
    return `https://accounts.google.com/o/oauth2/v2/auth?${q}`;
  }

  /// A sign-in that's over without an account: it answers why rather than failing, so its verifier is forgotten, as a
  /// failed call writes nothing.
  function failed(ctx, state, why) {
    ctx.user.exec("delete from pending where state = ?", [state]);
    return { signed_in: false, why };
  }

  async function finishSignIn(ctx, given) {
    const s = String(given || "").trim();
    const params = s.includes("=") ? new URL(s.startsWith("http") ? s : `http://x/?${s.split("?").pop()}`).searchParams : null;
    const state = params?.get("state")?.split("/")[2];
    const waiting = await ctx.user.query("select state, verifier, client from pending where at >= ?", [Date.now() - WAITS]);
    const p = state ? waiting.find((w) => w.state === state) : waiting.length === 1 ? waiting[0] : null;
    if (!p) {
      if (!state && waiting.length > 1) throw new Error("More than one sign-in is waiting: give the whole address the browser came back to, not just its code.");
      throw Object.assign(new Error(`Start signing in again: no sign-in is waiting for that address; ${app}_sign_in starts one`), { code: "not_waiting" });
    }
    const refused = params?.get("error");
    if (refused) return failed(ctx, p.state, `Google didn't sign you in (${refused}). Try again: ${app}_sign_in starts again.`);
    const code = params ? params.get("code") : s;
    if (!code) throw new Error("That address has no code. Copy the whole address the browser ended up on.");
    const client = (await ctx.vault.items()).find((i) => i.id === p.client && i.fields.client_secret) || await googleClient(ctx);
    if (!client) throw noClient();
    const r = await fetch(`https://${OAUTH}/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: `grant_type=authorization_code&code=${encodeURIComponent(code)}&client_id=${encodeURIComponent(client.fields.client_id.value)}&client_secret={{${client.id}:client_secret}}&redirect_uri=${encodeURIComponent(redirect(ctx))}&code_verifier=${encodeURIComponent(p.verifier)}`,
    });
    const tok = await r.json().catch(() => ({}));
    if (!r.ok) return failed(ctx, p.state, `Google didn't sign you in (${tok.error_description || tok.error || r.status}). Try again: ${app}_sign_in starts again.`);
    if (!tok.refresh_token) return failed(ctx, p.state, "Google didn't sign you in. Try again: it gave no refresh_token, so the sign-in wouldn't last.");
    const email = emailOf(tok.id_token);
    if (!email) return failed(ctx, p.state, "Google didn't say which account signed in. Try again.");
    ctx.user.exec("delete from pending where state = ?", [p.state]);
    await ctx.vault.create({
      title: email, type: "Google account", domain: "google.com", hosts: [OAUTH], same: "email",
      fields: { email: { value: email }, refresh_token: { value: tok.refresh_token, secret: true }, scopes: { value: tok.scope || scopes }, client_id: { value: client.fields.client_id.value } },
    });
    tokens.set(`${ctx.user.id}/${email}`, { value: tok.access_token, expires: Date.now() + (tok.expires_in || 3600) * 1000 });
    return { signed_in: true, account: email };
  }

  const tools = {
    accounts: {
      reads: true,
      accounts: true,
      description: `The Google accounts ${title} can use, the default first: [{ account, ok }], with why when ok is false (${app}_sign_in with that account fixes it). Every other tool's account takes one of these.`,
      input: { type: "object", properties: {} },
      async run(ctx) {
        return Promise.all((await accounts(ctx)).map(async (account) => {
          try { await token(ctx, account); return { account, ok: true }; } catch (e) { return { account, ok: false, why: e.message }; }
        }));
      },
    },
    sign_in: {
      reads: false,
      description: `Start signing a Google account in to ${title}, or allowing ${title} on one already signed in to another Google app: gives { url, then }. The person opens url and signs in, and Google sends them back to finish.`,
      input: { type: "object", properties: {
        account: { type: "string", description: "the account's address, like someone@gmail.com, to suggest it to Google; for one that needs signing in again, as accounts lists it" },
      } },
      async run(ctx, { account }) {
        if (!ctx.user) throw new Error("this device doesn't keep your data: use one of your own devices");
        return {
          url: await startSignIn(ctx, account),
          then: `The person opens url and signs in, and Google sends them back to finish; ${app}_accounts then lists the account. If it doesn't, give the address their browser came back to to ${app}_finish_sign_in.`,
        };
      },
    },
    finish_sign_in: {
      reads: false,
      description: `Finish signing in with the address the browser came back to after ${app}_sign_in's url, when it didn't finish by itself. Gives { signed_in: true, account }, or { signed_in: false, why } when Google didn't sign in.`,
      input: { type: "object", required: ["code"], properties: { code: { type: "string", description: "the whole address the browser came back to, like https://…/back?state=…&code=…, or just its code" } } },
      run: (ctx, { code }) => finishSignIn(ctx, code),
    },
    sign_out: {
      reads: false,
      description: `Sign a Google account out of ${title}; it stays signed in to the other Google apps that use it, and ${app}_sign_in signs it in again.`,
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: `the account's address, or a part of it only it has, as ${app}_accounts lists them` } } },
      async run(ctx, { account: name }) {
        const email = await account(ctx, name);
        const item = (await accountItems(ctx)).find((i) => i.fields.email.value === email);
        ctx.vault.release(item.id);
        forget(ctx, email);
        return `Signed ${email} out of ${title}.`;
      },
    },
  };

  /// Calls Google's API as `email`: a path under `base`, or a whole URL. `query` values may be arrays; `json` or `raw`
  /// (of content `type`) is the body. `missing` is what to say when what the path names isn't there, or the account
  /// can't open it; `hint` what to add to what Google says about an input it can't take.
  async function api(ctx, email, method, path, { query, json, raw, type, missing, hint } = {}) {
    const q = new URLSearchParams();
    for (const [k, v] of Object.entries(query || {})) {
      for (const one of Array.isArray(v) ? v : [v]) if (one !== undefined && one !== null && one !== "") q.append(k, String(one));
    }
    const r = await fetch((path.startsWith("http") ? path : base + path) + ([...q].length ? `?${q}` : ""), {
      method,
      headers: { authorization: `Bearer ${await token(ctx, email)}`, ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}) },
      body: json ? JSON.stringify(json) : raw,
    });
    const text = await r.text();
    if (r.status === 401) forget(ctx, email);
    if (!r.ok) {
      let message = text.slice(0, 300) || r.statusText;
      try { message = JSON.parse(text).error?.message || message; } catch {}
      // Gmail answers a made-up id with 400 "Invalid id value"; Calendar answers a deleted event with 410.
      if (missing && (r.status === 404 || r.status === 410 || /invalid id value/i.test(message))) throw new Error(missing);
      if (missing && r.status === 403) throw new Error(`${email} can't open it (${message.replace(/\.$/, "")}): use the account it's shared with (${app}_accounts lists them), or ask its owner to share it`);
      if (hint && r.status === 400) throw new Error(`${title}: ${message.replace(/\.$/, "")}: ${hint}`);
      throw new Error(`${title}: ${message}`);
    }
    return text ? JSON.parse(text) : null;
  }

  const str = (description) => ({ type: "string", description });
  const accountInput = str(`which account: its address or a part of it only it has, as ${app}_accounts lists them; default the first`);
  const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });

  return { vault, schema, tools, account, api, str, withAccount };
}
