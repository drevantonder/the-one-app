// A Google account, opened from the Accounts folder with its address after a `#`: what needs doing, which Google
// apps use it, each switched on or off here, and Sign Out. With none, Sign In adds an account. Signing in opens
// Google's sign-in in the device's own browser; Google sends the person back to the network's address, which
// finishes it, and this screen hears that something changed.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, leave, List, open, Problem, Row, Screen, Section, Skeleton, Switch, toast } from "ui"
import "./app.css"

const json = (tool, inputs) => call(tool, inputs).then(JSON.parse)
const account = decodeURIComponent(location.hash.slice(1))

/// Each Google app, with the accounts it uses.
const apps = createMemo(() =>
  json("google_apps").then((apps) => Promise.all(apps.map(async (app) => ({ ...app, accounts: await json(`${app.name}_accounts`) })))),
)
changed(() => refresh(apps))

/// Does what the person asked, and says what the server said if it wouldn't.
const does = (what) => what().catch((error) => toast(error.message))
const signIn = (app) => does(async () => open((await json(`${app.name}_sign_in`, account ? { account } : {})).url))
const signOut = (app) => does(async () => (await call(`${app.name}_sign_out`, { account }), refresh(apps)))

function Account(props) {
  const using = () => props.apps.filter((app) => app.accounts.some((a) => a.account === account))
  const out = () => using().filter((app) => !app.accounts.find((a) => a.account === account).ok)
  const all = async () => {
    const sure = await confirm({ heading: `Sign out of ${account}?`, text: "No Google app here uses it any more.", action: "Sign Out", destructive: true })
    if (!sure) return
    await does(async () => {
      for (const app of using()) await call(`${app.name}_sign_out`, { account })
      leave()
    })
  }
  return (
    <>
      <Show when={out().length}>
        <Problem
          warn
          heading="Needs signing in again"
          text={`Google signed it out. ${new Intl.ListFormat("en").format(out().map((app) => app.title))} can't reach it.`}
          action={<Button variant="outline" onClick={() => signIn(out()[0])}>Sign In</Button>}
        />
      </Show>
      <Section heading="Used By">
        <List>
          <For each={props.apps} keyed={(app) => app.name}>
            {(app) => {
              const on = () => using().includes(app())
              return <Row heading={app().title} end={<Switch aria-label={app().title} checked={on()} onChange={(event) => (event.currentTarget.checked ? signIn(app()) : signOut(app()))} />} />
            }}
          </For>
        </List>
      </Section>
      <Show when={using().length}>
        <div class="out">
          <Button variant="ghost" class="sign-out" onClick={all}>Sign Out</Button>
        </div>
      </Show>
    </>
  )
}

function Adding(props) {
  return (
    <Section>
      <p class="how">Sign in, then choose which apps use the account.</p>
      <div class="add">
        <Button onClick={() => signIn(props.apps[0])}>Sign In to Google</Button>
      </div>
    </Section>
  )
}

function Google() {
  return (
    <Screen heading={account || "Google"} back onBack={leave}>
      <Errored fallback={(error, reset) => <Problem heading="Couldn't show your accounts" text={error().message} action={<Button variant="outline" onClick={reset}>Try again</Button>} />}>
        <Loading fallback={<Skeleton rows={3} />}>
          <Show when={apps().length} fallback={<Problem warn heading="No Google app yet" text="Add Gmail, Calendar, Drive, Docs or Sheets from the App Store." />}>
            <Show when={account} fallback={<Adding apps={apps()} />}>
              <Account apps={apps()} />
            </Show>
          </Show>
        </Loading>
      </Errored>
    </Screen>
  )
}

render(() => <Google />, document.body)
