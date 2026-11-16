// What the Home Screen's tools answer, which every screen here reads.

/** An app on a page: its page's address, name and icon, its plugin, and whether it was put on the page rather than installed for it. */
export interface App {
  app: string
  name: string
  icon: string
  plugin?: string
  put?: boolean
  subject?: string
}

/** A plugin with accounts, in Accounts: its name there (what its tools start with), its title and icon, and the page its sign-in shows on, if it has one. */
export interface Signs {
  plugin: string
  name: string
  title: string
  icon: string
  sign_in?: string
  subject?: string
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
export type Item = (App & { widget?: undefined; size?: undefined }) | (Widget & { app?: undefined })

/** A page, as `home-screen_page` answers it, and whose it is. */
export interface Page {
  who: string
  items: Item[]
  widgets: Widget[]
  accounts: Signs[]
  arranges?: boolean
  subject?: string
  name: string
}

/** An outside account, as a plugin's `accounts` lists it. */
export interface Account {
  account: string
  ok: boolean
  why?: string
}
