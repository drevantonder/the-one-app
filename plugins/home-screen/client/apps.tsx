// Apps: every plugin with an app or accounts that could be on a page, each with its subtitle, and on its sheet what
// it's about and which of your pages it's on. Adding one
// to a page puts it on the clients of everyone who sees that page. Taking one off keeps what it kept, and adding it
// back brings that back. Opened as `#accounts` (Accounts' Add Account for Another App), it lists those that sign in.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, Empty, leave, List, Problem, Row, Screen, Sheet, Skeleton, Switch, toast } from "ui"
import type { Page } from "./types.ts"

/** A plugin, as `home-screen_store` lists it. */
interface App {
  plugin: string
  name: string
  icon: string
  subtitle?: string
  about?: string
  accounts: boolean
}

const named = (who?: string) => (!who ? "You" : who === "network" ? "Everyone" : who.replace("groups/", "").replace(/^./, (first) => first.toUpperCase()))
const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { for: who }).then(JSON.parse)

/// Every app there is, and the pages this user sees, each with the plugins it has.
const store = createMemo(async () => {
  const groups = await json<{ group: string }[]>("permissions_groups").catch((): { group: string }[] => [])
  const whos = [undefined, ...groups.map(({ group }) => group), "network"]
  const [all, pages] = await Promise.all([json<App[]>("home-screen_store"), Promise.all(whos.map((who) => json<Page>("home-screen_page", {}, who).then((page): Page => ({ ...page, for: who, name: named(who) }))))])
  const apps = location.hash === "#accounts" ? all.filter((app) => app.accounts) : all
  return { apps, pages }
})
changed(() => refresh(store))

/// The plugin, by its name there, if it's on the page: its app, or its accounts. `put` says it was put there
/// rather than installed for it.
function on(page: Page, app: App): { name: string; put?: boolean } | undefined {
  const item = page.items.find((item) => item.plugin === app.plugin)
  if (item?.app) return { name: item.app.split("/")[0], put: item.put }
  const signs = page.accounts.find((signs) => signs.plugin === app.plugin)
  return signs && { name: signs.name }
}
const [managing, setManaging] = createSignal<string>()

/// Puts an app on a page or takes it off. One in use for the network goes on without installing anything, and any
/// other is installed for whoever's page it is, which takes a server of their own.
async function put(page: Page, app: App, there: boolean) {
  const item = on(page, app)
  try {
    if (there) await call("home-screen_add", { plugin: app.plugin }, { for: page.for }).catch(() => call("install", { plugin: app.plugin }, { for: page.for }))
    else if (item?.put) await call("home-screen_remove", { plugin: app.plugin }, { for: page.for })
    else await call("uninstall", { name: item!.name }, { for: page.for })
  } catch (error) {
    toast((error as Error).message)
  }
  refresh(store)
}

function Apps(props: { store: { apps: App[]; pages: Page[] } }) {
  const pages = (app: App) => props.store.pages.filter((page) => on(page, app))
  const app = () => props.store.apps.find((app) => app.plugin === managing())
  return (
    <>
      <Show when={props.store.apps.length} fallback={<Empty>No apps yet. Ask your AI to make one.</Empty>}>
        <List>
          <For each={props.store.apps} keyed={(app) => app.plugin}>
            {(app) => (
              <Row
                heading={app().name}
                text={app().subtitle}
                start={<img class="icon" src={app().icon} alt="" />}
                end={<Button variant={pages(app()).length ? "outline" : "secondary"} onClick={() => setManaging(app().plugin)}>{pages(app()).length ? "Manage" : "Add to"}</Button>}
              />
            )}
          </For>
        </List>
      </Show>
      <Sheet heading={app()?.name} open={!!app()} onClose={() => setManaging()}>
        <Show when={app()}>
          <Show when={app()!.about}>
            <p class="about">{app()!.about}</p>
          </Show>
          <form>
            <For each={props.store.pages} keyed={(page) => page.who}>
              {(page) => <Switch label={page().name} checked={!!on(page(), app()!)} disabled={!page().arranges} onChange={(event) => put(page(), app()!, event.currentTarget.checked)} />}
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
          <Apps store={store()} />
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
