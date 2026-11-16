// Accounts: a section for each plugin with accounts (SKILL.md) on any of this user's pages, its accounts as its
// `accounts` lists them, each filling in as it answers. One that needs signing in again says why, with Sign In;
// each has Sign Out, and each section Add Account. Add Account for Another App opens the App Store's plugins that
// sign in.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Icon, leave, List, open, Problem, Row, Screen, Section, Skeleton, toast } from "ui"
import type { Account, Page, Signs } from "./types.ts"

const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { for: who }).then(JSON.parse)

/// Every page's plugins with accounts, each once, by its title.
const signs = createMemo(async () => {
  const groups = await json<{ group: string }[]>("permissions_groups").catch((): { group: string }[] => [])
  const pages = await Promise.all([undefined, ...groups.map(({ group }) => group), "network"].map((who) => json<Page>("home-screen_page", {}, who).then((page) => ({ ...page, for: who }))))
  const all = new Map(pages.flatMap((page) => page.accounts.map((one): [string, Signs] => [one.plugin, { ...one, for: page.for }])))
  return [...all.values()].sort((a, b) => a.title.localeCompare(b.title))
})
changed(() => refresh(signs))

/// Does what the person asked, and says what the server said if it wouldn't.
const does = (what: () => Promise<unknown>) => what().catch((error) => toast(error.message))

/// Signs an account in to a plugin, or with `account`, signs that one in again: on the plugin's own page for it, or
/// at the web address its `sign_in` answers, in the device's browser.
const signIn = (plugin: Signs, account?: string) =>
  does(async () => {
    if (plugin.sign_in) return open(account ? `${plugin.sign_in}#${encodeURIComponent(account)}` : plugin.sign_in, { for: plugin.for })
    const { url } = await json<{ url: string }>(`${plugin.name}_sign_in`, account ? { account } : {}, plugin.for)
    return open(url)
  })

/// A plugin's section: its accounts, once its `accounts` answers, or why it couldn't. With none, Add Account is all.
function Accounts(props: { plugin: Signs }) {
  const accounts = createMemo(() => json<Account[]>(`${props.plugin.name}_accounts`, {}, props.plugin.for))
  changed(() => refresh(accounts))
  const signOut = async (account: string) => {
    const sure = await confirm({ heading: `Sign out of ${account}?`, text: `${props.plugin.title} can't use it until it's signed in again.`, action: "Sign Out", destructive: true })
    if (sure) await does(() => call(`${props.plugin.name}_sign_out`, { account }, { for: props.plugin.for }))
    refresh(accounts)
  }
  return (
    <Section
      heading={
        <span class="heading">
          <img class="logo" src={props.plugin.icon} alt="" />
          <span>{props.plugin.title}</span>
          <Button variant="secondary" onClick={() => signIn(props.plugin)}>Add Account</Button>
        </span>
      }
    >
      <Errored fallback={(error) => <Problem warn heading={`Can't list ${props.plugin.title} accounts`} text={String((error() as Error).message ?? error())} />}>
        <Loading fallback={<Skeleton rows={1} />}>
          <Show when={accounts().length}>
            <List>
              <For each={accounts()} keyed={(account) => account.account}>
                {(account) => (
                  <Row
                    heading={account().account}
                    text={account().ok ? undefined : <span class="warn"><Icon name="warning-circle" />{account().why ?? "Needs signing in again"}</span>}
                    end={
                      <span class="actions">
                        <Show when={!account().ok}>
                          <Button variant="outline" onClick={() => signIn(props.plugin, account().account)}>Sign In</Button>
                        </Show>
                        <Button variant="ghost" onClick={() => signOut(account().account)}>Sign Out</Button>
                      </span>
                    }
                  />
                )}
              </For>
            </List>
          </Show>
        </Loading>
      </Errored>
    </Section>
  )
}

function All(props: { signs: Signs[] }) {
  return (
    <>
      <Show when={props.signs.length} fallback={<Empty>No accounts yet.</Empty>}>
        <For each={props.signs} keyed={(plugin) => plugin.plugin}>
          {(plugin) => <Accounts plugin={plugin()} />}
        </For>
      </Show>
      <div class="another">
        <Button variant="outline" onClick={() => open("home-screen/client/apps.html#accounts")}>Add Account for Another App</Button>
      </div>
    </>
  )
}

render(
  () => (
    <Screen heading="Accounts" back onBack={leave}>
      <style>{`
        .heading { display: flex; align-items: center; gap: 0.75rem; max-width: var(--readable); color: var(--foreground); font-size: var(--text-base); font-weight: 600; }
        .heading > span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .logo { display: block; width: 2rem; height: 2rem; border-radius: 22.5%; }
        .warn { display: inline-flex; align-items: center; gap: 0.25rem; color: var(--warning); }
        .actions { display: flex; gap: 0.25rem; }
        .another { padding-block: 1rem; }
      `}</style>
      <Errored fallback={(error) => <Problem heading="Can't show your accounts" text={String((error() as Error).message ?? error())} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <All signs={signs()} />
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
