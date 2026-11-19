// Whose Home Screen it is, what each owner's pages are as the tools answer them, and what's done to them: opening an
// app, putting one on a page or taking it off, keeping what was edited.
import { createEffect, createMemo, createSignal, refresh, untrack } from "solid-js"
import { call, changed, confirm, open, type Rect, toast } from "ui"
import { type Editing, fitted, type Page, shown } from "./machine.ts"
import { type Entrypoint, type Category, type Item, type Listed, type Pages, type Widget as WidgetItem } from "./types.ts"
import { state, setKept } from "./store.ts"

export const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

/** An owner: whose Home Screen it is, by the name a person reads, and what `home-screen_pages` answers for them. */
export interface Owner extends Pages {
  key: string
  subject?: string
  name: string
}

/// You, everyone, then each of your groups, in the order they're listed. A network with nothing that keeps groups
/// has none. Yours and everyone's are asked for beside the groups, which they don't wait on.
export const owners = createMemo(async () => {
  const pages = (who: { subject?: string; name: string }) => json<Pages>("home-screen_pages", {}, who.subject).then((pages): Owner => ({ ...pages, ...who, key: who.subject ?? "you" }))
  const groups = json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => [])
  const [yours, everyone, theirs] = await Promise.all([pages({ name: "You" }), pages({ subject: "network", name: "Everyone" }), groups.then((groups) => Promise.all(groups.map(({ group, called }) => pages({ subject: group, name: called }))))])
  return [yours, everyone, ...theirs]
})
changed(() => refresh(owners))

/// The red count on an app's icon, by who it's for and its badge tool (`badge`): what its app says waits for whoever's
/// looking, asked again with everything else as anything changes. One that couldn't be asked has none.
const [badges, setBadges] = createSignal<Record<string, number>>({})

/// Who an app is opened for, as `launch` opens it, and so who its badge is asked for.
const opensFor = (entrypoint: Entrypoint, owner: Owner) => (entrypoint.subject === "user" ? undefined : (entrypoint.subject ?? owner.subject))

export const badgeOf = (entrypoint: Entrypoint, owner: Owner) => (entrypoint.badge ? badges()[`${opensFor(entrypoint, owner) ?? ""} ${entrypoint.badge}`] : undefined)
createEffect(owners, (all) => {
  const asked = new Map<string, Promise<number>>()
  for (const owner of all)
    for (const item of [...owner.pages.flatMap((page) => page.items), ...owner.library.flatMap((category) => category.entrypoints)]) {
      const entrypoint = item as Entrypoint
      if (!entrypoint.badge) continue
      const subject = opensFor(entrypoint, owner)
      const key = `${subject ?? ""} ${entrypoint.badge}`
      if (!asked.has(key)) asked.set(key, json<{ count?: number }>(entrypoint.badge, {}, subject).then(({ count }) => count ?? 0, () => 0))
    }
  Promise.all([...asked].map(async ([key, count]) => [key, await count] as const)).then((counted) => setBadges(Object.fromEntries(counted)))
})

/// The apps of an App Library whose names have `words` in them: with a screen of their own, or `background`. Those
/// whose name starts with them come first, then those with a word that does, then the rest, each A to Z, as
/// Raycast's root search ranks what it finds.
export const findIn = (library: Category[], words: string, background: boolean) => {
  const typed = words.trim().toLowerCase()
  const rank = (name: string) => (name.toLowerCase().startsWith(typed) ? 0 : name.toLowerCase().split(/[\s&-]+/).some((word) => word.startsWith(typed)) ? 1 : 2)
  return library
    .flatMap((category) => category.entrypoints)
    .filter((listed) => !listed.entrypoint === background && listed.name.toLowerCase().includes(typed))
    .sort((a, b) => rank(a.name) - rank(b.name) || a.name.localeCompare(b.name))
}

/// Everything an App Library has whose name has `words` in it: the apps with a screen of their own, then the
/// background apps, each in `findIn`'s order, which is the order the launcher's search chooses among them in.
export const findAll = (library: Category[], words: string) => [...findIn(library, words, false), ...findIn(library, words, true)]

/** Where an icon or a widget is drawn, with how round its corners are there, which an app grows out of. */
export type From = Rect & { radius: number }

/// Opens an app for whoever its owner is, for the group whose install it is, or for its person where it's about
/// whoever opens it. `behind` only starts it, as a finger lands on it. It grows out of its icon or widget, `from`, as
/// an iPhone's does.
export const launch = (entrypoint: Entrypoint, owner: Owner, behind?: boolean, from?: From) =>
  open(entrypoint.entrypoint, { subject: entrypoint.subject === "user" ? undefined : (entrypoint.subject ?? owner.subject), behind, from })

/// An owner's pages as they're kept: each with what's on it and whether it's hidden, to edit.
export const draft = (owner: Owner, index: number): Editing => ({ owner: index, pages: fitted(owner.pages.map((page) => ({ items: [...page.items], hidden: page.hidden }))) })

/// Taking an item off a Home page asks first, as an iPhone's does: an app stays in the App Library, and a widget's app
/// and data stay.
export const removal = (item: Item): Parameters<typeof confirm>[0] =>
  item.widget ? { heading: "Remove Widget?", action: "Remove", destructive: true } : { heading: `Remove “${item.name}”?`, text: "It stays in your App Library.", action: "Remove from Home Screen" }

/// The pages of the owner Home is on, to edit, as they're kept, for what's lifted before editing began.
export const pending = () => {
  const at = state().context.at.owner
  return draft(untrack(owners)![at], at)
}

/// An app of an App Library as an entrypoint, which it opens, or nothing, for an app with none.
export const opens = (listed: Listed): Entrypoint | undefined => (listed.entrypoint ? (listed as Entrypoint) : undefined)

/// Opens an app of an App Library: its entrypoint, or for an app with none, the App Store at its page, by a link
/// (`one:app-store#app=…`), growing out of its icon as any app does.
export const press = (listed: Listed, owner: Owner, behind?: boolean, from?: From) => {
  if (opens(listed)) return launch(opens(listed)!, owner, behind, from)
  const store = untrack(owners)?.[0]?.store
  if (behind || !store) return
  open(`${store}#app=${encodeURIComponent(listed.app)}`, { from })
}

/// Who a call about an install is for: a group or the network, or for a user's own, nobody said.
export const subjectOf = (installed: string) => (installed.startsWith("users/") ? undefined : installed)

export const and = (words: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(words)

/// The app something on a page opens: an app's own, or a widget's app.
export const opener = (item: Item): Entrypoint | undefined => (item.widget ? ((item as WidgetItem).opens ?? undefined) : (item as Entrypoint))

/// Whether an app opens on an owner's pages: it's installed for them, or for everyone.
export const installedFor = (item: Entrypoint, owner: Owner) => owner.apps.some((one) => one.app === item.app)

/// Whether an app can be added to an owner's pages from the App Library: whoever's adding arranges them, it opens
/// there, and it isn't on them already, even on a page they hide.
export const addable = (item: Item, owner: Owner) =>
  !item.widget && owner.arranges && installedFor(item as Entrypoint, owner) && !owner.pages.some((page) => page.items.some((other) => !other.widget && other.entrypoint === theirs(item as Entrypoint, owner)))

/// Puts an app last on an owner's last page, as a phone's App Library does.
export function addToHome(owner: Owner, at: number, item: Item) {
  const editing = draft(owner, at)
  const last = shown(editing.pages).at(-1)
  const pages = last === undefined ? [...editing.pages, { items: [item] }] : editing.pages.map((page, index) => (index === last ? { ...page, items: [...page.items, item] } : page))
  keep([{ ...editing, pages: fitted(pages) }])
}

/// Takes what's on an owner's page off it, once its alert says that's what's wanted: `page` is which of the pages shown.
export async function removeFromHome(owner: Owner, at: number, page: number, index: number, item: Item) {
  const editing = draft(owner, at)
  if (!(await confirm(removal(item)))) return
  const kept = shown(editing.pages)[page]
  const pages = editing.pages.map((one, which) => (which === kept ? { ...one, items: one.items.toSpliced(index, 1) } : one))
  keep([{ ...editing, pages: pages.filter((one) => one.items.length) }])
}

/// An app's entrypoint as an owner's pages name it: by the name its app is installed by for them, which may not be
/// the one it has where it was taken from.
export const theirs = (item: Entrypoint, owner: Owner) => {
  const name = owner.apps.find((one) => one.app === item.app)?.name
  return name ? `${name}/${item.entrypoint.slice(item.entrypoint.indexOf("/") + 1)}` : item.entrypoint
}

/// Who an install is for, as a person reads it in a sentence: you, Everyone, or a group by its name.
export const whose = (installed: string) => {
  const owner = untrack(owners)?.find((one) => one.who === installed)
  return !owner ? installed : owner.subject ? owner.name : "you"
}

/// Keeps owners' pages as they were edited, each that changed, for everyone who sees them, and says why if they
/// couldn't be.
export async function keep(edited: Editing[]) {
  const all = untrack(owners)!
  // Pages as `arrange` takes them, each app by the name it's installed by for the owner, and an empty page gone.
  const asked = (pages: Page[], owner: Owner) =>
    pages.filter((page) => page.items.length).map((page) => ({ items: page.items.map((item) => (item.widget ? { widget: item.widget, size: item.size } : { entrypoint: theirs(item as Entrypoint, owner) })), ...(page.hidden ? { hidden: true } : {}) }))
  const changed = edited.filter((one) => JSON.stringify(asked(one.pages, all[one.owner])) !== JSON.stringify(asked(all[one.owner].pages, all[one.owner])))
  setKept(changed)
  await Promise.all(
    changed.map((one) =>
      call("home-screen_arrange", { pages: asked(one.pages, all[one.owner]) }, { subject: all[one.owner].subject }).catch((error) => toast(error as Error)),
    ),
  )
  refresh(owners)
}

/// Who an owner is in a sentence: you, Everyone, or a group by its name.
export const named = (owner: Owner) => (owner.subject ? owner.name : "you")
