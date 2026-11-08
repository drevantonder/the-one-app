// A remote MCP server as an app: each of its tools is one of the app's, forwarded to the server's tools/call over
// streamable HTTP. `apps/mcp/add` makes one from this file, beside server.json (url, title, headers) and tools.json
// (what refresh_tools said the server has), and deploys it again when the tools change.
//
// Servers that want OAuth (MCP's authorization: protected resource metadata, dynamic client registration, PKCE) are
// signed in with sign_in, as many accounts as the user likes. Each account is an item in the caller's Vault: its
// tokens (and its client's secret) as secret fields, sent only to the server and its token address, and how to
// refresh them as plain ones. The store keeps only a sign-in that's waiting, with its verifier.

import server from "./server.json";
import listed from "./tools.json";

const NAME = server.name;
const PROTOCOL = "2025-06-18";
// Where the browser goes after consent. Nothing listens there: the user copies the address it fails to load.
const REDIRECT = `http://127.0.0.1/oauth/${NAME}`;
const MINE = new Set(["sign_in", "sign_out", "accounts", "refresh_tools"]);

const host = (url) => new URL(url).hostname.toLowerCase();
const TYPE = `${server.title || NAME} account`;
// A form body, with the Vault's placeholders left as written for the node to fill (tokens are URL-safe).
const form = (o) => Object.entries(o).filter(([, v]) => v != null && v !== "")
  .map(([k, v]) => `${encodeURIComponent(k)}=${/^\{\{[\w:.-]+\}\}$/.test(v) ? v : encodeURIComponent(v)}`).join("&");
const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const random = () => b64url(crypto.getRandomValues(new Uint8Array(32)));

function toolName(tool) {
  let name = tool.toLowerCase().replace(/[^a-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  const own = NAME.replace(/-/g, "_") + "_";
  if (name.startsWith(own) && name.length > own.length) name = name.slice(own.length);
  return MINE.has(name) ? `${name}_tool` : name;
}

async function accounts(ctx) {
  if (!ctx.user) return [];
  const value = (i, f) => i.fields[f]?.value ?? null;
  return (await ctx.vault.items()).filter((i) => i.type === TYPE && i.fields.access_token).sort((a, b) => a.title.localeCompare(b.title)).map((i) => ({
    id: i.id, name: i.title, client_id: value(i, "client_id"), token_endpoint: value(i, "token_endpoint"), resource: value(i, "resource"), has_secret: !!i.fields.client_secret,
  }));
}

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
/// refreshed once; the new tokens are kept in its item.
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
    return await open(`{{${acct.id}:access_token}}`);
  } catch (e) {
    if (e.unauthorized == null) throw e;
    return { ...(await open(await renew(ctx, acct))), renewed: true };
  }
}

async function renew(ctx, acct) {
  const r = await fetch(acct.token_endpoint, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
    body: form({ grant_type: "refresh_token", refresh_token: `{{${acct.id}:refresh_token}}`, client_id: acct.client_id, client_secret: acct.has_secret ? `{{${acct.id}:client_secret}}` : null, resource: acct.resource }),
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok || !tok.access_token) throw new Error(`Needs signing in again (${acct.name}: ${tok.error_description || tok.error || r.status}): sign_in with account ${acct.name} signs it in again`);
  ctx.vault.update(acct.id, { fields: { access_token: { value: tok.access_token }, ...(tok.refresh_token ? { refresh_token: { value: tok.refresh_token } } : {}) } });
  return tok.access_token;
}

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

function email(idToken) {
  try {
    return JSON.parse(atob(idToken.split(".")[1].replace(/-/g, "+").replace(/_/g, "/"))).email || null;
  } catch {
    return null;
  }
}

const ACCOUNT = { type: "string", description: "which signed-in account, as accounts lists them; needed only when several are" };

const tools = Object.fromEntries(listed.tools.map((tool) => {
  const input = { type: "object", ...(tool.inputSchema || {}) };
  const own = input.properties && "account" in input.properties;
  if (listed.oauth && !own) input.properties = { ...(input.properties || {}), account: ACCOUNT };
  return [toolName(tool.name), {
    description: tool.description || tool.title || tool.name,
    input,
    reads: tool.annotations?.readOnlyHint === true,
    confirm: tool.annotations?.readOnlyHint !== true && tool.annotations?.destructiveHint === true,
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
  icon: "puzzle-piece",
  instructions: [
    `${server.title || NAME} is the remote MCP server at ${server.url}: its tools are this app's.`,
    listed.oauth ? "It needs a sign-in: accounts lists who's signed in, sign_in signs in (another account adds one), and with several each tool takes account." : "",
    listed.instructions || "",
  ].filter(Boolean).join(" "),

  vault: listed.oauth ? {
    account: { title: TYPE, type: TYPE, domain: host(server.url), accounts: true, hosts: [host(server.url)], fields: { access_token: { label: "Access token", secret: true }, refresh_token: { label: "Refresh token", secret: true } } },
  } : {},

  schema: {
    user: `
      create table pending (one integer primary key check (one = 1), state text not null, account text, client_id text not null,
                            token_endpoint text not null, resource text, client_secret text, verifier text not null);`,
  },

  tools: {
    ...tools,

    sign_in: {
      description: "Sign in to the server, or to another account: returns a link to open in a browser. After consent the browser fails to load a 127.0.0.1 page: pass that page's whole address as code. Answers { signed_in: false, why } when that didn't sign in.",
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
        if (!ctx.user) throw new Error("this device doesn't keep your data: sign in on one of your own devices");
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
        const verifier = random(), state = random();
        const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
        // A sign-in waiting for its code, in this app's own data: its verifier, and its client until the account has it.
        ctx.user.exec("insert or replace into pending values (1, ?, ?, ?, ?, ?, ?, ?)", [state, name || null, client_id, as.token_endpoint, as.resource || null, client_secret || null, verifier]);
        const link = `${as.authorization_endpoint}?${form({
          response_type: "code", client_id, redirect_uri: REDIRECT, scope: as.scope, state, resource: as.resource,
          code_challenge: challenge, code_challenge_method: "S256",
        })}`;
        return `Open this link in a browser on any device and sign in. The browser then fails to load a 127.0.0.1 page: copy that page's whole address and pass it to sign_in as code.\n\n${link}`;
      },
    },

    accounts: {
      description: "The accounts signed in to the server, for you.",
      reads: true,
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
        ctx.vault.release(acct.id);
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
          out.push(...(asked.result.tools || []).map((t) => ({ name: t.name, title: t.title, description: t.description, inputSchema: t.inputSchema, annotations: t.annotations })));
          cursor = asked.result.nextCursor;
        } while (cursor && out.length < 1000);
        return { title: init.serverInfo?.title || init.serverInfo?.name || null, instructions: init.instructions || "", oauth: !!acct, tools: out };
      },
    },
  },
};

/// A sign-in that's over without an account (a code is taken once): it answers why rather than failing, so its
/// verifier is forgotten, as a failed call writes nothing.
function failed(ctx, why) {
  ctx.user.exec("delete from pending");
  return { signed_in: false, why };
}

async function finish(ctx, code, name) {
  const p = (await ctx.user.query("select * from pending"))[0];
  if (!p) throw new Error("no sign-in is in progress: call sign_in without code first");
  let state = null, refused = null;
  try {
    const u = new URL(code);
    refused = u.searchParams.get("error_description") || u.searchParams.get("error");
    state = u.searchParams.get("state");
    code = u.searchParams.get("code") || code;
  } catch {} // a bare code, not the address it came in
  if (state && state !== p.state) throw new Error("that address is from an older sign-in: use the newest link sign_in gave");
  if (refused) return failed(ctx, `Sign-in failed: ${refused}. Start again with sign_in.`);
  const r = await fetch(p.token_endpoint, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
    body: form({
      grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: p.client_id, code_verifier: p.verifier,
      client_secret: p.client_secret, resource: p.resource,
    }),
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok || !tok.access_token) return failed(ctx, `Sign-in failed: ${tok.error_description || tok.error || r.status}. Start again with sign_in.`);
  if (!tok.refresh_token) return failed(ctx, "The server gave no refresh token, so the sign-in wouldn't last. Start again with sign_in.");
  const acct = name || p.account || email(tok.id_token || "") || "default";
  ctx.user.exec("delete from pending");
  const fields = {
    name: { value: acct }, access_token: { value: tok.access_token, secret: true }, refresh_token: { value: tok.refresh_token, secret: true },
    client_id: { value: p.client_id }, token_endpoint: { value: p.token_endpoint }, ...(p.resource ? { resource: { value: p.resource } } : {}),
    ...(p.client_secret ? { client_secret: { value: p.client_secret, secret: true } } : {}),
  };
  await ctx.vault.create({ title: acct, type: TYPE, domain: host(server.url), fields, hosts: [...new Set([host(server.url), host(p.token_endpoint)])], same: "name" });
  return `Signed in as ${acct}.${listed.oauth ? "" : ` Run apps/mcp/add ${NAME} to make the server's tools this app's.`}`;
}
