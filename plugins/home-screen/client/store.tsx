// The App Store, which Settings calls Apps: every plugin a person could add, one with an app or with accounts, each
// with its subtitle, and on its sheet what it's about and which of your pages it's on. Adding one to a page installs
// it for whoever's page it is or puts its app there, and so on the clients of everyone who sees that page; one with
// accounts and no app is then in their Accounts. Taking one off keeps what it kept, and adding it back brings that
// back. Opened as `#accounts` (Accounts' Add Account for Another App), it lists the plugins with accounts.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, Empty, leave, List, Problem, Row, Screen, Sheet, Skeleton, Switch, toast } from "ui"
import type { Page } from "./types.ts"

/** A plugin, as `home-screen_store` lists it. */
interface Plugin {
  plugin: string
  title: string
  icon: string
  subtitle?: string
  about?: string
  accounts: boolean
}

const named = (who?: string) => (!who ? "You" : who === "network" ? "Everyone" : who.replace("groups/", "").replace(/^./, (first) => first.toUpperCase()))
const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { subject: who }).then(JSON.parse)

/// Every plugin a person could add, and the pages this user sees, each with the plugins it has.
const store = createMemo(async () => {
  const groups = await json<{ groups: { group: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string }[] => [])
  const whos = [undefined, ...groups.map(({ group }) => group), "network"]
  const [all, pages] = await Promise.all([json<{ plugins: Plugin[] }>("home-screen_store").then(({ plugins }) => plugins), Promise.all(whos.map((who) => json<Page>("home-screen_page", {}, who).then((page): Page => ({ ...page, subject: who, name: named(who) }))))])
  const plugins = location.hash === "#accounts" ? all.filter((plugin) => plugin.accounts) : all
  return { plugins, pages }
})
changed(() => refresh(store))

/// The plugin, by its name there, if it's on the page: its app, or, with none, it in Accounts. `put` says it was
/// put there rather than installed for it.
function on(page: Page, plugin: Plugin): { name: string; put?: boolean } | undefined {
  const item = page.items.find((item) => item.plugin === plugin.plugin)
  if (item?.app) return { name: item.app.split("/")[0], put: item.put }
  const withAccounts = page.plugins_with_accounts.find((one) => one.plugin === plugin.plugin)
  return withAccounts && { name: withAccounts.name }
}
const [managing, setManaging] = createSignal<string>()

/// Puts a plugin on a page or takes it off. One in use for the network has its app put on without installing
/// anything, and any other is installed for whoever's page it is, which takes a server of their own.
async function put(page: Page, plugin: Plugin, there: boolean) {
  const item = on(page, plugin)
  try {
    if (there) await call("home-screen_add", { plugin: plugin.plugin }, { subject: page.subject }).catch(() => call("install", { plugin: plugin.plugin }, { subject: page.subject }))
    else if (item?.put) await call("home-screen_remove", { plugin: plugin.plugin }, { subject: page.subject })
    else await call("uninstall", { name: item!.name }, { subject: page.subject })
  } catch (error) {
    toast((error as Error).message)
  }
  refresh(store)
}

function Store(props: { store: { plugins: Plugin[]; pages: Page[] } }) {
  const pages = (plugin: Plugin) => props.store.pages.filter((page) => on(page, plugin))
  const plugin = () => props.store.plugins.find((plugin) => plugin.plugin === managing())
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
                end={<Button variant={pages(plugin()).length ? "outline" : "secondary"} onClick={() => setManaging(plugin().plugin)}>{pages(plugin()).length ? "Manage" : "Add to"}</Button>}
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
            <For each={props.store.pages} keyed={(page) => page.who}>
              {(page) => <Switch label={page().name} checked={!!on(page(), plugin()!)} disabled={!page().arranges} onChange={(event) => put(page(), plugin()!, event.currentTarget.checked)} />}
            </For>
          </form>
        </Show>
      </Sheet>
    </>
  )
}

render(
  () => (
    <Screen heading="Apps" back onBack={leave}>
      <style>{".icon { display: block; width: 2.25rem; height: 2.25rem; border-radius: 22.5%; } .about { margin: 0 0 1rem; }"}</style>
      <Errored fallback={(error) => <Problem heading="Can't show the apps" text={String((error() as Error).message ?? error())} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <Store store={store()} />
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
