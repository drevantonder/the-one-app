// A stand-in for the relay Worker in tests: the endpoints it allows are `ALLOWED`, `{ <key hex>: { admin } }`.
import { WorkerEntrypoint } from "cloudflare:workers";

export default class extends WorkerEntrypoint {
  async allowed(key) {
    return JSON.parse(this.env.ALLOWED || "{}")[key] || null;
  }
}
