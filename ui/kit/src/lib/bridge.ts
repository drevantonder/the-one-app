/// The bridge every screen has (ui/bridge.js), as `one`.
type Bridge = {
  call: (tool: string, input?: object) => Promise<any>
  on: (event: string, run: () => void) => void
  plain: (err: unknown) => { title: string; next?: string; final?: boolean; mends?: boolean; notAdded?: boolean }
  open: (screen: string, params?: object, app?: string) => Promise<unknown>
  app: string
  largeTitle: (text: string) => string
}

export const bridge = () => (globalThis as unknown as { one: Bridge }).one
