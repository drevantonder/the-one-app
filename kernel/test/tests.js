// The kernel's own tests, run by `workerd test` (tests/kernel.rs runs them with cargo test).
import { appKey, b64u, databaseKey, idOf, keyPair, openSealed, openUnder, random, sealTo, sealUnder } from "../shared/crypto.js";
import { presign } from "../shared/s3.js";
import { verify } from "../shared/token.js";

function equal(a, b, what) {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${what}: ${JSON.stringify(a)} isn't ${JSON.stringify(b)}`);
}

async function fails(f, what) {
  try {
    await f();
  } catch {
    return;
  }
  throw new Error(`${what} didn't fail`);
}

export default {
  async test() {
    // AWS's own example of a presigned URL (Signature Version 4, "Authenticating Requests: Using Query Parameters").
    const url = await presign({
      method: "GET", host: "examplebucket.s3.amazonaws.com", path: "/test.txt", region: "us-east-1",
      keyId: "AKIAIOSFODNN7EXAMPLE", secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", expires: 86400,
      date: new Date("2013-05-24T00:00:00Z"),
    });
    equal(new URL(url).searchParams.get("X-Amz-Signature"), "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404", "AWS's example signature");
    const create = new URL(await presign({ method: "PUT", host: "h", path: "/b/k", region: "auto", keyId: "k", secret: "s", expires: 60, headers: { "If-None-Match": "*" } }));
    equal(create.searchParams.get("X-Amz-SignedHeaders"), "host;if-none-match", "a create URL signs If-None-Match in");

    // Keys derive one way: an app's key from its scope's, a database's from its app's, each different.
    const scope = random(32);
    const app = await appKey(scope, "board");
    equal((await appKey(scope, "board")).join(), app.join(), "deriving again");
    if ((await appKey(scope, "boards")).join() === app.join()) throw new Error("two apps share a key");
    if ((await databaseKey(app, "main")).join() === app.join()) throw new Error("a database has its app's key");

    // Sealing to a key pair opens only with it.
    const me = await keyPair(), them = await keyPair();
    const sealed = await sealTo(me.public, scope);
    equal((await openSealed(me, sealed)).join(), scope.join(), "opening what was sealed to me");
    await fails(() => openSealed(them, sealed), "opening what was sealed to someone else");

    // Sealing under a secret opens only with it, and its id names it without opening it.
    const under = await sealUnder("a refresh token, long enough", scope);
    equal((await openUnder("a refresh token, long enough", under)).join(), scope.join(), "opening under the secret");
    await fails(() => openUnder("another refresh token, long", under), "opening under another secret");
    if ((await idOf("one secret here")) === (await idOf("another secret"))) throw new Error("two secrets share an id");

    // A token checks only with the key that signed it, from its issuer, before it expires.
    const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid: "k1" };
    const sign = async (claims, kid = "k1") => {
      const enc = (v) => b64u.encode(new TextEncoder().encode(JSON.stringify(v)));
      const signing = `${enc({ alg: "EdDSA", kid })}.${enc(claims)}`;
      return `${signing}.${b64u.encode(await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, new TextEncoder().encode(signing)))}`;
    };
    const exp = Math.floor(Date.now() / 1000) + 60;
    equal((await verify(await sign({ iss: "https://a", sub: "u", exp }), { keys: [jwk] }, "https://a")).sub, "u", "a good token");
    await fails(async () => verify(await sign({ iss: "https://b", sub: "u", exp }), { keys: [jwk] }, "https://a"), "another issuer's token");
    await fails(async () => verify(await sign({ iss: "https://a", sub: "u", exp: exp - 120 }), { keys: [jwk] }, "https://a"), "an expired token");
    await fails(async () => verify(await sign({ iss: "https://a", sub: "u", exp }, "k2"), { keys: [jwk] }, "https://a"), "a token from an unknown key");
    const token = await sign({ iss: "https://a", sub: "u", exp });
    const forged = token.slice(0, token.lastIndexOf(".") + 1) + b64u.encode(new Uint8Array(64));
    await fails(() => verify(forged, { keys: [jwk] }, "https://a"), "a forged token");
  },
};
