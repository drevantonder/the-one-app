// A remote MCP server as an app: each of its tools is an operation, forwarded to the server's tools/call over
// streamable HTTP. `apps/mcp/add` makes one from this file, beside server.json (url, title, headers) and tools.json
// (what refresh_tools said the server has), and deploys it again when the tools change.
//
// Servers that want OAuth (MCP's authorization: protected resource metadata, dynamic client registration, PKCE) are
// signed in with sign_in, as many accounts as the person likes. Every token is one of the caller's own secrets, sent
// only to the host it's for; the store keeps only which accounts there are and how to refresh them.

import server from "./server.json";
import listed from "./tools.json";

const NAME = server.name;
const PROTOCOL = "2025-06-18";
// Where the browser goes after consent. Nothing listens there: the person copies the address it fails to load.
const REDIRECT = `http://127.0.0.1/oauth/${NAME}`;
const MINE = new Set(["sign_in", "sign_out", "accounts", "refresh_tools"]);

const host = (url) => new URL(url).hostname.toLowerCase();
const slug = (name) => name.replace(/[^A-Za-z0-9_.-]/g, "_").slice(0, 48);
const access = (name) => `access.${slug(name)}`;
const refresh = (name) => `refresh.${slug(name)}`;
// A form body, with secrets' placeholders left as written for the node to fill (tokens and verifiers are URL-safe).
const form = (o) => Object.entries(o).filter(([, v]) => v != null && v !== "")
  .map(([k, v]) => `${encodeURIComponent(k)}=${/^\{\{[\w.-]+\}\}$/.test(v) ? v : encodeURIComponent(v)}`).join("&");
const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const random = () => b64url(crypto.getRandomValues(new Uint8Array(32)));

/// An op's name for a tool: lowercase words joined by _, without the server's own name in front.
function opName(tool) {
  let name = tool.toLowerCase().replace(/[^a-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  const own = NAME.replace(/-/g, "_") + "_";
  if (name.startsWith(own) && name.length > own.length) name = name.slice(own.length);
  return MINE.has(name) ? `${name}_tool` : name;
}

/// The accounts signed in for the caller.
async function accounts(ctx) {
  if (!ctx.user) return [];
  return ctx.user.query("select * from accounts order by name");
}

/// The account a call acts as: the one named, or the only one.
async function account(ctx, name) {
  const all = await accounts(ctx);
  if (name) {
    const a = all.find((a) => a.name === name);
    if (!a) throw new Error(`no account ${name} is signed in: accounts lists them`);
    return a;
  }
  if (all.length > 1) throw new Error(`several accounts are signed in (${all.map((a) => a.name).join(", ")}): say which as account`);
  return all[0] || null;
}

/// A JSON-RPC message to the server, and its answer (from JSON or an event stream), with the session's headers.
async function rpc(session, method, params, id) {
  const headers = { "content-type": "application/json", accept: "application/json, text/event-stream", ...server.headers };
  if (session.id) headers["mcp-session-id"] = session.id;
  if (session.started) headers["mcp-protocol-version"] = session.protocol;
  if (session.token) headers.authorization = `Bearer ${session.token}`;
  const r = await fetch(server.url, { method: "POST", headers, body: JSON.stringify({ jsonrpc: "2.0", method, params, ...(id == null ? {} : { id }) }) });
  if (r.status === 401) {
    const e = new Error(`${server.url} wants a sign-in: sign_in signs in`);
    e.unauthorized = r.headers.get("www-authenticate") || "";
    throw e;
  }
  if (!r.ok) throw new Error(`${method}: ${r.status} ${(await r.text()).slice(0, 500)}`);
  session.id = r.headers.get("mcp-session-id") || session.id;
  if (id == null) return null;
  const text = await r.text();
  const messages = (r.headers.get("content-type") || "").includes("text/event-stream")
    ? text.split(/\r?\n/).filter((l) => l.startsWith("data:")).map((l) => { try { return JSON.parse(l.slice(5)); } catch { return null; } })
    : [JSON.parse(text)];
  const answer = messages.find((m) => m && m.id === id);
  if (!answer) throw new Error(`${method}: the server didn't answer`);
  if (answer.error) throw new Error(`${method}: ${answer.error.message || JSON.stringify(answer.error)}`);
  return answer.result;
}

/// Opens a session as the account (null: none), asks `method`, and gives its result. A rejected access token is
/// refreshed once; the new tokens are kept as the caller's secrets.
async function ask(ctx, acct, method, params) {
  const open = async (token) => {
    const session = { protocol: PROTOCOL, token };
    const init = await rpc(session, "initialize", { protocolVersion: PROTOCOL, capabilities: {}, clientInfo: { name: NAME, version: "1" } }, 0);
    session.protocol = init.protocolVersion || PROTOCOL;
    session.started = true;
    await rpc(session, "notifications/initialized", {});
    return { init, result: await rpc(session, method, params, 1) };
  };
  if (!acct) return open(null);
  try {
    return await open(`{{${access(acct.name)}}}`);
  } catch (e) {
    if (e.unauthorized == null) throw e;
    return { ...(await open(await renew(ctx, acct))), renewed: true };
  }
}

/// A new access token for the account, from its refresh token.
async function renew(ctx, acct) {
  const r = await fetch(acct.token_endpoint, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
    body: form({ grant_type: "refresh_token", refresh_token: `{{${refresh(acct.name)}}}`, client_id: acct.client_id, client_secret: acct.has_secret ? `{{client_secret.${slug(acct.client_id)}}}` : null, resource: acct.resource }),
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok || !tok.access_token) throw new Error(`${acct.name}'s sign-in no longer works (${tok.error_description || tok.error || r.status}): sign_in with account ${acct.name} signs it in again`);
  ctx.user.secret(access(acct.name), tok.access_token, [host(server.url)]);
  if (tok.refresh_token) ctx.user.secret(refresh(acct.name), tok.refresh_token, [host(acct.token_endpoint)]);
  return tok.access_token;
}

/// An MCP tool result as an op's result: its text, else its structured content.
function result(res) {
  const parts = (res.content || []).map((c) =>
    c.type === "text" ? c.text
      : c.type === "resource" ? c.resource?.text ?? `[${c.resource?.uri}]`
        : c.type === "resource_link" ? `[${c.name || c.uri}](${c.uri})`
          : `[${c.type}${c.mimeType ? ` ${c.mimeType}` : ""}]`);
  const text = parts.join("\n");
  if (res.isError) throw new Error(text || "the tool failed");
  return text || res.structuredContent || "Done.";
}

/// Fetches JSON, or null if it isn't there.
async function json(url) {
  try {
    const r = await fetch(url, { headers: { accept: "application/json" } });
    return r.ok ? await r.json() : null;
  } catch {
    return null;
  }
}

/// Where to sign in to the server: its protected resource's metadata, then its authorization server's.
async function discover(unauthorized) {
  const mcp = new URL(server.url);
  const named = unauthorized.match(/resource_metadata="([^"]+)"/)?.[1];
  const resource = (named && (await json(named)))
    || (await json(`${mcp.origin}/.well-known/oauth-protected-resource${mcp.pathname.replace(/\/$/, "")}`))
    || (await json(`${mcp.origin}/.well-known/oauth-protected-resource`))
    || { authorization_servers: [mcp.origin] };
  const issuer = new URL(resource.authorization_servers?.[0] || mcp.origin);
  const path = issuer.pathname.replace(/\/$/, "");
  const meta = (await json(`${issuer.origin}/.well-known/oauth-authorization-server${path}`))
    || (await json(`${issuer.origin}/.well-known/openid-configuration${path}`))
    || (await json(`${issuer.origin}${path}/.well-known/openid-configuration`))
    || { authorization_endpoint: `${issuer.origin}/authorize`, token_endpoint: `${issuer.origin}/token`, registration_endpoint: `${issuer.origin}/register` };
  const scope = unauthorized.match(/scope="([^"]+)"/)?.[1] || (resource.scopes_supported || []).join(" ");
  return { issuer: issuer.href, resource: resource.resource || server.url, scope, ...meta };
}

/// The email address an OpenID Connect ID token names, if any.
function email(idToken) {
  try {
    return JSON.parse(atob(idToken.split(".")[1].replace(/-/g, "+").replace(/_/g, "/"))).email || null;
  } catch {
    return null;
  }
}

const ACCOUNT = { type: "string", description: "which signed-in account, as accounts lists them; needed only when several are" };

/// Every tool the server listed as an operation.
const tools = Object.fromEntries(listed.tools.map((tool) => {
  const input = { type: "object", ...(tool.inputSchema || {}) };
  const own = input.properties && "account" in input.properties;
  if (listed.oauth && !own) input.properties = { ...(input.properties || {}), account: ACCOUNT };
  return [opName(tool.name), {
    description: tool.description || tool.title || tool.name,
    input,
    async run(ctx, args) {
      args = { ...args };
      const named = own ? null : args.account;
      if (!own) delete args.account;
      const acct = await account(ctx, named);
      const { result: res, renewed } = await ask(ctx, acct, "tools/call", { name: tool.name, arguments: args });
      // A call that throws writes nothing, and a refresh token may have just been replaced: keep the new one.
      if (renewed && res.isError) return `Error: ${(res.content || []).map((c) => c.text || "").join("\n")}`;
      return result(res);
    },
  }];
}));

export default {
  name: NAME,
  title: server.title || listed.title || NAME,
  icon: "puzzlepiece.extension",
  instructions: [
    `${server.title || NAME} is the remote MCP server at ${server.url}: its tools are this app's operations.`,
    listed.oauth ? "It needs a sign-in: accounts lists who's signed in, sign_in signs in (another account adds one), and with several each operation takes account." : "",
    listed.instructions || "",
  ].filter(Boolean).join(" "),

  schema: {
    user: `
      create table accounts (name text primary key, client_id text not null, token_endpoint text not null, resource text, has_secret integer not null);
      create table pending (one integer primary key check (one = 1), state text not null, account text, client_id text not null,
                            token_endpoint text not null, resource text, has_secret integer not null);`,
  },

  on: {
    account(db, e) {
      const d = e.data;
      db.exec("insert or replace into accounts values (?, ?, ?, ?, ?)", [d.name, d.client_id, d.token_endpoint, d.resource || null, d.has_secret ? 1 : 0]);
      db.exec("delete from pending");
    },
    sign_out(db, e) {
      db.exec("delete from accounts where name = ?", [e.data.name]);
    },
    pending(db, e) {
      const d = e.data;
      db.exec("insert or replace into pending values (1, ?, ?, ?, ?, ?, ?)", [d.state, d.account || null, d.client_id, d.token_endpoint, d.resource || null, d.has_secret ? 1 : 0]);
    },
  },

  ops: {
    ...tools,

    sign_in: {
      description: "Sign in to the server, or to another account: returns a link to open in a browser. After consent the browser fails to load a 127.0.0.1 page: pass that page's whole address as code.",
      input: {
        type: "object",
        properties: {
          code: { type: "string", description: "the address the browser ended up on after consent" },
          account: { type: "string", description: "a name for the account; by default its email address if the server says it, else default" },
          client_id: { type: "string", description: "an OAuth client of your own, for servers that don't register one themselves" },
          client_secret: { type: "string", description: "that client's secret, if it has one" },
        },
      },
      async run(ctx, { code, account: name, client_id, client_secret }) {
        if (!ctx.user) throw new Error("sign in from your own scope");
        if (code) return finish(ctx, code, name);
        let unauthorized;
        try {
          await rpc({ protocol: PROTOCOL }, "initialize", { protocolVersion: PROTOCOL, capabilities: {}, clientInfo: { name: NAME, version: "1" } }, 0);
          return `${server.url} needs no sign-in.`;
        } catch (e) {
          if (e.unauthorized == null) throw e;
          unauthorized = e.unauthorized;
        }
        const as = await discover(unauthorized);
        if (!client_id) {
          if (!as.registration_endpoint) throw new Error(`${as.issuer} doesn't register clients itself: make an OAuth client there, with redirect URI ${REDIRECT}, and pass its client_id (and client_secret)`);
          const r = await fetch(as.registration_endpoint, {
            method: "POST",
            headers: { "content-type": "application/json", accept: "application/json" },
            body: JSON.stringify({
              client_name: `${server.title || NAME} app`, redirect_uris: [REDIRECT], grant_types: ["authorization_code", "refresh_token"],
              response_types: ["code"], token_endpoint_auth_method: "none", scope: as.scope || undefined,
            }),
          });
          const reg = await r.json().catch(() => ({}));
          if (!r.ok || !reg.client_id) throw new Error(`${as.issuer} wouldn't register a client: ${reg.error_description || reg.error || r.status}`);
          client_id = reg.client_id;
          client_secret = reg.client_secret;
        }
        const tokenHost = host(as.token_endpoint);
        if (client_secret) ctx.user.secret(`client_secret.${slug(client_id)}`, client_secret, [tokenHost]);
        const verifier = random(), state = random();
        const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
        ctx.user.secret("verifier", verifier, [tokenHost]);
        ctx.user.append("pending", { state, account: name || null, client_id, token_endpoint: as.token_endpoint, resource: as.resource, has_secret: !!client_secret });
        const link = `${as.authorization_endpoint}?${form({
          response_type: "code", client_id, redirect_uri: REDIRECT, scope: as.scope, state, resource: as.resource,
          code_challenge: challenge, code_challenge_method: "S256",
        })}`;
        return `Open this link in a browser on any device and sign in. The browser then fails to load a 127.0.0.1 page: copy that page's whole address and pass it to sign_in as code.\n\n${link}`;
      },
    },

    accounts: {
      description: "The accounts signed in to the server, for you.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const all = await accounts(ctx);
        return all.length ? all.map((a) => a.name).join("\n") : "No account is signed in: sign_in signs one in, if the server wants one.";
      },
    },

    sign_out: {
      description: "Sign an account out, on every device.",
      input: { type: "object", properties: { account: ACCOUNT } },
      async run(ctx, { account: name }) {
        const acct = await account(ctx, name);
        if (!acct) throw new Error("no account is signed in");
        ctx.user.append("sign_out", { name: acct.name });
        ctx.user.secret(access(acct.name), null, []);
        ctx.user.secret(refresh(acct.name), null, []);
        return `Signed out ${acct.name}.`;
      },
    },

    refresh_tools: {
      description: "The server's tools as it lists them now, as `apps/mcp/add` keeps them; it deploys this app again when they changed.",
      input: { type: "object", properties: { account: ACCOUNT } },
      async run(ctx, { account: name }) {
        const acct = await account(ctx, name);
        const out = [];
        let init, cursor;
        do {
          const asked = await ask(ctx, acct, "tools/list", cursor ? { cursor } : {});
          init = asked.init;
          out.push(...(asked.result.tools || []).map((t) => ({ name: t.name, title: t.title, description: t.description, inputSchema: t.inputSchema })));
          cursor = asked.result.nextCursor;
        } while (cursor && out.length < 1000);
        return { title: init.serverInfo?.title || init.serverInfo?.name || null, instructions: init.instructions || "", oauth: !!acct, tools: out };
      },
    },
  },
};

/// The second half of sign_in: trades the code the browser came back with for tokens.
async function finish(ctx, code, name) {
  const p = (await ctx.user.query("select * from pending"))[0];
  if (!p) throw new Error("no sign-in is in progress: call sign_in without code first");
  let state = null;
  try {
    const u = new URL(code);
    if (u.searchParams.get("error")) throw new Error(`sign-in failed: ${u.searchParams.get("error_description") || u.searchParams.get("error")}`);
    state = u.searchParams.get("state");
    code = u.searchParams.get("code") || code;
  } catch (e) {
    if (e.message.startsWith("sign-in failed")) throw e;
  }
  if (state && state !== p.state) throw new Error("that address is from an older sign-in: use the newest link sign_in gave");
  const r = await fetch(p.token_endpoint, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
    body: form({
      grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: p.client_id, code_verifier: "{{verifier}}",
      client_secret: p.has_secret ? `{{client_secret.${slug(p.client_id)}}}` : null, resource: p.resource,
    }),
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok || !tok.access_token) throw new Error(`sign-in failed: ${tok.error_description || tok.error || r.status}`);
  if (!tok.refresh_token) throw new Error("the server gave no refresh token, so the sign-in wouldn't last");
  const acct = name || p.account || email(tok.id_token || "") || "default";
  ctx.user.secret(access(acct), tok.access_token, [host(server.url)]);
  ctx.user.secret(refresh(acct), tok.refresh_token, [host(p.token_endpoint)]);
  ctx.user.secret("verifier", null, []);
  ctx.user.append("account", { name: acct, client_id: p.client_id, token_endpoint: p.token_endpoint, resource: p.resource, has_secret: p.has_secret });
  return `Signed in as ${acct}.${listed.oauth ? "" : ` Run apps/mcp/add ${NAME} to make the server's tools this app's operations.`}`;
}
