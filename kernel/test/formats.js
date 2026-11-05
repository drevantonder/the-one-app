// The kernel's formats, for tests/contracts.rs to check against the Rust that reads them: POST { op, ... } → result.
import { appOf, askedFor, b64u, BEHIND, canonical, certify, checkCertificate, epochHash, checkEntry, checkRequest, databaseKey, ed25519Public, epochOf, hex, idOf, inviteProof, joinChallenge, joinCode, joinNonce, keyPair, mayHand, MOST_NEEDED, oldestNeeded, open, openAppKey, openHandoff, openSealed, openUnder, passkeyKeys, seal, sealAppKey, sealHandoff, sealTo, sealUnder, signEntry, signedRequest, tokenId, twinPublic, waited, x25519Public } from "../shared/crypto.js";
import { members } from "../web/members.js";

const ops = {
  /// Session endpoints' certificates: one signed here, and each of `check` checked at `now`.
  async certificates({ seed, endpoint, until, at, check, now }) {
    const made = await certify(b64u.decode(seed), await ed25519Public(b64u.decode(seed)), hex.decode(endpoint), until, at);
    const checked = await Promise.all(check.map((c) => checkCertificate(c, now).catch((err) => ({ error: String(err.message || err) }))));
    return { made, checked };
  },
  async sealAppKey({ scopeKey, app, key }) {
    return { sealed: b64u.encode(await sealAppKey(b64u.decode(scopeKey), app, b64u.decode(key))) };
  },
  async openAppKey({ scopeKey, app, sealed, database }) {
    const key = await openAppKey(b64u.decode(scopeKey), app, b64u.decode(sealed));
    return { key: b64u.encode(key), database: b64u.encode(await databaseKey(key, database)) };
  },
  async keyPair() {
    const p = await keyPair();
    return { secret: b64u.encode(p.secret), public: b64u.encode(p.public) };
  },
  async sealTo({ to, plain }) {
    return { sealed: b64u.encode(await sealTo(b64u.decode(to), b64u.decode(plain))) };
  },
  async openSealed({ secret, public: pub, sealed }) {
    return { plain: b64u.encode(await openSealed({ secret: b64u.decode(secret), public: b64u.decode(pub) }, b64u.decode(sealed))) };
  },
  async sealUnder({ secret, plain }) {
    return { sealed: b64u.encode(await sealUnder(secret, b64u.decode(plain))), id: await idOf(secret) };
  },
  async keys({ seed, domain, fields, at, request, now, epoch, key, name, plain, sealed, to, handoff, pair, token, secret, prf }) {
    const signedBy = await signedRequest(b64u.decode(seed), await ed25519Public(b64u.decode(seed)), domain, fields, at);
    const checked = await checkRequest(domain, request, now).then((c) => c.by, (err) => ({ error: String(err) }));
    const opened = await open(b64u.decode(key), name, b64u.decode(sealed));
    const pk = await passkeyKeys(b64u.decode(prf));
    return {
      canonical: new TextDecoder().decode(canonical(fields)),
      signed: signedBy,
      checked,
      sealed: b64u.encode(await seal(epoch, b64u.decode(key), name, b64u.decode(plain))),
      opened: b64u.encode(opened),
      epoch: epochOf(b64u.decode(sealed)),
      handoff: await sealHandoff(b64u.decode(to), handoff),
      handedBack: await openHandoff({ secret: b64u.decode(pair.secret), public: b64u.decode(pair.public) }, pair.sealed),
      tokenId: await tokenId(token),
      epochHash: await epochHash(b64u.decode(key)),
      twin: hex.encode(twinPublic(hex.decode(pair.key))),
      joinCode: await joinCode(hex.decode(pair.key), hex.decode(pair.seal)),
      proof: await inviteProof(b64u.decode(secret), hex.decode(pair.key), hex.decode(pair.seal)),
      nonce: await joinNonce(hex.decode(pair.key), hex.decode(pair.seal)),
      challenge: b64u.encode(joinChallenge(hex.decode(pair.key), hex.decode(pair.seal), 1_800_000_000_123)),
      passkey: { sign: hex.encode(await ed25519Public(pk.sign)), seal: b64u.encode(pk.seal), sealPublic: hex.encode(await x25519Public(pk.seal)) },
    };
  },
  /// An entry signed with `seed`, and one Rust signed, checked: → the one signed, as POST /v1/entries takes it.
  async entries({ seed, fields, body, entry }) {
    const signed = await signEntry(b64u.decode(seed), await ed25519Public(b64u.decode(seed)), fields, body);
    const checked = await checkEntry({ header: b64u.decode(entry.header), signature: b64u.decode(entry.signature), body: b64u.decode(entry.body) }).then((e) => ({ hash: e.hash, user: e.user, kind: e.content.kind }), (err) => ({ error: String(err) }));
    const tampered = await checkEntry({ header: b64u.decode(entry.header), signature: b64u.decode(entry.signature), body: new TextEncoder().encode("{}") }).then(() => "checked", (err) => String(err));
    const b = (v) => b64u.encode(v);
    return { signed: { header: b(signed.header), signature: b(signed.signature), body: b(signed.body) }, hash: signed.hash, checked, tampered };
  },
  /// Member entries' content as the web client builds it: `[[builder, args]]` → each content.
  async members({ calls }) {
    return { contents: calls.map(([name, args]) => members[name](args)) };
  },
  /// Entries Rust refuses: → why JavaScript does too, or "checked".
  async refused({ entries }) {
    const check = (e) => checkEntry({ header: b64u.decode(e.header), signature: b64u.decode(e.signature), body: b64u.decode(e.body) });
    return { refused: await Promise.all(entries.map((e) => check(e).then(() => "checked", (err) => String(err)))) };
  },
  /// What a node is handed with each kind of request: `[[route, need]]` → whether each is, and `[[hint, newest]]` → the
  /// oldest epoch each needs.
  async handed({ hands, hints }) {
    return { hands: hands.map(([route, need]) => mayHand(route, need)), oldest: hints.map(([hint, newest]) => oldestNeeded(hint, newest)), apps: hands.map(([route]) => appOf(route)) };
  },
  /// What a client does with a node's 428 and auth's word while it waits to be let in: `[[route, node, answer]]` → what
  /// each asks it to hand, or null; `[[status, said]]` → what each says.
  async joining({ asked, said }) {
    return { asked: asked.map(([route, node, answer]) => askedFor(route, node, answer)), said: said.map(([status, body]) => waited(status, body)), most: MOST_NEEDED, behind: BEHIND };
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
