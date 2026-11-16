// Accounts: the outside accounts of the apps in a page's Accounts folder, under each app, as its plugin's `accounts`
// lists them. An account opens its app's page for it, and Add Account lists the apps, whose page signs one in.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { call, changed, Empty, Icon, leave, List, open, Problem, Row, Screen, Section, Skeleton, Split } from "ui"
import type { Account, App, Page } from "./types.ts"

const json = <T,>(tool: string, inputs?: object): Promise<T> => call(tool, inputs).then(JSON.parse)

/** An app of the folder, with its accounts, or why they couldn't be listed. */
type Listed = App & { accounts: Account[]; problem?: string }

/// The folder's apps, each with its accounts. An app whose accounts couldn't be listed says why.
const folder = createMemo(async () => {
  const page = await json<Page>("home-screen_page")
  const apps = page.items.find((item) => item.folder)?.folder ?? []
  return Promise.all(
    apps.map((app) =>
      json<Account[]>(`${app.app.split("/")[0]}_accounts`)
        .then((accounts): Listed => ({ ...app, accounts }))
        .catch((error): Listed => ({ ...app, accounts: [], problem: error.message })),
    ),
  )
})
changed(() => refresh(folder))

const [adding, setAdding] = createSignal(false)
const logo = (app: App) => <img class="logo" src={app.icon} alt="" />

function Accounts(props: { apps: Listed[] }) {
  const some = () => props.apps.some((app) => app.accounts.length || app.problem)
  return (
    <>
      <Show when={some()} fallback={<Empty>No accounts yet.</Empty>}>
        <For each={props.apps.filter((app) => app.accounts.length || app.problem)} keyed={(app) => app.app}>
          {(app) => (
            <Section heading={app().name}>
              <Show when={app().problem}>
                <Problem warn heading={`Can't list ${app().name} accounts`} text={app().problem} />
              </Show>
              <List>
                <For each={app().accounts} keyed={(account) => account.account}>
                  {(account) => (
                    <Row
                      start={logo(app())}
                      heading={account().account}
                      text={account().ok ? account().about : <span class="warn">{account().problem ?? "Needs attention"}</span>}
                      end={<Icon name="caret-right" along />}
                      onClick={() => open(`${app().app}#${encodeURIComponent(account().account)}`)}
                    />
                  )}
                </For>
              </List>
            </Section>
          )}
        </For>
      </Show>
      <Section>
        <List>
          <Row start={<span class="add"><Icon name="plus" /></span>} heading="Add Account" current={adding()} onClick={() => setAdding(true)} />
        </List>
      </Section>
    </>
  )
}

/// The apps an account can be added to, each opening its page, which signs one in.
function Adding(props: { apps: Listed[] }) {
  return (
    <Screen heading="Add Account" back onBack={() => setAdding(false)}>
      <List>
        <For each={props.apps} keyed={(app) => app.app}>
          {(app) => <Row start={logo(app())} heading={app().name} end={<Icon name="caret-right" along />} onClick={() => open(app().app)} />}
        </For>
      </List>
    </Screen>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Accounts" back onBack={leave}><Problem heading="Can't show your accounts" text={String((error() as Error).message ?? error())} /></Screen>}>
      <style>{`
        .logo { display: block; width: 2rem; height: 2rem; border-radius: 22.5%; }
        .add { display: grid; place-items: center; width: 2rem; height: 2rem; border-radius: 22.5%; background: var(--secondary); }
        .warn { color: var(--warning); }
      `}</style>
      <Loading fallback={<Screen heading="Accounts" back onBack={leave}><Skeleton rows={4} /></Screen>}>
        <Split open={adding()} onClose={() => setAdding(false)} detail={<Adding apps={folder()} />}>
          <Screen heading="Accounts" back onBack={leave}>
            <Accounts apps={folder()} />
          </Screen>
        </Split>
      </Loading>
    </Errored>
  ),
  document.body,
)
