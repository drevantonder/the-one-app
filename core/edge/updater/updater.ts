// The network update: one per network (`NetworkUpdater`), which takes the network's servers to the latest build
// published to the network, by itself, as self-updating software and a small cluster that holds leases do (Tailscale,
// CockroachDB's rolling upgrades). Nobody runs `one update`: publishing a build is all.
//
//     up to date ──a newer build published──► preparing
//       every awake server downloads it, checks it and says how it would take it: loaded in place, or restarted (why)
//     preparing ──all checked──► loading
//       every server that loads it in place takes it at once
//     loading ──► restarting
//       one server at a time, never the last ready one: a lone ready server that must restart waits for a quiet
//       moment (no calls for a while), or Update Now
//     restarting ──► up to date
//
//     any state ──a newer build──► preparing, for the newer one
//     preparing, loading or restarting ──a server goes back, or doesn't finish──►
//       held (which, why): Try Again (Update Now), or a newer build, starts it again
//     up to date ──a server that missed one is back──► preparing, for it alone
//
// A server that's asleep or gone as an update begins, or goes while it's under way, is skipped, and caught up under
// the same rules once it's back. Each server prepares and goes as it's told, and decides itself how it takes a build
// (`clients/cli/src/serving.rs`): this decides only when.
//
// It hears only through the relay, which is bound to it, never the other way round (Cloudflare deploys no Worker
// bound to one that isn't there yet, so two bound to each other couldn't be deployed to a new network): a build
// published or Update Now (`/relay/update`), what each server says of itself as it changes (`/relay/seen`), and a
// server coming or going there. Each time it's given who's here (`Presence`), and answers with what it is, and what
// every server is to do, which the relay tells them all (`notifications/one/update`). So it decides only as it's
// told something, and its alarm only marks what didn't finish in time: a server that hears nothing new from it has
// nothing to do.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";

type NetworkUpdateStatus = "up_to_date" | "preparing" | "loading" | "restarting" | "held";
type ServerUpdateStatus = "up_to_date" | "downloading" | "checked" | "verifying";
/** What a server is told to do: prepare the build, take it now, or take it once it has had no calls for a while. */
type Instruction = "prepare" | "go" | "go_when_quiet";

/** What a server last said of itself, as it says it at the relay (`OneCore.Servers.say/0`). */
export interface Reported {
  version?: string;
  name?: string;
  /** Whether it's installed with `one serve on`, which takes builds: one a developer runs from source never does. */
  installed?: boolean;
  server_update_status?: ServerUpdateStatus;
  update_method?: "hot_load" | "restart" | null;
  update_method_reason?: string | null;
  next_build?: string | null;
  went_back?: string | null;
}

/** Who's here, as the relay says of each server it knows (`relay.ts`'s `Presence`), with the build it last said. */
export interface Presence {
  connected: boolean;
  server_membership?: string;
  server_ready?: boolean;
  version?: string;
}

/** What it's told. */
export type Input =
  | { published: string }
  | { now: true }
  | { reported: { key: string } & Reported }
  | { seen: string }
  | { status: true };

/** A server taking part in an update, and when each step of it was reached, in ms after the build was published. */
interface Taking {
  key: string;
  prepared?: number;
  went?: number;
  /** When it first said it's on the build: loaded, or started and not yet running. */
  loaded?: number;
  ready?: number;
  /** Whether it has begun since it was told to, so a `went_back` it says is this update's. */
  begun?: boolean;
}

interface State {
  network_update_status: NetworkUpdateStatus;
  /** The build the network takes: the latest published. */
  build?: string;
  /** When it was published, in ms since 1970. */
  published?: number;
  servers: Taking[];
  /** Asleep or gone as it began, or since: each is caught up once it's back. */
  skipped: string[];
  waiting_for?: "quiet" | null;
  /** Update Now was asked, so a lone server restarts without waiting for a quiet moment. */
  now?: boolean;
  held?: { server: string; name?: string; why: string } | null;
  /** When what's under way is taken as not finishing, in ms since 1970. */
  until?: number | null;
  /** When each step of the whole was reached, in ms after the build was published. */
  timings: Record<string, number>;
}

/** How long every server has to prepare, and a server that restarts to be back, as `serving.rs`'s `START`, and more. */
const PREPARING = 10 * 60_000;
const RESTARTING = 5 * 60_000;

const MESSAGE = "notifications/one/update";

export default class extends WorkerEntrypoint<UpdaterEnv> {
  fetch() {
    return new Response("The network's updater is reached through its relay", { status: 404 });
  }

  /** Tells the network's one updater `input`, with who's here: what it is, and what to tell every server, if anything. */
  update(input: Input, presence: Record<string, Presence | null>) {
    return this.env.UPDATER.get(this.env.UPDATER.idFromName("network")).update(input, presence);
  }
}

export class NetworkUpdater extends DurableObject<UpdaterEnv> {
  async update(input: Input, presence: Record<string, Presence | null>): Promise<{ status: object; broadcast?: string }> {
    const state: State = (await this.ctx.storage.get<State>("state")) ?? { network_update_status: "up_to_date", servers: [], skipped: [], timings: {} };
    const reported = await this.reported();
    // A server the relay knows that hasn't said more of itself here, as every one hasn't when this is first deployed,
    // is on the build the relay says, with no update under way.
    for (const [key, here] of Object.entries(presence)) reported[key] ??= { version: here?.version, server_update_status: "up_to_date" };
    const before = JSON.stringify([state, this.instructions(state, reported)]);
    let concerned = false;

    if ("published" in input) {
      // Only a newer build moves it: one published again, as a release run again publishes, changes nothing.
      if (input.published !== state.build && (!state.build || newer(input.published, state.build))) this.begin(state, input.published, presence, reported);
    } else if ("now" in input) {
      if (state.network_update_status === "held" || (state.network_update_status === "up_to_date" && state.build)) this.begin(state, state.build!, presence, reported);
      else state.now = true;
    } else if ("reported" in input) {
      const { key, ...said } = input.reported;
      reported[key] = { ...reported[key], ...said };
      await this.ctx.storage.put(`reported ${key}`, reported[key]);
      const taking = state.servers.find((server) => server.key === key);
      if (taking && said.server_update_status && said.server_update_status !== "up_to_date") taking.begun = true;
      concerned = !!this.instructions(state, reported)[key];
    }
    // A server coming or going, or saying what it is, moves it on as anything does.
    this.step(state, presence, reported);

    await this.ctx.storage.put("state", state);
    await this.ctx.storage.setAlarm(state.until ?? Date.now() + 365 * 86_400_000);
    const status = this.status(state, reported);
    const changed = JSON.stringify([state, this.instructions(state, reported)]) !== before;
    if (changed) console.log(JSON.stringify({ updater: status }));
    // Every server is told again what to do once anything changed, and as one that has something to do says what it
    // is, so one that missed being told, as one reconnecting does, hears it.
    return { status, ...((changed || concerned) && { broadcast: JSON.stringify({ jsonrpc: "2.0", method: MESSAGE, params: status }) }) };
  }

  /** What didn't finish in time: the update is held. Nobody is told until something next comes in. */
  async alarm() {
    const state = await this.ctx.storage.get<State>("state");
    if (!state?.until || state.until > Date.now()) return;
    const reported = await this.reported();
    const preparing = state.network_update_status === "preparing";
    const late = state.servers.find((server) => server.ready === undefined && (preparing ? server.prepared === undefined : server.went !== undefined));
    const why = preparing ? `didn't finish preparing within ${PREPARING / 60_000} min` : `didn't take it within ${RESTARTING / 60_000} min`;
    this.hold(state, late?.key ?? "", late && reported[late.key]?.name, why);
    await this.ctx.storage.put("state", state);
    console.log(JSON.stringify({ updater: this.status(state, reported) }));
  }

  async reported(): Promise<Record<string, Reported>> {
    const kept = await this.ctx.storage.list<Reported>({ prefix: "reported " });
    return Object.fromEntries([...kept].map(([at, said]) => [at.slice("reported ".length), said]));
  }

  /** Begins taking `build`: every server that's ready and not on it prepares it, and the rest are skipped. */
  begin(state: State, build: string, presence: Record<string, Presence | null>, reported: Record<string, Reported>) {
    const known = Object.keys(reported).filter((key) => this.takes(key, presence, reported));
    const behind = known.filter((key) => reported[key].version !== build);
    Object.assign(state, {
      network_update_status: "preparing",
      build,
      published: state.build === build && state.published ? state.published : Date.now(),
      servers: behind.filter((key) => presence[key]?.server_ready).map((key) => ({ key })),
      skipped: behind.filter((key) => !presence[key]?.server_ready),
      waiting_for: null,
      now: false,
      held: null,
      until: Date.now() + PREPARING,
      timings: {},
    } satisfies State);
    if (!state.servers.length) this.finish(state);
  }

  /**
   * Whether the server whose key is `key` takes builds: one installed with `one serve on` (one a developer runs from
   * source never does) that's one of the network's servers, or was when it was last seen, not one that left.
   */
  takes(key: string, presence: Record<string, Presence | null>, reported: Record<string, Reported>) {
    return reported[key].installed !== false && !["leaving", "inactive"].includes(presence[key]?.server_membership ?? "");
  }

  /** Moves the update on as far as what's known of every server lets it. */
  step(state: State, presence: Record<string, Presence | null>, reported: Record<string, Reported>) {
    const status = state.network_update_status;
    if (status === "held") return;
    if (status === "up_to_date") {
      // One that missed an update is back.
      const back =
        state.build &&
        Object.keys(reported).some((key) => this.takes(key, presence, reported) && reported[key].version !== state.build && presence[key]?.server_ready);
      if (back) this.begin(state, state.build!, presence, reported);
      return;
    }
    const at = Date.now() - state.published!;
    const on = (key: string) => reported[key]?.version === state.build && reported[key]?.server_update_status === "up_to_date";
    for (const server of state.servers) {
      const said = reported[server.key] ?? {};
      if (server.prepared === undefined && said.server_update_status === "checked" && said.next_build === state.build) server.prepared = at;
      if (server.loaded === undefined && said.version === state.build) server.loaded = at;
      if (server.ready === undefined && on(server.key) && presence[server.key]?.server_ready) server.ready = at;
      // Back on the build it was on, saying why: it went back, or couldn't prepare.
      if (server.begun && server.ready === undefined && said.server_update_status === "up_to_date" && said.version !== state.build && said.went_back)
        return this.hold(state, server.key, said.name, said.went_back);
    }
    // One that's gone, but for one restarting to take it, is caught up once it's back.
    const method = (key: string) => reported[key]?.update_method;
    const gone = state.servers.filter(
      (server) => server.ready === undefined && !presence[server.key]?.connected && (server.went === undefined || method(server.key) !== "restart"),
    );
    state.skipped.push(...gone.map((server) => server.key));
    state.servers = state.servers.filter((server) => !gone.includes(server));

    if (state.network_update_status === "preparing") {
      if (state.servers.some((server) => server.prepared === undefined && server.ready === undefined)) return;
      state.timings.prepared = at;
      state.network_update_status = "loading";
      state.until = Date.now() + RESTARTING;
    }
    if (state.network_update_status === "loading") {
      const loading = state.servers.filter((server) => method(server.key) !== "restart");
      for (const server of loading) server.went ??= at;
      if (loading.length) state.timings.go ??= at;
      if (loading.some((server) => server.ready === undefined)) return;
      if (loading.length) state.timings.loaded = Math.max(...loading.map((server) => server.loaded ?? at));
      state.network_update_status = "restarting";
    }
    if (state.network_update_status === "restarting") {
      const restarting = state.servers.filter((server) => method(server.key) === "restart" && server.ready === undefined);
      const [next] = restarting.filter((server) => server.went !== undefined).concat(restarting);
      if (!next) return this.finish(state);
      if (next.went !== undefined && state.waiting_for !== "quiet") return;
      // Never the last ready one: another must answer while it restarts, unless it's asked to now.
      const others = Object.keys(presence).some((key) => key !== next.key && presence[key]?.server_ready && this.takes(key, presence, reported));
      if (others || state.now) {
        next.went = at;
        state.timings.go ??= at;
        state.waiting_for = null;
        state.until = Date.now() + RESTARTING;
      } else {
        state.waiting_for = "quiet";
        state.until = null;
        // It goes by itself once it's quiet, and says so as it begins.
        if (reported[next.key]?.server_update_status === "verifying") {
          next.went = at;
          state.waiting_for = null;
          state.until = Date.now() + RESTARTING;
        }
      }
    }
  }

  finish(state: State) {
    state.network_update_status = "up_to_date";
    state.waiting_for = null;
    state.until = null;
    if (state.published) state.timings.ready = Date.now() - state.published;
  }

  hold(state: State, server: string, name: string | undefined, why: string) {
    state.network_update_status = "held";
    state.held = { server, ...(name && { name }), why };
    state.waiting_for = null;
    state.until = null;
  }

  /** What every server taking part is to do now, by its key. */
  instructions(state: State, reported: Record<string, Reported>): Record<string, Instruction> {
    const told: Record<string, Instruction> = {};
    if (state.network_update_status === "held" || state.network_update_status === "up_to_date") return told;
    for (const server of state.servers) {
      if (server.ready !== undefined) continue;
      if (state.network_update_status === "preparing" && server.prepared === undefined) told[server.key] = "prepare";
      else if (server.went !== undefined && reported[server.key]?.server_update_status === "checked") told[server.key] = "go";
      else if (server.went !== undefined && reported[server.key]?.server_update_status !== "checked") continue;
      else if (state.network_update_status === "restarting" && state.waiting_for === "quiet" && reported[server.key]?.update_method === "restart")
        told[server.key] = "go_when_quiet";
      // A server that went back to the build before, or never prepared it, as one whose prepare was missed, prepares it.
      if (reported[server.key]?.server_update_status === "up_to_date" && reported[server.key]?.version !== state.build) told[server.key] ??= "prepare";
    }
    return told;
  }

  /** What it is, as servers and people are told: `network_update_status`, the build, how far it is and what it's waiting on. */
  status(state: State, reported: Record<string, Reported>) {
    // A machine's name, and its key's start where two are called the same.
    const names = Object.values(reported).map((said) => said.name);
    const name = (key: string) => {
      const called = reported[key]?.name;
      return called && names.indexOf(called) === names.lastIndexOf(called) ? called : `${called ?? "server"} ${key.slice(0, 8)}`;
    };
    return {
      network_update_status: state.network_update_status,
      build: state.build ?? null,
      done: state.servers.filter((server) => server.ready !== undefined).length,
      of: state.servers.length,
      waiting_for: state.waiting_for ?? null,
      held: state.held ?? null,
      skipped: state.skipped.map(name),
      servers: this.instructions(state, reported),
      timings: {
        ...state.timings,
        ...Object.fromEntries(state.servers.map(({ key, prepared, went, loaded, ready }) => [name(key), { prepared, went, loaded, ready }])),
      },
    };
  }
}

/** Whether the build `a` is newer than `b`: `<version>+<count>.<commit>`, by its commit's count. */
function newer(a: string, b: string) {
  const count = (build: string) => Number(/\+(\d+)\./.exec(build)?.[1] ?? 0);
  return count(a) > count(b);
}
