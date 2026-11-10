// The Workers' formats, for tests/contracts.rs to check against the Rust that reads them: POST { op, ... } → result.
import { b64u, canonical, certify, checkCertificate, checkRequest, discoveryUrl, ed25519Public, hex, idOf, joinChallenge, joinCode, joinNonce, openNote, openUnder, sealTo, sealUnder, signedRequest, x25519Public } from "../shared/crypto.js";
import { CODES, copyName, databasePrefix, newestName, stepName, tidiedName, updateRefusal } from "../shared/protocol.js";
import { AGENTS } from "../screen/agents.js";
import { BOUNDS, EXCEPTIONS, TABLES, TRACED, plainWords, workerLogs, workerSpans } from "../telemetry/worker.js";
import { PRODUCTS, query, rows } from "../telemetry/usage.js";

const ops = {
  telemetryBounds: () => BOUNDS,
  telemetryTraced: () => TRACED,
  telemetryWorkerSpans: ({ request, cli }) => workerSpans(request, cli),
  telemetryWorkerLogs: ({ request, cli }) => workerLogs(request, cli),
  telemetryPlainWords: ({ said }) => said.map(plainWords),
  telemetryExceptions: () => EXCEPTIONS,
  /// The one analytics query of the network's resources (`usage`), and an hour's answer (each dataset's groups by its name) as rows.
  telemetryUsage: ({ usage, answer }) => ({ query: query(usage), rows: rows(Object.fromEntries(PRODUCTS.map((p, i) => [`p${i}`, answer[p.dataset] ?? []])), usage, 0, "0".repeat(64)) }),

  /// Each table the telemetry Worker holds rows to: each column's name, and whether a row must have it.
  telemetryTables() {
    return Object.fromEntries(Object.entries(TABLES).map(([table, columns]) => [table, Object.entries(columns).map(([name, [, needed]]) => [name, !!needed])]));
  },

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
  async keys({ seed, domain, fields, at, request, now, pair }) {
    const signedBy = await signedRequest(b64u.decode(seed), await ed25519Public(b64u.decode(seed)), domain, fields, at);
    const checked = await checkRequest(domain, request, now).then((c) => c.by, (err) => ({ error: String(err) }));
    return {
      canonical: new TextDecoder().decode(canonical(fields)),
      signed: signedBy,
      checked,
      joinCode: await joinCode(hex.decode(pair.key), hex.decode(pair.seal)),
      nonce: await joinNonce(hex.decode(pair.seal), hex.decode(pair.key), hex.decode(pair.seal)),
      challenge: b64u.encode(joinChallenge(hex.decode(pair.seal), hex.decode(pair.key), hex.decode(pair.seal), 1_800_000_000_123)),
    };
  },
  /// Who the Settings screens say agents are in an item's apps.
  async agents() {
    return AGENTS;
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
  async discoveryUrl({ issuer }) {
    return { url: discoveryUrl(issuer) };
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
