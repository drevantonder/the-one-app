// The vault: the Vault's items (items.js). It holds no one's keys: an item's secret fields were sealed under its
// scope's key, which only people's devices have now, so until items are sealed per scope where devices open them (a
// later piece), every route refuses, and items.js is kept for that piece to build on.

import { DurableObject } from "cloudflare:workers";
import { Items, schema } from "./items.js";
import { bearer, Issuer } from "../shared/token.js";

class Refused extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

const refuse = (status, message) => {
  throw new Refused(status, message);
};

const json = (v, status = 200) => Response.json(v, { status });

export class Vault extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(schema);
    this.items = new Items(this.sql, this);
  }

  async fetch(request) {
    try {
      return await this.route(request, new URL(request.url));
    } catch (err) {
      if (err instanceof Refused) return json({ error: err.message }, err.status);
      console.error(err.stack || err);
      return json({ error: "the vault failed" }, 500);
    }
  }

  async route(request, url) {
    const token = bearer(request) || refuse(401, "bring an access token");
    await this.verify(token);
    switch (`${request.method} ${url.pathname}`) {
      case "GET /items":
      case "POST /items":
      case "POST /fetch":
      case "POST /fill":
        refuse(503, "The Vault's items wait to be sealed where your devices open them.");
    }
    refuse(404, "no such route");
  }

  async verify(token) {
    this.issuer ||= new Issuer(this.env.ISSUER, () => this.env.AUTH.fetch(new Request(`${this.env.ISSUER}/jwks`)));
    return this.issuer.verify(token).catch((err) => refuse(401, `that access token won't do: ${err.message}`));
  }
}

export default {
  fetch(request, env) {
    return env.VAULT.get(env.VAULT.idFromName("vault")).fetch(request);
  },
};
