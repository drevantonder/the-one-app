// The kernel's formats, for tests/contracts.rs to check against the Rust that reads them: POST { op, ... } → result.
import { appKey, b64u, databaseKey, idOf, keyPair, openSealed, openUnder, sealTo, sealUnder } from "../shared/crypto.js";

const ops = {
  async derive({ scopeKey, app, database }) {
    const a = await appKey(b64u.decode(scopeKey), app);
    return { app: b64u.encode(a), database: b64u.encode(await databaseKey(a, database)) };
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
