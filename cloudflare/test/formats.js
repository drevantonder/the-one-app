// The Workers' formats, for tests/contracts.rs to check against the Rust that reads them: POST { op, ... } → result.
import { b64u, canonical, certify, checkCertificate, checkRequest, ed25519Public, hex, idOf, inviteProof, joinChallenge, joinCode, joinNonce, openNote, openUnder, passkeyKeys, sealTo, sealUnder, signedRequest, waited, x25519Public } from "../shared/crypto.js";
import { CODES, copyName, databasePrefix, newestName, stepName, tidiedName, updateRefusal } from "../shared/protocol.js";
import { AGENTS } from "../screen/agents.js";

const ops = {
  /// Session endpoints' certificates: one signed here, and each of `check` checked at `now`.
  async certificates({ seed, endpoint, until, at, check, now }) {
    const made = await certify(b64u.decode(seed), await ed25519Public(b64u.decode(seed)), hex.decode(endpoint), until, at);
    const checked = await Promise.all(check.map((c) => checkCertificate(c, now).catch((err) => ({ error: String(err.message || err) }))));
    return { made, checked };
  },
  async sealTo({ to, plain }) {
    return { sealed: b64u.encode(await sealTo(b64u.decode(to), b64u.decode(plain))) };
  },
  /// A note, opened as the service worker does: the note, or why not.
  async openNote({ secret, public: pub, sealed }) {
    return { note: await openNote({ secret: b64u.decode(secret), public: b64u.decode(pub) }, b64u.decode(sealed)).catch((err) => ({ error: String(err.message || err) })) };
  },
  async sealUnder({ secret, plain }) {
    return { sealed: b64u.encode(await sealUnder(secret, b64u.decode(plain))), id: await idOf(secret) };
  },
  async keys({ seed, domain, fields, at, request, now, pair, secret, prf }) {
    const signedBy = await signedRequest(b64u.decode(seed), await ed25519Public(b64u.decode(seed)), domain, fields, at);
    const checked = await checkRequest(domain, request, now).then((c) => c.by, (err) => ({ error: String(err) }));
    const pk = await passkeyKeys(b64u.decode(prf));
    return {
      canonical: new TextDecoder().decode(canonical(fields)),
      signed: signedBy,
      checked,
      joinCode: await joinCode(hex.decode(pair.key), hex.decode(pair.seal)),
      proof: await inviteProof(b64u.decode(secret), hex.decode(pair.key), hex.decode(pair.seal)),
      nonce: await joinNonce(hex.decode(pair.seal), hex.decode(pair.key), hex.decode(pair.seal)),
      challenge: b64u.encode(joinChallenge(hex.decode(pair.seal), hex.decode(pair.key), hex.decode(pair.seal), 1_800_000_000_123)),
      passkey: { sign: hex.encode(await ed25519Public(pk.sign)), seal: b64u.encode(pk.seal), sealPublic: hex.encode(await x25519Public(pk.seal)) },
    };
  },
  /// Who the Settings screens say agents are in an item's apps.
  async agents() {
    return AGENTS;
  },
  /// What auth's word says while a device waits to be let in: `[[status, said]]` → what each says.
  async joining({ said }) {
    return { said: said.map(([status, body]) => waited(status, body)) };
  },
  /// The bucket's names, the refusal the Workers make and the codes answers carry: `[prefix, n]` → each of the names, and `[scope, app, database]`
  /// → its prefix.
  async names({ names, prefixes, yours }) {
    return {
      names: names.map(([prefix, n]) => [stepName(prefix, n), copyName(prefix, n), tidiedName(prefix), newestName(prefix)]),
      prefixes: prefixes.map(([scope, app, database]) => databasePrefix(scope, app, database)),
      refusal: updateRefusal(yours),
      codes: CODES,
    };
  },
  async openUnder({ secret, sealed }) {
    return { plain: b64u.encode(await openUnder(secret, b64u.decode(sealed))) };
  },
};

export default {
  async fetch(request) {
    const body = await request.json();
    try {
      return Response.json(await ops[body.op](body));
    } catch (err) {
      return Response.json({ error: String(err) }, { status: 400 });
    }
  },
};
