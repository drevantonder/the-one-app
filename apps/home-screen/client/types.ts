// What the Home Screen's tools answer, which every screen here reads.

/** An app: its page's address, name, icon, app and category, its `badge`, and its `subject`: `"user"` if it's opened for whoever
 * opens it, or the group it's opened for, where a group's install is in a user's App Library. */
export interface Entrypoint {
  entrypoint: string
  name: string
  icon: string
  app: string
  category: string
  subject?: "user" | `groups/${string}`
  /** The tool whose answer's `count` is the red count on its icon, while it's more than 0. */
  badge?: string
}

/** A widget on a page, or one that could go on one: its tool, the sizes it comes in, and the app pressing it opens. */
export interface Widget {
  widget: string
  name: string
  size?: Size
  sizes: Size[]
  opens: Entrypoint | null
}

export type Size = "2x2" | "4x2" | "4x4"

/** Something on a page: an app or a widget. */
export type Item = (Entrypoint & { widget?: undefined; size?: undefined }) | (Widget & { entrypoint?: undefined })

/** An app as an App Library has it: an entrypoint of its, or an app with none (`entrypoint` null), which opens its
 * section in Settings; its app's name, and who its app is installed for, by what name, and whether you may remove it. */
export interface Listed extends Omit<Entrypoint, "entrypoint"> {
  entrypoint: string | null
  title: string
  installed: { for: string; name: string; removes: boolean }
}

/** A category of an App Library, as it shows it: its apps by name. */
export interface Category {
  category: string
  entrypoints: Listed[]
}

/** A Home Screen, as `home-screen_pages` answers it: whose it is, its pages, each what's on it and whether it's hidden,
 * its App Library and the apps that open on its pages. */
export interface Pages {
  who: string
  arranges: boolean
  pages: { items: Item[]; hidden: boolean }[]
  library: Category[]
  widgets: Widget[]
  apps: { app: string; name: string }[]
}

/** Where this page loads an icon, from its address, `ui://icons/<sha256>/<file>`: beside its app's own files, at that
 * hash, where the client serves every app's files by version (`<app>@<version>/`), so it's asked for once. */
export const src = (icon: string) => icon.replace(/^ui:\/\/([^/]+)\/([^/]+)\//, "../../$1@$2/")
