// The kernel's formats, for tests/contracts.rs to check against the Rust that reads them: POST { op, ... } → result.
import { next, when } from "../schedules/worker.js";
import { b64u, canonical, checkRequest, databaseKey, ed25519Public, epochOf, hex, idOf, inviteProof, joinCode, keyPair, open, openAppKey, openHandoff, openSealed, openUnder, passkeyKeys, seal, sealAppKey, sealHandoff, sealTo, sealUnder, signedRequest, tokenId } from "../shared/crypto.js";

const ops = {
  async next({ spec, now }) {
    return { next: next(when(spec), now) };
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
      joinCode: await joinCode(hex.decode(pair.key)),
      proof: await inviteProof(b64u.decode(secret), hex.decode(pair.key)),
      passkey: { sign: hex.encode(await ed25519Public(pk.sign)), seal: b64u.encode(pk.seal) },
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
