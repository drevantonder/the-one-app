// What the window's page asks of where it runs: the app's Rust half (`host.js`, by Tauri's commands and events), or
// in a browser, the network's edge (`clients/web/host.js`). Both are this module, each typed by it, and `client.js`
// reads only this.

/** Asks where the page runs to do `command`. */
export declare const invoke: Invoke
/** Hears what where the page runs says. */
export declare const listen: Listen
/** Where an app's page is. */
export declare const address: Address
/** What an app's frame may do. */
export declare const sandbox: string
/** A web address an app opens. */
export declare const openLink: (url: string) => void

/** An open app, as `client.js` keeps it. */
export interface View {
  key: string
  app: string
  subject: string | undefined
  host: string
  frame: HTMLIFrameElement
  tapped?: number
  drawn?: boolean
  over?: boolean
  /** Whether one of its plugins has a new version it hasn't loaded yet. */
  stale?: boolean
}

/** A file a server lists for an app's page (MCP's `resources/list`). */
export interface Resource {
  uri: string
  mimeType?: string
  _meta?: { category?: string }
}

/**
 * Where a device is with its network, as the client says it (`Session` in core/client): opening, in, not in with the
 * code it's let in by, or its network not answering.
 */
export type Session =
  | { state: "opening" }
  | { state: "signedIn" }
  | { state: "signIn"; code?: { link: string; code: string }; why?: "denied" | "expired" | "removed" | null }
  | { state: "unreachable"; why: string }

/** What the page is told of the device's session: the session, its place in the order the page hears them in, and while it's signing in, where the window goes. */
export interface Told {
  seq: number
  session: Session
  link?: string | null
}

/** What the window shows of itself, one state at a time (`client.js`). */
export type Shown =
  | { state: "opening"; step: 0 | 1 | 2 }
  | { state: "unreachable" }
  | { state: "home" }
  | { state: "unlisted"; why: string }
  | { state: "tooOld" }
  | { state: "updating"; message?: string }
  | { state: "updateFailed"; why: string }

/** What moves what the window shows (`client.js`). */
export type Moved =
  | { type: "session"; session: Session }
  | { type: "listed" }
  | { type: "unlisted"; why: string }
  | { type: "tooOld" }
  | { type: "drawn" }
  | { type: "retry" }
  | { type: "update" }
  | { type: "updated"; message: string }
  | { type: "updateFailed"; why: string }

/** Each command the page invokes: what it's given, and what it answers. */
export interface Commands {
  /** Where the device is with its network as the window opens, and whether a phone hears taps on notifications. */
  start(): Told & { taps?: boolean }
  /** The files a server lists for `subject`, and the plugins whose open apps are out of date. */
  apps(args: { subject: string | undefined }): { resources: Resource[]; outdated: string[] }
  request(args: { view: string; method: string; params: Record<string, unknown> }): unknown
  refresh(args: { view: string }): void
  close(args: { view: string }): void
  client(): { device: string; network?: string; build?: string; target?: string; server?: string; push?: unknown }
  upload(file: ArrayBuffer, options: { headers: { plugin: string } }): { upload: string }
  /** Asks the network's address again now, which didn't answer. */
  retry(): void
  /** Signs this device out, once it's removed from its person's devices. */
  sign_out(): void
  /** Has a phone whose push failed try again, saying where it is once that try has ended. */
  push(): { message: string }
  serve(args: { on: boolean }): string
  update(): string
  recovery_key(args: { subject: string | undefined }): string
  recover(args: { key: string; subject: string | undefined }): void
  visible(args: { onScreen: boolean }): void
  opened(args: { view: string; ms: number }): void
  /** Reports to Telemetry, as Telemetry's tool `record` takes a record, what went wrong here. */
  report(args: { record: Record<string, unknown> }): void
  /** Turns the device's own back and forward gestures on or off. */
  navigation_gestures(args: { on: boolean }): void
  listen(): void
  tapped(args: { tapped: unknown }): void
  "plugin:notifications|set_click_listener_active"(args: { active: boolean }): void
}

/** Asks where the page runs to do `command`. */
export type Invoke = <Command extends keyof Commands>(command: Command, ...args: Parameters<Commands[Command]>) => Promise<ReturnType<Commands[Command]>>

/** Each event the page hears, and what it carries. */
export interface Events {
  /** A notification from a plugin's app was tapped: the plugin's name. */
  open: string
  /** What a server says changed: an open app's key, or `client` for the files listed. */
  changed: string
  /** Whether a server answered the last thing asked. */
  answering: boolean
  /** Where the device is with its network, as it moves. */
  session: Told
}

export type Listen = <Event extends keyof Events>(event: Event, handler: (event: { payload: Events[Event] }) => void) => unknown

/** Where an app's page is. */
export type Address = (view: Pick<View, "host" | "app">) => string

declare global {
  /** Tauri's API, which the app gives its window's page (`withGlobalTauri`). */
  interface Window {
    __TAURI__: {
      core: { invoke: Invoke; addPluginListener(plugin: string, event: string, handler: (payload: unknown) => void): Promise<unknown> }
      event: { listen: Listen }
    }
    /** The device's own back (`client.js`). */
    back: () => boolean
  }
}
