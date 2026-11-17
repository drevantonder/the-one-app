// What the Home Screen's tools answer, which every screen here reads.

/** An app: its page's address, name, icon, plugin and category, and `subject: "user"` if it's opened for whoever opens it. */
export interface App {
  app: string
  name: string
  icon: string
  plugin: string
  category: string | null
  subject?: "user"
}

/** A plugin with accounts, which Accounts lists. Not an app: its name there (what its tools start with), its own title and icon, and its `sign_in` tool's page, if it has one. */
export interface PluginWithAccounts {
  plugin: string
  name: string
  title: string
  icon: string
  sign_in?: string
  subject?: string
}

/** A widget on a page, or one that could go on one: its tool, the sizes it comes in, and the app pressing it opens. */
export interface Widget {
  widget: string
  name: string
  size?: Size
  sizes: Size[]
  opens: App | null
}

export type Size = "2x2" | "4x2" | "4x4"

/** Something on a page: an app or a widget. */
export type Item = (App & { widget?: undefined; size?: undefined }) | (Widget & { app?: undefined })

/** A category of an App Library, as it shows it: its apps by name. Other's `category` is null. */
export interface Category {
  category: string | null
  name: string
  apps: App[]
}

/** A Home Screen, as `home-screen_pages` answers it: whose it is, its pages, each what's on it and whether it's hidden, and its App Library. */
export interface Pages {
  who: string
  arranges: boolean
  pages: { items: Item[]; hidden: boolean }[]
  library: Category[]
  widgets: Widget[]
  plugins_with_accounts: PluginWithAccounts[]
}

/** An outside account, as a plugin's `accounts` lists it. */
export interface Account {
  account: string
  ok: boolean
  why?: string
}
