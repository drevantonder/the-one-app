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
/** Whether a page whose app was updated under it asks its person to reload it ("Updated · Reload"), as a web app in a browser does, or loads again by itself once it isn't in use, as a desktop app or a phone's does. */
export declare const asksToReload: boolean
/** A web address an app opens. */
export declare const openLink: (url: string) => void
/** Plays a haptic an app asked for (`one/haptic`), where the device has them on. */
export declare const haptic: (kind: Haptic) => Promise<unknown> | void
/** Has what's the window's own and not its page (its bars) follow light or dark, or with `null`, the device. */
export declare const setTheme: (theme: "light" | "dark" | null) => Promise<unknown> | void

/** A haptic, by iOS's name: an impact, a selection, or a notification's outcome. */
export type Haptic = "light" | "medium" | "heavy" | "rigid" | "soft" | "selection" | "success" | "warning" | "error"

/** A place in a page, as `getBoundingClientRect` says it, and how round its corners are, where that's said. */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
  radius?: number
}

/** An open app, as `client.js` keeps it. */
export interface View {
  key: string
  entrypoint: string
  subject: string | undefined
  host: string
  frame: HTMLIFrameElement
  tapped?: number
  /** Whether something of its own is open, which the device's back shuts first, and whether any of it is drawn over it (a sheet, a menu), where its UI library says. */
  over?: boolean
  layer?: boolean
  /** Whether it's on its first screen, with nothing of its own to go back to, where its UI library says. */
  first?: boolean
  /** Whether the device's status bar shows while it does: unless it asked otherwise (`client({ statusBar })`). */
  statusBar?: boolean
}

/** What moves an open app's page (`client.js`): it drew, didn't, showed an error, was loaded against a version that's
 * gone, went off screen, its window may not be in use, it was opened again, or its person pressed Reload or Try Again. */
export type Paged =
  | { type: "drawn" }
  | { type: "errored" }
  | { type: "stale" }
  | { type: "away" }
  | { type: "unused" }
  | { type: "seen" }
  | { type: "opened" }
  | { type: "reload" }
  | { type: "retry" }
  | { type: "unopened"; why: string }

/** A file a server lists for an app's page (MCP's `resources/list`). */
export interface Resource {
  uri: string
  name?: string
  mimeType?: string
  /** An app entrypoint's icon, as a `data:` address. */
  icons?: { src: string }[]
  /** An app entrypoint's category, and whether its app is trusted. */
  _meta?: { category?: string; trusted?: boolean }
}

/**
 * Where a device is with its network, as the client says it (`State` in core/client): opening, in, or not in with
 * the code it's let in by.
 */
export type Session =
  | { state: "opening" }
  | { state: "signedIn" }
  | { state: "signIn"; code?: { link: string; code: string }; why?: "denied" | "expired" | "removed" | null }

/** Where a phone or a browser is with being woken by its notifications (`Push` in apps/notifications/client/native), and an iPhone's browser that can be only from its Home Screen. */
export type Push = { state: "notAsked" | "denied" | "registering" | "registered" | "homeScreen" } | { state: "failed"; why: string }

/** Whether the client reaches its network, beside its session (`Connection` in core/client). */
export type Connection = "connecting" | "online" | "offline" | "reconnecting"

/** What the page is told of the device's session: the session, its place in the order the page hears them in, and while it's signing in, where the window goes. */
export interface Told {
  seq: number
  session: Session
  link?: string | null
}

/** Where the device is with its network, as the window hears it (`State` in core/client): its session, as it's told, and whether its network answers, or its servers say it's behind them. */
export interface State extends Told {
  connection: Connection | "behind"
}

/** What the window knows as it moves (`client.js`): the device's session and whether its network answers, as they
 * were last told, whether the launcher is loading, whether an update for this device is ready, and what its state
 * says. */
export interface Context {
  told?: Told
  connection: Connection
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

/** What moves the window's move between its launcher and its apps (`client.js`'s `moveMachine`): an app to show,
 * coming back or going on, and the icon it comes out of or goes back into; one the device has already drawn the move
 * to; the spring at rest; the window covering every app; and a finger taking the move, back or with `home` all the
 * way to the Home Screen (the rail's line), moving it and letting go, at `velocity`, in moves a second, and `end`,
 * where it ends when the gesture decided that itself. */
export type Shown =
  | { type: "show"; to: View; back: boolean; icon?: Rect; tapped: number }
  | { type: "popped"; to: View; tapped: number }
  | { type: "settled" }
  | { type: "freeze" }
  | { type: "grab"; home?: boolean }
  | { type: "drag"; p: number }
  | { type: "release"; velocity: number; end?: 0 | 1 }

/** What moves the window (`client.js`). */
export type Moved =
  | Extract<Shown, { type: "show" | "popped" | "grab" | "drag" | "release" }>
  | { type: "session"; told: Told }
  | { type: "connection"; connection: Connection }
  | { type: "listed" }
  | { type: "none"; only?: string }
  | { type: "tooOld" }
  | { type: "unchecked"; why: string }
  | { type: "unnative"; why: string }
  | { type: "drawn"; key: string }
  | { type: "failed"; key: string; why: string }
  | { type: "load"; view: View }
  | { type: "page"; key: string; paged: Paged }
  | { type: "away" }
  | { type: "unused" }
  | { type: "seen" }
  | { type: "openAppStore" }
  | { type: "retry" }
  | { type: "update" }
  | { type: "updateReady"; ready: boolean }
  | { type: "link"; link: string }

/** Each command the page invokes: what it's given, and what it answers. */
export interface Commands {
  /** Where the device is with its network as the window opens, whether its network answers, whether a phone hears taps on notifications, whether each app opens in a window of its own (a computer's), which app this window is if it's an app's own, where the window's title bar is over the page, which keeps it clear, if it's known yet (`title_bar` says it as it changes), and the Colour Mode chosen on the device, if one has been. */
  start(): { state: State; taps?: boolean; open?: string | null; windows?: boolean; window?: { entrypoint: string; subject?: string; name?: string | null } | null; titleBar?: TitleBar; theme?: "light" | "dark" | null; unnative?: string | null }
  /** The files a server lists for `subject`, and the apps whose open apps are out of date. */
  apps(args: { subject: string | undefined }): { resources: Resource[]; outdated: string[] }
  request(args: { view: string; method: string; params: Record<string, unknown> }): unknown
  refresh(args: { view: string }): void
  close(args: { view: string }): void
  client(): { device: string; network?: string; build?: string; target?: string; server?: string; push?: Push; settings?: boolean; computerUse?: unknown }
  upload(file: ArrayBuffer, options: { headers: { app: string } }): { upload: string }
  /** Asks the network again now, which didn't answer. */
  retry(): Promise<void>
  /** Signs this device out, once it's removed from its person's devices. */
  sign_out(): void
  /** Has a phone whose push failed try again, saying where it is once that try has ended, or a browser turn its notifications on, as its person taps to. */
  push(): { message: string }
  serve(args: { on: boolean }): string
  update(): string
  recovery_key(args: { subject: string | undefined }): string
  recover(args: { key: string; subject: string | undefined }): void
  visible(args: { onScreen: boolean }): void
  /** Whether the window holds something its person typed and hasn't sent, which a new build waits for. */
  editing(args: { editing: boolean }): void
  opened(args: { view: string; ms: number }): void
  /** Reports to Telemetry, as Telemetry's tool `record` takes a record, what went wrong here. */
  report(args: { record: Record<string, unknown> }): void
  /** Turns the device's own back and forward gestures on or off. */
  navigation_gestures(args: { on: boolean; home: boolean }): void
  /** Shows or hides an iPhone's status bar, as the app that's showing asks; a device with none keeps what it has. */
  status_bar(args: { shown: boolean }): void
  /** Light or dark for the window's own bars, or with `null`, as the device is. */
  theme(args: { theme: "light" | "dark" | null }): void
  listen(): void
  /** Opens an app in a window of its own, at `link`, or brings it forward there if it's open. */
  open_window(args: { entrypoint: string; subject: string | undefined; link: string; icon?: [number, number, number, number] }): void
  /** Brings the Home Screen's window forward. */
  front(): void
  /** Makes a computer's launcher panel as tall as `height`, in points, or its tallest without, its top where it is. */
  resize(args: { height?: number }): void
  /** Moves the window with the pointer, held down where it has no title bar of its own to hold. */
  "plugin:window|start_dragging"(): void
  /** Does what a double-click on a Mac's title bar does, as the Mac is set: zoom, minimize or nothing. */
  title_bar_double_click(): void
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
  /** The open apps loaded against a version of an app, or a build of the UI library, a server says is gone, as their addresses name them. */
  stale: string[]
  /** Where the device is with its network, as it moves. */
  state: State
  /** The Colour Mode chosen on this device, in any of its windows: `null` for as the device is. */
  theme: "light" | "dark" | null
  /** Where this window's title bar is over its page, as that changes. */
  title_bar: TitleBar
}

/** Where a window's title bar is over its page, which keeps it clear, in CSS pixels: its height, where what goes at its start goes, after the window's own controls there (a Mac's traffic lights, Linux's buttons where the desktop puts them on the left), and how much of its end the window's own controls take (Windows' caption buttons, Linux's on the right). */
export interface TitleBar {
  height: number
  start: number
  end: number
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
    /** Android's back gesture coming in from the edge, as far in as `progress`, and let go short (`client.js`). */
    backStarted: () => void
    backProgressed: (progress: number) => void
    backCancelled: () => void
    /** An iPhone's swipe in from the edge taking the move, moving it as far as `p`, and letting it go (`client.js`). */
    grab: () => void
    drag: (p: number) => void
    release: (velocity: number, end?: 0 | 1) => void
    /** A tap on an iPhone's status bar (`client.js`). */
    scrollToTop: () => void
  }
  /** Where Android's app hears whether something is open over the app that's showing (`MainActivity`). */
  var androidBack: { postMessage(message: string): void } | undefined
  /** Where Android's app plays a haptic, by its name (`MainActivity`). */
  var androidHaptic: { postMessage(message: string): void } | undefined
  /** Where Android's app opens its own Settings page, `accessibility` or `app` (`MainActivity`). */
  var androidSettings: { postMessage(message: string): void } | undefined
  /** Where Android's app hears where the rail's line is, `left,top,right,bottom` in the page's pixels, or "" while
   * it's not there, which its back gesture leaves to it (`MainActivity`). */
  var androidEdge: { postMessage(message: string): void } | undefined
  /** Where Android's app puts an app on the device's home screen, its shortcut as JSON (`MainActivity`). */
  var androidPin: { postMessage(message: string): void } | undefined
  /** Where Android's app hears the apps used lately and those installed, as JSON, for its shortcuts (`MainActivity`). */
  var androidShortcuts: { postMessage(message: string): void } | undefined
}
