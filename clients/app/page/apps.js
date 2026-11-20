// The files this user's apps list, and what they say: the launcher, the App Store, each app's name and icon.
import { invoke } from "./host.js"
import { appTitle } from "./chrome.js"
import { on, open, own, page, phone, shown } from "./client.js"
import { shortcuts } from "./shortcuts.js"
import { exception } from "./util.js"

/// The launcher's page, as this user's apps list it, once they're listed: `<app>/<page>`. And the App Store's,
/// which installs one where there's none: the app in the category `app-store`, if there is one.
export let launcher = ""
export let appStore = ""
/// Where an app's settings are: the app in the category `settings`, if there is one, which a link to an app with no
/// page of its own opens at that app.
export let settings = ""
/// The runtime version each app's native page needs, by its entrypoint, as its `runtimeVersion` says.
/** @type {Map<string, number>} */
export let needs = new Map()
/// Each app's first app, by the app's name, among this user's: what a notification from it, or a `one:` link to it, opens.
/** @type {Map<string, string>} */
export let first = new Map()
/// Every file of every app listed, by `<app>/<path>`: what a link to a page can open.
/** @type {Set<string>} */
export let files = new Set()
/// Each app's name and icon, by its page, as this user's apps list them.
/** @type {Map<string, string>} */
export let names = new Map()
/** @type {Map<string, string>} */
export let icons = new Map()
/// Who this client has listed files for: its user (`undefined`), and each group or the network an app was opened
/// for.
/** @type {Set<string | undefined>} */
export const listed = new Set()
/// The apps each of those lists says are trusted, by who it's for, as each was last listed. A page of any other app,
/// one installed untrusted or not listed yet, asks only what its app's server code may, behind its sandbox, for
/// whoever it was opened for, and changes nothing of this device (`respond`).
/** @type {Map<string | undefined, Promise<Set<string>>>} */
export const trusted = new Map()
/// Whether this device is behind the latest build of its program for its kind of machine, as its network says
/// (`builds`): a browser, or an app whose network says nothing of its build, isn't. Throws why, where its network
/// couldn't say.
export async function behind() {
  const { build, target } = await invoke("client")
  if (!build || !target) return false
  const latest = (await call("builds")).latest?.[`one/${target}`]?.build
  return Boolean(latest) && latest !== build
}

/// Calls the network's tool `name` as this device's client, and gives what it answered, or throws the error it said.
/** @param {string} name @param {Record<string, unknown>} [inputs] @returns {Promise<any>} */
async function call(name, inputs = {}) {
  const result = /** @type {{ structuredContent?: unknown, content: { text: string }[], isError?: boolean }} */ (await invoke("request", { view: "client", method: "tools/call", params: { name, arguments: inputs } }))
  if (result.isError) throw new Error(result.content[0]?.text)
  return result.structuredContent ?? JSON.parse(result.content[0].text)
}

/// Lists the files a server has for `subject`, which the client keeps, so an app opens with no server up. An open app
/// whose apps have a new version loads again. This user's own list has the launcher in it, which starts, or none:
/// which is a server's word, so a list kept from before is asked for again first, and while its network doesn't
/// answer, the window waits. With none, this app needs updating where its network says an update is ready for it
/// (`behind`): otherwise it's the network that has none right now. A launcher missing from a later list, as during a
/// release, leaves the one that's open as it is. Each of this user's lists says again whether an update is ready, by
/// one check while its network answers; a check that fails leaves what was said as it was, and with no launcher says
/// why in its problem box, with Try Again, since it can't say which it is.
/** @param {string} [subject] */
export async function list(subject) {
  listed.add(subject)
  const listing = invoke("apps", { subject })
  const apps = (/** @type {import("./host.js").Resource[]} */ resources) => new Set(resources.filter(({ _meta }) => _meta?.trusted === true).map(({ uri }) => uri.slice("ui://".length).split("/")[0]))
  const before = trusted.get(subject) ?? Promise.resolve(new Set())
  trusted.set(subject, listing.then(({ resources }) => apps(resources), () => before))
  // Nothing kept, and its network didn't answer: it's listed again once it does.
  const got = await listing.catch(() => undefined)
  if (!got) return
  let { resources, outdated } = got
  // A screen runs its own app and the UI library, which every one of them runs.
  const library = outdated.some((host) => host.split(".")[0] === "ui")
  for (const view of open.values()) if (library || outdated.includes(view.host)) page(view, { type: "stale" })
  if (!subject) {
    if (!launcherIn(resources) && shown.getSnapshot().matches("opening")) {
      await invoke("refresh", { view: "client" })
      ;({ resources } = await invoke("apps", { subject }))
    }
    launcher = launcherIn(resources) || launcher
    if (own) document.title = names.get(own.entrypoint) || document.title
    appTitle()
    shortcuts()
    if (launcher || own) on({ type: "listed" })
    if (shown.getSnapshot().context.connection !== "online") return
    const late = await behind().catch((/** @type {Error} */ error) => error)
    if (late instanceof Error) {
      if (launcher || own) exception(`Couldn't check for an update: ${late.message}`)
      else on({ type: "unchecked", why: late.message })
      return
    }
    if (!launcher && !own) on(late ? { type: "tooOld" } : { type: "none", only: await only() })
    on({ type: "updateReady", ready: !phone && late })
  }
}

/// The launcher's page among the files listed, and with them, which app each app opens first (`first`) and the
/// App Store's page (`appStore`). An app is a page of an MCP App's kind, at ui://<app>/<version>/<page>.
/** @param {import("./host.js").Resource[]} resources */
function launcherIn(resources) {
  const pages = resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app")
  const named = (/** @type {string} */ uri) => {
    const [app, _version, ...page] = uri.slice("ui://".length).split("/")
    return `${app}/${page.join("/")}`
  }
  // The last one set for an app is the one kept: its first.
  first = new Map(pages.toReversed().map(({ uri }) => [named(uri).split("/")[0], named(uri)]))
  files = new Set(resources.map(({ uri }) => named(uri)))
  names = new Map(pages.map(({ uri, name }) => [named(uri), name ?? ""]))
  icons = new Map(pages.flatMap(({ uri, icons }) => (icons?.[0] ? [[named(uri), icons[0].src]] : [])))
  needs = new Map(pages.flatMap(({ uri, _meta }) => (typeof _meta?.runtimeVersion === "number" ? [[named(uri), _meta.runtimeVersion]] : [])))
  const found = (/** @type {string} */ category) => pages.find(({ _meta }) => _meta?.category === category)?.uri
  appStore = found("app-store") ? named(/** @type {string} */ (found("app-store"))) : ""
  settings = found("settings") ? named(/** @type {string} */ (found("settings"))) : ""
  const home = found("launcher") ? named(/** @type {string} */ (found("launcher"))) : ""
  // The launcher's app opens the launcher, as `one:home-screen` does.
  if (home) first.set(home.split("/")[0], home)
  return home
}

/// Who may install for everyone, if not this person, who then can't install a Home Screen: "Only Andre can install a
/// Home Screen for Everyone", by the names they go by, as the App Store says it. Nothing where they can, or where
/// the network can't say.
async function only() {
  try {
    const [{ who }, { users }] = await Promise.all([call("permissions_check", { artifact: "network" }), call("permissions_users", { artifact: "network", relation: "installer" })])
    if (users.includes(who) || users.includes("everyone")) return undefined
    const { people } = await call("sign-in_everyone").catch(() => ({ people: [] }))
    const called = (/** @type {string} */ user) => people.find((/** @type {{ who: string }} */ person) => person.who === user)?.called ?? user.replace(/^users\//, "").replace(/^./, (first) => first.toUpperCase())
    const names = new Intl.ListFormat("en", { type: "disjunction" }).format(users.map(called))
    return users.length ? `Only ${names} can install a Home Screen for Everyone` : "Nobody can install a Home Screen for Everyone"
  } catch {
    return undefined
  }
}
