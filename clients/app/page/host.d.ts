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
  for: string | undefined
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

/** Each command the page invokes: what it's given, and what it answers. */
export interface Commands {
  /** Where the window starts: here, on the page that signs its device in, or why the network couldn't be reached. */
  start(): { signIn?: string; unreached?: string; taps?: boolean }
  /** The files a server lists for `who`, and the plugins whose open apps are out of date. */
  apps(args: { who: string | undefined }): { resources: Resource[]; outdated: string[] }
  request(args: { view: string; method: string; params: Record<string, unknown> }): unknown
  refresh(args: { view: string }): void
  close(args: { view: string }): void
  client(): { device: string; network?: string; build?: string; server?: string }
  upload(file: ArrayBuffer, options: { headers: { plugin: string } }): { upload: string }
  enter(): void
  serve(args: { on: boolean }): string
  update(): string
  recovery_key(args: { who: string | undefined }): string
  recover(args: { key: string; who: string | undefined }): void
  visible(args: { onScreen: boolean }): void
  opened(args: { view: string; ms: number }): void
  /** Reports to Telemetry, as Telemetry's tool `record` takes a record, what went wrong here. */
  report(args: { record: Record<string, unknown> }): void
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
