// What a page asks its client, and hears back: MCP Apps' JSON-RPC 2.0, as an MCP App's page posts it to its frame's
// parent and hears it in its `message` event, the same on every host. It's one of the two
// channels between a page's sandbox and its host; the other draws (renderer.ts).

/** What a native host gives an app's sandbox, as `globalThis.host`, for its calls. */
export interface Host {
  /** Takes a JSON-RPC message for the app's client, as a page on the DOM posts one to its frame's parent (MCP Apps):
   * `ui/initialize`, `tools/call` and the rest, which the client answers as it answers a page. */
  post(message: object): void
  /** Loads what another app's component module, at `url`, is drawn with (its app's `styles.json` beside it), before
   * `component` imports it: a host whose own loader loads modules does it as it loads one, and has none of this. */
  component?(url: string): Promise<void>
}

/** What the UI library gives the host, as `globalThis.ui` (with renderer.ts's `Events`), once it's loaded. */
export interface Sandbox {
  /** A JSON-RPC message from the client, as a page on the DOM gets one in its `message` event: an answer to what it
   * posted, or a notification (`notifications/one/changed`, `ui/notifications/host-context-changed`). */
  message(message: object): void
}
