// What the relay's Worker (`worker.ts`) and the edge's side of it (`relay.ts`) both say: what a server says of itself,
// and how the relay's Durable Object is reached. A Worker's main module exports only its handlers, so they're here.
import type { Relay } from "./worker.ts";

/**
 * How long a call to the relay's Durable Object is tried again, in milliseconds. A deploy leaves the object on the
 * code before while it's busy, up to `max_delay` in `wrangler.jsonc`, and a Worker on either side of the deploy may
 * call it meanwhile, as Cloudflare says (Durable Objects' "Code updates"): a call the other doesn't take yet, or no
 * longer, fails until it's switched over. That's also what a reset or a broken connection to it fails with, which
 * Cloudflare has callers try again, on a new stub, waiting longer each time, here up to a second, so a try comes soon
 * after it's switched over. An overloaded one isn't tried again. It's within what a server waits for an answer,
 * 15 seconds.
 */
const SWITCH = 12_000;
/** The protocols a part speaks, from `oldest` to `newest` (`core/protocol.json`). */
export type Protocol = { oldest: number; newest: number };
/** The protocols a part speaks, as it `said`: none known for one that said none, as `OneCore.Protocol.of/1` says. */
export function spoken(said: unknown): Protocol | undefined {
  const { oldest, newest } = (said ?? {}) as Partial<Protocol>;
  return Number.isInteger(oldest) && Number.isInteger(newest) && oldest! <= newest! ? { oldest: oldest!, newest: newest! } : undefined;
}
export type ServerMembership = "inactive" | "joining" | "active" | "leaving";
export type ServerDaemonStatus = "starting" | "running" | "stopping" | "stopped";
export type ServerReadyReason = "joining" | "starting" | "draining" | "stopped";
/** What a server last said it is (`/relay/seen`), and when. */
export interface ServerStatus {
  server_membership: ServerMembership;
  server_daemon_status: ServerDaemonStatus;
  at: number;
}

/**
 * Whether a server or a device is here: `connected` now, or when it was `last_seen` (ms since 1970). A server's
 * `server_membership` and `server_daemon_status` too, the daemon stopped once it's gone, and from those whether it's
 * `server_ready`, active and running, or the reason it isn't.
 */
export interface Presence {
  connected: boolean;
  last_seen?: number;
  server_membership?: ServerMembership;
  server_daemon_status?: ServerDaemonStatus;
  server_ready?: boolean;
  server_ready_reason?: ServerReadyReason;
  /** Whether it answered a ping within `REACH`, where `/relay/seen` was asked to ping it. */
  answered?: boolean;
}

/** Gives what `call` gives of the network's one relay, tried again for up to `SWITCH` where it fails. */
export async function relay<T>(relays: DurableObjectNamespace<Relay>, call: (relay: DurableObjectStub<Relay>) => Promise<T>): Promise<T> {
  const until = Date.now() + SWITCH;
  for (let wait = 250; ; wait = Math.min(wait * 2, 1_000)) {
    try {
      return await call(relays.getByName("relay"));
    } catch (error) {
      if ((error as { overloaded?: boolean }).overloaded || Date.now() + wait > until) throw error;
      console.log(JSON.stringify({ relay: "tried again", error: String(error) }));
      await scheduler.wait(wait);
    }
  }
}

