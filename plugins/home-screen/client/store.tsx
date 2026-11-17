// The App Store: every plugin a person could add, each with its subtitle, and on its sheet what it's about and whose
// it's in use for: yours, everyone's or a group's. Adding one installs it for them, into their App Library, or where
// it's in use for everyone already, puts its app on their Home Screen. Taking one off keeps what it kept, and adding
// it back brings that back. Where the person may not install, a grey line says who may, whom they ask themselves.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, Empty, leave, List, Problem, Row, Screen, Sheet, Skeleton, Switch, toast } from "ui"
import type { Pages } from "./types.ts"

/** A plugin, as `home-screen_store` lists it. */
interface Plugin {
  plugin: string
  title: string
  icon: string
  subtitle?: string
  about?: string
}

/** An owner of a Home Screen, by the name a person reads, what `home-screen_pages` answers for them, the plugins
 * installed for them, by their names there, and who may install for them. */
interface Owner extends Pages {
  subject?: string
  name: string
  installed: Map<string, string>
  installers: string[]
}

const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

/// Every plugin a person could add, and the Home Screens this user sees: theirs, everyone's and their groups', each
/// with who may install for it, by the names they go by.
const store = createMemo(async () => {
  const [groups, people] = await Promise.all([
    json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => []),
    json<{ people: { who: string; called?: string }[] }>("sign-in_everyone").then(({ people }) => people, (): { who: string; called?: string }[] => []),
  ])
  const called = (who: string) => people.find((person) => person.who === who)?.called ?? who.replace(/^users\//, "").replace(/^./, (first) => first.toUpperCase())
  const whose: { subject?: string; name: string }[] = [{ name: "You" }, { subject: "network", name: "Everyone" }, ...groups.map(({ group, called }) => ({ subject: group, name: called }))]
  const [all, owners] = await Promise.all([
    json<{ plugins: Plugin[] }>("home-screen_store").then(({ plugins }) => plugins),
    Promise.all(
      whose.map(async (who): Promise<Owner> => {
        const [pages, plugins] = await Promise.all([json<Pages>("home-screen_pages", {}, who.subject), json<{ plugins: { name: string; plugin: string; for: string }[] }>("plugins", {}, who.subject).then(({ plugins }) => plugins)])
        // What's installed for them, rather than for whoever they're in.
        const installed = new Map(plugins.filter((plugin) => plugin.for === pages.who).map((plugin) => [plugin.plugin, plugin.name]))
        // Whoever may install for a group or everyone; anyone installs for themselves.
        const installers = who.subject ? await json<{ users: string[] }>("permissions_users", { artifact: who.subject, relation: "installer" }).then(({ users }) => users, (): string[] => []) : [pages.who]
        return { ...pages, ...who, installed, installers }
      }),
    ),
  ])
  return { plugins: all, owners, me: owners[0].who, called }
})
changed(() => refresh(store))

/// The plugin, by its name there, if an owner has it: installed for them, or with its app put on their Home Screen
/// (`put`).
function on(owner: Owner, plugin: Plugin): { name: string; put?: boolean } | undefined {
  const name = owner.installed.get(plugin.plugin)
  if (name) return { name }
  const item = owner.pages.flatMap((page) => page.items).find((item) => item.app && item.plugin === plugin.plugin)
  return item?.app ? { name: item.app.split("/")[0], put: true } : undefined
}
const [managing, setManaging] = createSignal<string>()

/// Adds a plugin for an owner or takes it off. One in use for everyone has its app put on their Home Screen without
/// installing anything, and any other is installed for them, which takes a server of their own.
async function put(owner: Owner, plugin: Plugin, there: boolean) {
  const item = on(owner, plugin)
  try {
    if (there) await call("home-screen_add", { app: plugin.plugin }, { subject: owner.subject }).catch(() => call("install", { plugin: plugin.plugin }, { subject: owner.subject }))
    else if (item?.put) await call("home-screen_remove", { app: plugin.plugin }, { subject: owner.subject })
    else await call("uninstall", { name: item!.name }, { subject: owner.subject })
  } catch (error) {
    toast(error as Error)
  }
  refresh(store)
}

function Store(props: { store: Awaited<ReturnType<typeof store>> }) {
  const owners = (plugin: Plugin) => props.store.owners.filter((owner) => on(owner, plugin))
  const plugin = () => props.store.plugins.find((plugin) => plugin.plugin === managing())
  /// Who may install for an owner, if not this person: "Only Andre can install for Everyone".
  const only = (owner: Owner) => {
    if (owner.installers.includes(props.store.me) || owner.installers.includes("everyone")) return undefined
    const who = new Intl.ListFormat("en", { type: "disjunction" }).format(owner.installers.map(props.store.called))
    return owner.installers.length ? `Only ${who} can install for ${owner.name}` : `Nobody can install for ${owner.name}`
  }
  return (
    <>
      <Show when={props.store.plugins.length} fallback={<Empty>No apps yet. Ask your AI to make one.</Empty>}>
        <List>
          <For each={props.store.plugins} keyed={(plugin) => plugin.plugin}>
            {(plugin) => (
              <Row
                heading={plugin().title}
                text={plugin().subtitle}
                start={<img class="icon" src={plugin().icon} alt="" />}
                end={<Button variant={owners(plugin()).length ? "outline" : "secondary"} onClick={() => setManaging(plugin().plugin)}>{owners(plugin()).length ? "Manage" : "Add to"}</Button>}
              />
            )}
          </For>
        </List>
      </Show>
      <Sheet heading={plugin()?.title} open={!!plugin()} onClose={() => setManaging()}>
        <Show when={plugin()}>
          <Show when={plugin()!.about}>
            <p class="about">{plugin()!.about}</p>
          </Show>
          <form>
            <For each={props.store.owners} keyed={(owner) => owner.who}>
              {(owner) => (
                <Show when={only(owner())} fallback={<Switch label={owner().name} checked={!!on(owner(), plugin()!)} onChange={(event) => put(owner(), plugin()!, event.currentTarget.checked)} />}>
                  {(line) => <p class="only">{line()}</p>}
                </Show>
              )}
            </For>
          </form>
        </Show>
      </Sheet>
    </>
  )
}

render(
  () => (
    <Screen heading="App Store" back onBack={leave}>
      <style>{".icon { display: block; width: 2.25rem; height: 2.25rem; border-radius: 22.5%; } .about { margin: 0 0 1rem; } .only { margin: 0.5rem 0; color: var(--muted-foreground); font-size: var(--text-sm); }"}</style>
      <Errored fallback={(error) => <Problem heading="Can't show the App Store" text={error() as Error} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <Store store={store()} />
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
