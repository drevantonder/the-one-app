/// The bridge every screen has (src/screen/bridge.js), as `one`.
type Bridge = {
  call: (tool: string, input?: object) => Promise<any>
  on: (event: string, run: () => void) => void
  plain: (err: unknown) => { title: string; next?: string; final?: boolean }
  largeTitle: (text: string) => string
}

export const bridge = () => (globalThis as unknown as { one: Bridge }).one
