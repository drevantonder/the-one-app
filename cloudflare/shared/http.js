// What the Workers say over HTTP alike: a refusal thrown and as an answer, JSON, CORS, where a Durable Object is
// made, and a request's body read up to a limit. Each Worker imports only what it uses.

/// A refusal to answer with: `said` is what it says besides `error`.
export class Refused extends Error {
  constructor(status, message, said = {}) {
    super(message);
    this.status = status;
    this.said = said;
  }
}

export const refuse = (status, message, said) => {
  throw new Refused(status, message, said);
};

export const json = (v, status = 200, headers = {}) => Response.json(v, { status, headers });

/// A refusal as an answer: its text, or `{ error, code }` where the caller acts on what kind it is (`CODES`).
export const refusal = (status, why, headers = {}) => json(typeof why === "string" ? { error: why } : why, status, headers);

export const CORS = { "access-control-allow-origin": "*" };

/// Where a Durable Object is made: near its people (`LOCATION`, a location hint), not wherever its first request came from.
export const near = (env) => (env.LOCATION ? { locationHint: env.LOCATION } : {});

/// A request's body as text, or null if it's more than `most` bytes.
export async function textOf(request, most) {
  if (Number(request.headers.get("content-length") || 0) > most) return null;
  if (!request.body) return "";
  const reader = request.body.getReader();
  const parts = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > most) {
      reader.cancel().catch(() => {});
      return null;
    }
    parts.push(value);
  }
  const bytes = new Uint8Array(size);
  parts.reduce((at, part) => (bytes.set(part, at), at + part.byteLength), 0);
  return new TextDecoder().decode(bytes);
}
