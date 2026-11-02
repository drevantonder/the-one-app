// The kernel's formats, for tests/contracts.rs to check against the Rust that reads them: POST { op, ... } → result.
import { next, when } from "../schedules/worker.js";
import { b64u, databaseKey, idOf, keyPair, openAppKey, openSealed, openUnder, sealAppKey, sealTo, sealUnder } from "../shared/crypto.js";

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
