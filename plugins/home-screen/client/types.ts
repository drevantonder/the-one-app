// What the Home Screen's tools answer, which every screen here reads.

/** An app on a page, or in its Accounts folder: its page's address, name and icon, its plugin, and whether it was put on the page rather than installed for it. */
export interface App {
  app: string
  name: string
  icon: string
  plugin?: string
  put?: boolean
  folder?: App[]
  for?: string
}

/** A widget on a page, or one a page's plugins have: its tool, the sizes it comes in, and what pressing it opens. */
export interface Widget {
  widget: string
  name: string
  size?: Size
  sizes: Size[]
  opens?: string
  plugin?: string
}

export type Size = "2x2" | "4x2" | "4x4"

/** Something on a page: an app or a widget. */
export type Item = (App & { widget?: undefined; size?: undefined }) | (Widget & { app?: undefined; folder?: undefined })

/** A page, as `home-screen_page` answers it, and whose it is. */
export interface Page {
  who: string
  items: Item[]
  widgets: Widget[]
  arranges?: boolean
  for?: string
  name: string
}

/** An outside account, as a plugin's `accounts` lists it. */
export interface Account {
  account: string
  ok: boolean
  about?: string
  problem?: string
}
