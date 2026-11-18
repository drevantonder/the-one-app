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
/** Plays a haptic an app asked for (`one/haptic`), where the device has them on. */
export declare const haptic: (kind: Haptic) => Promise<unknown> | void
/** Has what's the window's own and not its page (its bars) follow light or dark, or with `null`, the device. */
export declare const setTheme: (theme: "light" | "dark" | null) => Promise<unknown> | void

/** A haptic, by iOS's name: an impact, a selection, or a notification's outcome. */
export type Haptic = "light" | "medium" | "heavy" | "rigid" | "soft" | "selection" | "success" | "warning" | "error"

/** A place in a page, as `getBoundingClientRect` says it. */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** An open app, as `client.js` keeps it. */
export interface View {
  key: string
  entrypoint: string
  subject: string | undefined
  host: string
  frame: HTMLIFrameElement
  tapped?: number
  over?: boolean
  /** Where in the window the icon it was opened from was, which it goes back into as it closes. */
  from?: Rect
  /** Whether the device's status bar shows while it does: unless it asked otherwise (`client({ statusBar })`). */
  statusBar?: boolean
}

/** What moves an open app's page (`client.js`): it drew, didn't, showed an error, was loaded against a version that's
 * gone, went off screen, was opened again, or its person pressed Reload or Try Again. */
export type Paged =
  | { type: "drawn" }
  | { type: "errored" }
  | { type: "stale" }
  | { type: "away" }
  | { type: "seen" }
  | { type: "opened" }
  | { type: "reload" }
  | { type: "retry" }

/** A file a server lists for an app's page (MCP's `resources/list`). */
export interface Resource {
  uri: string
  name?: string
  mimeType?: string
  _meta?: { category?: string }
}

/**
 * Where a device is with its network, as the client says it (`Session` in core/client): opening, in, or not in with
 * the code it's let in by.
 */
export type Session =
  | { state: "opening" }
  | { state: "signedIn" }
  | { state: "signIn"; code?: { link: string; code: string }; why?: "denied" | "expired" | "removed" | null }

/** Whether the client reaches its network, beside its session (`Connection` in core/client). */
export type Connection = "connecting" | "online" | "offline"

/** What the page is told of the device's session: the session, its place in the order the page hears them in, and while it's signing in, where the window goes. */
export interface Told {
  seq: number
  session: Session
  link?: string | null
}

/** What the window knows as it moves (`client.js`): the device's session and whether its network answers, as they
 * were last told, whether its network has not answered since the window opened, whether the launcher is loading,
 * whether an update for this device is ready, and what its state says. */
export interface Context {
  told?: Told
  connection: Connection
  lost: boolean
  starting: boolean
  updateReady: boolean
  /** What a `one:` link opens once the launcher has drawn: `<app>[/<page>][#<fragment>]`. */
  link?: string
  /** Who may install a Home Screen for everyone, where it isn't this person: "Only Andre can install a Home Screen for Everyone". */
  only?: string
  why?: string
  message?: string
  /** Whether a server refused this client for being behind it, until one talks with it again. */
  behind?: boolean
}

/** What moves the window (`client.js`). */
export type Moved =
  | { type: "session"; told: Told }
  | { type: "connection"; connection: Connection }
  | { type: "listed" }
  | { type: "none"; only?: string }
  | { type: "tooOld" }
  | { type: "drawn"; key: string }
  | { type: "failed"; key: string; why: string }
  | { type: "load"; view: View }
  | { type: "page"; key: string; paged: Paged }
  | { type: "away" }
  | { type: "seen" }
  | { type: "openAppStore" }
  | { type: "retry" }
  | { type: "update" }
  | { type: "updateReady"; ready: boolean }
  | { type: "link"; link: string }

/** Each command the page invokes: what it's given, and what it answers. */
export interface Commands {
  /** Where the device is with its network as the window opens, whether its network answers, and whether a phone hears taps on notifications. */
  start(): Told & { connection: Connection; taps?: boolean; open?: string | null }
  /** The files a server lists for `subject`, and the apps whose open apps are out of date. */
  apps(args: { subject: string | undefined }): { resources: Resource[]; outdated: string[] }
  request(args: { view: string; method: string; params: Record<string, unknown> }): unknown
  refresh(args: { view: string }): void
  close(args: { view: string }): void
  client(): { device: string; network?: string; build?: string; target?: string; server?: string; push?: unknown }
  upload(file: ArrayBuffer, options: { headers: { app: string } }): { upload: string }
  /** Asks the network again now, which didn't answer. */
  retry(): Promise<void>
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
  /** Shows or hides an iPhone's status bar, as the app that's showing asks; a device with none keeps what it has. */
  status_bar(args: { shown: boolean }): void
  /** Light or dark for the window's own bars, or with `null`, as the device is. */
  theme(args: { theme: "light" | "dark" | null }): void
  listen(): void
  tapped(args: { tapped: unknown }): void
  "plugin:notifications|set_click_listener_active"(args: { active: boolean }): void
  "plugin:haptics|impact_feedback"(args: { style: string }): void
  "plugin:haptics|selection_feedback"(): void
  "plugin:haptics|notification_feedback"(args: { type: string }): void
}

/** Asks where the page runs to do `command`. */
export type Invoke = <Command extends keyof Commands>(command: Command, ...args: Parameters<Commands[Command]>) => Promise<ReturnType<Commands[Command]>>

/** Each event the page hears, and what it carries. */
export interface Events {
  /** A notification from an app's entrypoint was tapped: the app's name. */
  open: string
  /** What a server says changed: an open app's key, or `client` for the files listed. */
  changed: string
  /** Whether the client reaches its network, as it moves. */
  connection: Connection
  /** The open apps loaded against a version of an app, or a build of the UI library, a server says is gone, as their addresses name them. */
  stale: string[]
  /** Where the device is with its network, as it moves. */
  session: Told
  /** Why a server refused this client for being on a version before its own, once one has. */
  behind: string
}

export type Listen = <Event extends keyof Events>(event: Event, handler: (event: { payload: Events[Event] }) => void) => unknown

/** Where an app's page is. */
export type Address = (view: Pick<View, "host" | "entrypoint" | "subject">) => string

declare global {
  /** XState, which the window's page is given as its UMD build (`xstate.umd.min.js`). */
  var XState: typeof import("xstate")
  /** Tauri's API, which the app gives its window's page (`withGlobalTauri`). */
  interface Window {
    __TAURI__: {
      core: { invoke: Invoke; addPluginListener(app: string, event: string, handler: (payload: unknown) => void): Promise<unknown> }
      event: { listen: Listen }
    }
    /** The device's own back (`client.js`). */
    back: () => boolean
    /** A tap on an iPhone's status bar (`client.js`). */
    scrollToTop: () => void
  }
  /** Where Android's app hears whether something is open over the app that's showing (`MainActivity`). */
  var androidBack: { postMessage(message: string): void } | undefined
  /** Where Android's app plays a haptic, by its name (`MainActivity`). */
  var androidHaptic: { postMessage(message: string): void } | undefined
}
