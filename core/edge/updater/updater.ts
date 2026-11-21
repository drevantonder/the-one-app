// The network update: one per network (`NetworkUpdater`), which takes the network's servers to the latest builds
// the core released (`core.ts`), by itself, as self-updating software and a small cluster that holds leases do
// (Tailscale, CockroachDB's rolling upgrades). Nobody runs `one update`: releasing a build is all.
//
//     up to date ──a newer build published: the server's, or a `one` that runs servers──► preparing
//       every awake server not on its latest pair (the server build, and the `one` that runs it) downloads it and
//       checks it starts
//     preparing ──all checked──► restarting
//       one server at a time restarts on it, never the last ready one, as Elixir fleets roll a release: a lone ready
//       server waits for a quiet moment (no calls for a while), or Update Now
//     restarting ──► up to date
//
//     any state ──a newer build──► preparing, for the newer one
//     preparing or restarting ──a server goes back, or doesn't finish──► it's behind (which, why), and the rest go
//       on: a machine can be off or broken, and one mustn't keep every other from a build. It tries again by itself
//       1, 5 and then every 15 min after it went back (`RETRIES`), or at once with Update Now or a newer build
//     up to date ──a server that missed one is back──► preparing, for it alone
//
// The edge has an update of its own beside it (`edge_update`), which never holds the servers': once they're all up to
// date on a build the edge is older than, one of them with an app that deploys the edge (`deploys_edge`) is told to
// deploy its build's (`deploy_edge`, `core/edge/deploy.js`, which rolls the edge back if it doesn't answer as that
// build). It's tried once for each build, and Update Now tries again:
//
//     (none) ──the servers are up to date on a build the edge is older than──► deploying
//     deploying ──the edge runs the build──► (none)
//     deploying ──the app can't, as it has no token, or the Vault doesn't open there──► needs_setup, said once
//     deploying ──the deploy went back, or 10 min passed──► failed, said once
//     any ──a newer build──► (none), and servers first
//
// A server that's asleep or gone as an update begins, or goes while it's under way, is skipped, and caught up under
// the same rules once it's back. Each server prepares and goes as it's told (`clients/cli/src/serving.rs`): this
// decides only when.
//
// It hears only through the relay's Durable Object, which calls it: a build released (`core.ts`) or Update Now (`/relay/update`), what each server says of itself as it changes (`/relay/seen`), and a
// server coming or going there. Each time it's given who's here (`Presence`), and answers with what it is, and what
// every server is to do, which the relay tells them all (`notifications/one/update`). So it decides only as it's
// told something, and its alarm only marks what didn't finish in time, and when one behind tries again: a server that
// hears nothing new from it has nothing to do.
import { DurableObject } from "cloudflare:workers";
import { relay } from "../relay/common.ts";

type NetworkUpdateStatus = "up_to_date" | "preparing" | "restarting";
type ServerUpdateStatus = "up_to_date" | "downloading" | "checked" | "verifying";
/**
 * What a server is told to do: prepare the build, take it now, take it once it has had no calls for a while, or
 * deploy its build's edge.
 */
type Instruction = "prepare" | "go" | "go_when_quiet" | "deploy_edge";

/** What a server last said of itself, as it says it at the relay (`OneCore.Servers.say/0`). */
export interface Reported {
  version?: string;
  name?: string;
  /** Whether it's installed with `one serve on`, which takes builds: one a developer runs from source never does. */
  installed?: boolean;
  server_update_status?: ServerUpdateStatus;
  next_build?: string | null;
  /** The builds' name of the `one` that runs it (`one-cli/linux-x86_64`), and its build, now and being taken. */
  program_name?: string | null;
  program?: string | null;
  next_program?: string | null;
  went_back?: string | null;
  /** Whether an app on it deploys the network's edge (One Core's hook `edge`), which the edge's own update needs. */
  deploys_edge?: boolean;
  /** Why it didn't deploy the edge of `build`, and whether that's as it isn't set up to (`setup`), until one deploys. */
  edge_went_back?: { build: string; why: string; setup?: boolean } | null;
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
  | { published: string; latest: Record<string, string> }
  | { now: true }
  | { reported: { key: string } & Reported }
  | { seen: string }
  | { status: true };

/** A server taking part in an update, and when each step of it was reached, in ms after the build was published. */
interface Taking {
  key: string;
  prepared?: number;
  went?: number;
  /** When it first said it's on the build: started, and not yet running. */
  started?: number;
  ready?: number;
  /** Whether it has begun since it was told to, so a `went_back` it says is this update's. */
  begun?: boolean;
}

interface State {
  network_update_status: NetworkUpdateStatus;
  /** The build the network takes: the newest published, of a server or a `one` that runs one. */
  build?: string;
  /** The latest build of each name a server takes, `server-code` and each `one`, as `OneCore.Builds` names them. */
  latest?: Record<string, string>;
  /** When it was published, in ms since 1970. */
  published?: number;
  servers: Taking[];
  /** Asleep or gone as it began, or since: each is caught up once it's back. */
  skipped: string[];
  waiting_for?: "quiet" | null;
  /** Update Now was asked, so a lone server restarts without waiting for a quiet moment. */
  now?: boolean;
  /** The servers that went back from this build, or didn't take it in time, why, and when each tries again. */
  went_back?: { server: string; name?: string; why: string; retry_at?: number }[];
  /** How many times each server went back from this build, which its next try waits longer for (`RETRIES`). */
  tries?: Record<string, number>;
  /** When what's under way is taken as not finishing, in ms since 1970. */
  until?: number | null;
  /** The edge's own update, while the edge is older than the servers' build: which server deploys it, and how it went. */
  edge_update?: EdgeUpdate | null;
  /** When each step of the whole was reached, in ms after the build was published. */
  timings: Record<string, number>;
  /**
   * The edge's build, which serves the app's window page (`clients/app/src/page.rs`): every server is told again
   * once it's deployed, as it first hears from the relay, and tells the apps watching it, which take a newer page.
   */
  edge?: string;
}

/** The edge's update, for one build: a server `deploying` it, or why it didn't (`needs_setup`, `failed`). */
interface EdgeUpdate {
  build: string;
  status: "deploying" | "needs_setup" | "failed";
  server: string;
  why?: string;
  /** While it's deploying, when it's taken as failed, in ms since 1970. */
  until?: number;
}

/** How long every server has to prepare, and a server that restarts to be back, as `serving.rs`'s `START`, and more. */
const PREPARING = 10 * 60_000;
const RESTARTING = 5 * 60_000;
/** How long a server has to deploy the edge, as `deploy.js` takes a few minutes, with its check and a rollback. */
const DEPLOYING_EDGE = 10 * 60_000;
/** How long a server that went back waits before it tries again, after its first time, its second, and every time after. */
const RETRIES = [1, 5, 15].map((minutes) => minutes * 60_000);

const MESSAGE = "notifications/one/update";

export class NetworkUpdater extends DurableObject<Env> {
  async update(input: Input, presence: Record<string, Presence | null>): Promise<{ status: object; broadcast?: string }> {
    const state: State = (await this.ctx.storage.get<State>("state")) ?? { network_update_status: "up_to_date", servers: [], skipped: [], timings: {} };
    const reported = await this.reported();
    // Kept by the edge before its update was its own (0.34.294), which held the servers' as `updating_edge`.
    if ((state.network_update_status as string) === "updating_edge") this.finish(state);
    // A server the relay no longer knows, which it forgot as it was gone too long, is forgotten here too.
    const gone = Object.keys(reported).filter((key) => !(key in presence));
    if (gone.length) {
      await this.ctx.storage.delete(gone.map((key) => `reported ${key}`));
      for (const key of gone) delete reported[key];
      state.skipped = state.skipped.filter((key) => !gone.includes(key));
      state.went_back = state.went_back?.filter((server) => !gone.includes(server.server));
    }
    // A server the relay knows that hasn't said more of itself here, as every one hasn't when this is first deployed,
    // is on the build the relay says, with no update under way.
    for (const [key, here] of Object.entries(presence)) reported[key] ??= { version: here?.version, server_update_status: "up_to_date" };
    const before = JSON.stringify([state, this.instructions(state, reported)]);
    let concerned = false;
    state.edge = this.env.VERSION || state.edge;

    if ("published" in input) {
      // Only a newer build moves it: one published again, as a release run again publishes, changes nothing.
      const moved = Object.entries(input.latest).filter(([name, build]) => !state.latest?.[name] || newer(build, state.latest[name]));
      if (moved.length || !state.build) {
        state.latest = { ...state.latest, ...Object.fromEntries(moved) };
        this.begin(state, state.build && !newer(input.published, state.build) ? state.build : input.published, presence, reported);
      }
    } else if ("now" in input) {
      // Update Now takes everyone not on the build to it, those behind too, and one under way on without waiting.
      if (state.network_update_status !== "preparing" && state.network_update_status !== "restarting") {
        if (state.build) this.begin(state, state.build, presence, reported);
      } else state.now = true;
      // And the edge's, which tried once for this build, tries again.
      if (state.edge_update?.status !== "deploying") state.edge_update = null;
    } else if ("reported" in input) {
      const { key, ...said } = input.reported;
      reported[key] = { ...reported[key], ...said };
      await this.ctx.storage.put(`reported ${key}`, reported[key]);
      const taking = state.servers.find((server) => server.key === key);
      if (taking && said.server_update_status && said.server_update_status !== "up_to_date") taking.begun = true;
      concerned = !!this.instructions(state, reported)[key];
    }
    // A server coming or going, or saying what it is, moves it on as anything does, and then the edge's.
    this.step(state, presence, reported);
    for (const key of this.updateEdge(state, presence, reported)) await this.ctx.storage.put(`reported ${key}`, reported[key]);

    await this.ctx.storage.put("state", state);
    const alarm = alarmAt(state);
    if (alarm !== null) await this.ctx.storage.setAlarm(alarm);
    else await this.ctx.storage.deleteAlarm();
    const status = this.status(state, reported);
    const changed = JSON.stringify([state, this.instructions(state, reported)]) !== before;
    if (changed) said(state, status);
    // Every server is told again what to do once anything changed, and as one that has something to do says what it
    // is, so one that missed being told, as one reconnecting does, hears it.
    return { status, ...((changed || concerned) && { broadcast: JSON.stringify({ jsonrpc: "2.0", method: MESSAGE, params: status }) }) };
  }

  /**
   * What didn't finish in time: each server late is behind, and the update goes on without them, as the relay asks it
   * again with who's here, which tells every server what's next. And once one behind is due to try again, the relay
   * asks it as Update Now does, which takes each not on the build to it.
   */
  async alarm() {
    const state = await this.ctx.storage.get<State>("state");
    if (!state) return;
    const edge = state.edge_update;
    if (edge?.status === "deploying" && edge.until! <= Date.now()) {
      Object.assign(edge, { status: "failed", why: `it wasn't deployed within ${DEPLOYING_EDGE / 60_000} min`, until: undefined });
      edgeSaid(edge);
      await this.ctx.storage.put("state", state);
    }
    if (state.until && state.until <= Date.now()) {
      const reported = await this.reported();
      const preparing = state.network_update_status === "preparing";
      const late = state.servers.filter((server) => server.ready === undefined && (preparing ? server.prepared === undefined : server.went !== undefined));
      const why = preparing ? `didn't finish preparing within ${PREPARING / 60_000} min` : `didn't take it within ${RESTARTING / 60_000} min`;
      for (const server of late) this.wentBack(state, server.key, reported[server.key]?.name, why);
      state.until = null;
      await this.ctx.storage.put("state", state);
      said(state, this.status(state, reported));
    }
    const due = (alarmAt(state) ?? Infinity) <= Date.now();
    await relay(this.env.RELAY, (relay) => relay.updated(due ? { now: true } : { status: true }));
  }

  async reported(): Promise<Record<string, Reported>> {
    const kept = await this.ctx.storage.list<Reported>({ prefix: "reported " });
    return Object.fromEntries([...kept].map(([at, said]) => [at.slice("reported ".length), said]));
  }

  /** Begins taking `build`: every server that's ready and not on its pair prepares it, and the rest are skipped. */
  begin(state: State, build: string, presence: Record<string, Presence | null>, reported: Record<string, Reported>) {
    const published = state.build === build && state.published ? state.published : Date.now();
    const tries = state.build === build ? (state.tries ?? {}) : {};
    state.build = build;
    const known = Object.keys(reported).filter((key) => this.takes(key, presence, reported));
    const behind = known.filter((key) => !on(state, reported[key]));
    Object.assign(state, {
      network_update_status: "preparing",
      build,
      published,
      servers: behind.filter((key) => presence[key]?.server_ready).map((key) => ({ key })),
      skipped: behind.filter((key) => !presence[key]?.server_ready),
      waiting_for: null,
      now: false,
      went_back: [],
      tries,
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
    if (status === "up_to_date") {
      // One that missed an update is back. One behind tries again once its wait is over (`alarm`), not each time it
      // says what it is, which would fail as it did.
      const wentBack = new Set((state.went_back ?? []).map(({ server }) => server));
      const back =
        state.build &&
        Object.keys(reported).some((key) => this.takes(key, presence, reported) && !wentBack.has(key) && !on(state, reported[key]) && presence[key]?.server_ready);
      if (back) this.begin(state, state.build!, presence, reported);
      return;
    }
    const at = Date.now() - state.published!;
    for (const server of state.servers) {
      const said = reported[server.key] ?? {};
      const next = { ...said, version: said.next_build ?? undefined, program: said.next_program };
      if (server.prepared === undefined && said.server_update_status === "checked" && on(state, next)) server.prepared = at;
      if (server.started === undefined && on(state, said)) server.started = at;
      if (server.ready === undefined && on(state, said) && said.server_update_status === "up_to_date" && presence[server.key]?.server_ready) server.ready = at;
      // Back on the pair it was on, saying why: it went back, or couldn't prepare. It's behind, and the rest go on.
      if (server.begun && server.ready === undefined && said.server_update_status === "up_to_date" && !on(state, said) && said.went_back)
        this.wentBack(state, server.key, said.name, said.went_back);
    }
    // One that's gone, but for one restarting to take it, is caught up once it's back.
    const gone = state.servers.filter((server) => server.ready === undefined && !presence[server.key]?.connected && server.went === undefined);
    state.skipped.push(...gone.map((server) => server.key));
    state.servers = state.servers.filter((server) => !gone.includes(server));

    if (state.network_update_status === "preparing") {
      if (state.servers.some((server) => server.prepared === undefined && server.ready === undefined)) return;
      state.timings.prepared = at;
      state.network_update_status = "restarting";
      state.until = Date.now() + RESTARTING;
    }
    if (state.network_update_status === "restarting") {
      const restarting = state.servers.filter((server) => server.ready === undefined);
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

  /**
   * Moves the edge's own update on (`edge_update`), which never holds the servers': only once they're all up to date
   * on a build the edge is older than, one that's ready, on that build, and has an app that deploys the edge
   * (`deploys_edge`) deploys it, once for that build. What it says of how it went is kept, and said once. Gives the
   * servers whose report it changed.
   */
  updateEdge(state: State, presence: Record<string, Presence | null>, reported: Record<string, Reported>): string[] {
    const build = state.latest?.["server-code"] ?? state.build;
    if (!build || !state.edge || !newer(build, state.edge)) {
      if (state.edge_update?.status === "deploying") state.timings.edge = Date.now() - (state.published ?? Date.now());
      state.edge_update = null;
      return [];
    }
    // A newer build starts it afresh, once the servers are on it.
    if (state.edge_update && state.edge_update.build !== build) state.edge_update = null;
    const edge = state.edge_update;
    if (edge?.status === "deploying") {
      // Said by whichever server ran the app that deploys it, which runs where the network's apps do, not always on
      // the server told; one that goes, or never says, is failed by the deadline (`alarm`).
      const went = Object.values(reported).find((said) => said.edge_went_back?.build === build)?.edge_went_back;
      if (went) {
        Object.assign(edge, { status: went.setup ? "needs_setup" : "failed", why: went.why, until: undefined });
        edgeSaid(edge);
      }
      return [];
    }
    if (edge || state.network_update_status !== "up_to_date") return [];
    const [server] = Object.keys(reported)
      .filter((key) => presence[key]?.server_ready && reported[key].deploys_edge && reported[key].version === build)
      .sort();
    if (!server) return [];
    state.edge_update = { build, status: "deploying", server, until: Date.now() + DEPLOYING_EDGE };
    // What an earlier try said of this build is forgotten, so this one's is heard: the keys changed, to keep.
    const earlier = Object.keys(reported).filter((key) => reported[key].edge_went_back?.build === build);
    for (const key of earlier) reported[key] = { ...reported[key], edge_went_back: null };
    return earlier;
  }

  /** Takes `server` out of the update, saying why, as behind: the rest go on without it, and it tries again (`RETRIES`). */
  wentBack(state: State, server: string, name: string | undefined, why: string) {
    const tries = (state.tries?.[server] ?? 0) + 1;
    state.tries = { ...state.tries, [server]: tries };
    const retry_at = Date.now() + RETRIES[Math.min(tries, RETRIES.length) - 1];
    state.servers = state.servers.filter((taking) => taking.key !== server);
    state.went_back = [...(state.went_back ?? []).filter((back) => back.server !== server), { server, ...(name && { name }), why, retry_at }];
  }

  /** What every server taking part is to do now, by its key. */
  instructions(state: State, reported: Record<string, Reported>): Record<string, Instruction> {
    const told: Record<string, Instruction> = {};
    // The edge is deployed only while the servers are up to date: they never wait on it.
    if (state.network_update_status === "up_to_date")
      return state.edge_update?.status === "deploying" ? { [state.edge_update.server]: "deploy_edge" } : told;
    for (const server of state.servers) {
      if (server.ready !== undefined) continue;
      if (state.network_update_status === "preparing" && server.prepared === undefined) told[server.key] = "prepare";
      else if (server.went !== undefined && reported[server.key]?.server_update_status === "checked") told[server.key] = "go";
      else if (server.went !== undefined && reported[server.key]?.server_update_status !== "checked") continue;
      else if (state.network_update_status === "restarting" && state.waiting_for === "quiet") told[server.key] = "go_when_quiet";
      // A server that went back to the build before, or never prepared it, as one whose prepare was missed, prepares it.
      if (reported[server.key]?.server_update_status === "up_to_date" && !on(state, reported[server.key])) told[server.key] ??= "prepare";
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
      latest: state.latest ?? null,
      done: state.servers.filter((server) => server.ready !== undefined).length,
      of: state.servers.length,
      waiting_for: state.waiting_for ?? null,
      went_back: (state.went_back ?? []).map(({ server, why, retry_at }) => ({ server, name: name(server), why, retry_at: retry_at ?? null })),
      skipped: state.skipped.map(name),
      edge: state.edge ?? null,
      edge_update: state.edge_update ? { status: state.edge_update.status, build: state.edge_update.build, why: state.edge_update.why ?? null } : null,
      servers: this.instructions(state, reported),
      timings: {
        ...state.timings,
        ...Object.fromEntries(state.servers.map(({ key, prepared, went, started, ready }) => [name(key), { prepared, went, started, ready }])),
      },
    };
  }
}

/**
 * When the alarm is due: what's under way taken as not finishing (`until`), or, once nothing is, the first server
 * behind trying again.
 */
function alarmAt(state: State): number | null {
  const retries = state.network_update_status === "up_to_date" ? (state.went_back ?? []).flatMap(({ retry_at }) => (retry_at ? [retry_at] : [])) : [];
  const at = Math.min(state.until ?? Infinity, state.edge_update?.until ?? Infinity, ...retries);
  return at === Infinity ? null : at;
}

/**
 * Says what the update is now, as it changes, in the line Telemetry's Tail Worker keeps as a span
 * (`apps/telemetry/edge/worker.ts`): `network update`, from when its build was published to now, with its status,
 * build and how far it is, and as an error, which servers are behind and why, so Telemetry raises an alert.
 */
function said(state: State, status: ReturnType<NetworkUpdater["status"]>) {
  const wentBack = status.went_back.length ? status.went_back.map(({ name, why }) => `${name}: ${why}`).join("; ") : null;
  const attributes = {
    "one.update.status": status.network_update_status,
    ...(status.build && { "one.update.build": status.build }),
    "one.update.done": status.done,
    "one.update.of": status.of,
    "one.update.skipped": status.skipped.length,
    ...(status.waiting_for && { "one.update.waiting_for": status.waiting_for }),
    ...(status.went_back.length && { "one.update.went_back": status.went_back.map(({ name }) => name).join(", ") }),
  };
  console.log(JSON.stringify({ telemetry: { name: "network update", start: state.published ?? Date.now(), end: Date.now(), attributes, ...(wentBack && { error: wentBack }) } }));
}

/**
 * Whether a server that says `said` is on the pair the network takes it to (`OneCore.Release.target/2`): the latest
 * server build and the `one` that runs it, any `one` where it, or what's published, doesn't say. That `one` is the
 * build that first ran a server as it's run now (`OneCore.Builds`), so one as new or newer is on it: a `one` that
 * changed only the app around `one serve run` restarts no server.
 */
function on(state: State, said: Reported) {
  const program = said.program_name ? state.latest?.[said.program_name] : undefined;
  return said.version === (state.latest?.["server-code"] ?? state.build) && (!program || !said.program || !newer(program, said.program));
}

/** Whether the build `a` is newer than `b`: `<version>+<count>.<commit>`, by its commit's count. */
function newer(a: string, b: string) {
  const count = (build: string) => Number(/\+(\d+)\./.exec(build)?.[1] ?? 0);
  return count(a) > count(b);
}

/**
 * Says once, as Telemetry keeps it, that the edge didn't take its servers' build and why: `needs_setup` where the app
 * that deploys it can't (no token, or the Vault doesn't open on its server), and `failed` where the deploy went back.
 */
function edgeSaid(edge: EdgeUpdate) {
  const error = edge.status === "needs_setup" ? `The edge needs setting up to update itself: ${edge.why}` : `The edge didn't update to ${edge.build}: ${edge.why}`;
  console.log(JSON.stringify({ telemetry: { name: "edge update", start: Date.now(), end: Date.now(), attributes: { "one.update.status": edge.status, "one.update.build": edge.build }, error } }));
}
