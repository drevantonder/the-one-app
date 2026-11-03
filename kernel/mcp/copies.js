// A remote MCP client's device secret, kept only sealed under that client's own tokens (shared/crypto.js `sealUnder`),
// each copy by its token's id, so the MCP Worker opens it only while the client calls, and with nothing of its own.
//
// A sign-in seals it under the client's refresh token and its access token, until each expires. Refresh tokens are
// single-use (auth takes none twice): a refresh opens the copy under the old one, seals it under the new pair, and
// deletes the old copy, so the old token opens nothing; nor does a copy whose token has expired. An access token's copy
// opens it for each MCP request until that token expires, even if its sign-in is revoked sooner: tokens are checked
// offline, by auth's signature.

import { idOf, openUnder, sealUnder } from "../shared/crypto.js";

/// How long a refresh token's copy is kept unused, as long as the auth service's refresh tokens last.
export const REFRESH_LASTS = 30 * 86_400_000;

export class Copies {
  /// `store`: `get`, `put`, `delete` and `list({ prefix })`, as a Durable Object's storage has them.
  constructor(store, now = () => Date.now()) {
    this.store = store;
    this.now = now;
  }

  async name(token) {
    return `copy:${await idOf(token)}`;
  }

  /// Seals `secret` under `token`, opening until `until` (ms).
  async keep(secret, token, until) {
    await this.store.put(await this.name(token), { sealed: await sealUnder(token, secret), until });
  }

  /// The secret, if `token` opens a copy that's still kept: or null.
  async open(token) {
    const name = await this.name(token);
    const copy = await this.store.get(name);
    if (!copy) return null;
    if (copy.until <= this.now()) {
      await this.store.delete(name);
      return null;
    }
    try {
      return await openUnder(token, copy.sealed);
    } catch {
      return null;
    }
  }

  /// A new sign-in's secret, under its tokens alone: every other copy goes.
  async start(secret, refresh, access, accessUntil) {
    const old = await this.store.list({ prefix: "copy:" });
    await this.store.delete([...old.keys()]);
    await this.keep(secret, refresh, this.now() + REFRESH_LASTS);
    await this.keep(secret, access, accessUntil);
  }

  /// A refresh from `old` to `refresh` and `access`: the secret, sealed under them, the old copy gone. → the secret,
  /// or null if `old` opens nothing.
  async rotate(old, refresh, access, accessUntil) {
    const secret = await this.open(old);
    if (!secret) return null;
    await this.keep(secret, refresh, this.now() + REFRESH_LASTS);
    await this.keep(secret, access, accessUntil);
    await this.store.delete(await this.name(old));
    await this.sweep();
    return secret;
  }

  /// Forgets the copies whose time is over.
  async sweep() {
    const now = this.now();
    const over = [...(await this.store.list({ prefix: "copy:" })).entries()].filter(([, c]) => c.until <= now).map(([k]) => k);
    if (over.length) await this.store.delete(over);
  }
}
