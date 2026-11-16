// Chrome Profiles: your profiles, the sites each is signed in to, and signing in to a site on one yourself, on its
// browser's page drawn here, which you click and type on.
import { createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Field, Input, leave, List, Menu, Problem, Row, Screen, Section, Sheet, Skeleton, Split, toast } from "ui"
import "./app.css"

/** A profile, as `chrome-profiles_list` answers it. */
interface Profile {
  profile: string
  sites: { site: string; used: number }[]
  saved?: number
  signing_in?: { site: string; session: string }
}

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 hours ago".
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -60) return ago.format(Math.min(minutes, -1), "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
const summary = (p: Profile) =>
  p.signing_in
    ? `Signing in to ${p.signing_in.site}`
    : [p.sites.length ? p.sites.map((s) => s.site).join(", ") : "No sites yet", p.saved && `Saved ${since(p.saved)}`].filter(Boolean).join(" · ")

const [open, setOpen] = createSignal<string>()
const [asking, setAsking] = createSignal<"add" | "rename" | "sign-in">()

// Keys as the browser names them, as xdotool does, which is how a screen is told them.
const KEYS: Record<string, string> = { Enter: "Return", Backspace: "BackSpace", Tab: "Tab", Escape: "Escape", Delete: "Delete", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Home: "Home", End: "End", PageUp: "Prior", PageDown: "Next" }
const WIDTH = 1280
const HEIGHT = 800

/// A browser's page, as it is now, which you click, scroll and type on: what you do is sent to it in order, and
/// what you type while one is on its way goes with the next.
/** What's sent to a page, and how it's sent on: a click, a scroll, a key or what's typed. */
type Inputs = Record<string, any>

function Page(props: { session: string }) {
  // The browser it's of, for good: what's still on its way once the page has gone goes to it.
  const session = props.session
  const [picture, setPicture] = createSignal<string>()
  let field!: HTMLInputElement
  let queue: Promise<unknown> = Promise.resolve()
  let last: { action: string; inputs: Inputs } | undefined
  let timer: ReturnType<typeof setTimeout>
  const look = () =>
    call("chrome_screen", { session, action: "screenshot" })
      .then(setPicture)
      .catch(() => {})
  onSettled(() => {
    let stopped = false
    const again = async () => {
      if (document.visibilityState === "visible") await look()
      if (!stopped) timer = setTimeout(again, 1000)
    }
    again()
    return () => ((stopped = true), clearTimeout(timer))
  })
  const act = (action: string, inputs: Inputs, merge?: (was: Inputs, now: Inputs) => boolean) => {
    if (merge && last?.action === action && merge(last.inputs, inputs)) return
    const job = (last = { action, inputs: { ...inputs } })
    queue = queue
      .then(() => {
        if (last === job) last = undefined
        return call("chrome_screen", { session, action, ...job.inputs })
      })
      .then(look)
      .catch((error) => toast(error.message))
  }
  const at = (event: MouseEvent & { currentTarget: HTMLElement }) => {
    const box = event.currentTarget.getBoundingClientRect()
    return { x: Math.round(((event.clientX - box.left) * WIDTH) / box.width), y: Math.round(((event.clientY - box.top) * HEIGHT) / box.height) }
  }
  const type = (text: string) => text && act("type", { text }, (was, now) => ((was.text += now.text), true))
  const key = (event: KeyboardEvent) => {
    const named = KEYS[event.key] ?? (event.key.length === 1 ? event.key.toLowerCase() : undefined)
    const modified = event.ctrlKey || event.metaKey || event.altKey
    // Pasting is the field's own, and comes as what's typed.
    if (!named || (!modified && event.key.length === 1) || (modified && event.key.toLowerCase() === "v")) return
    event.preventDefault()
    const held = [event.ctrlKey && "ctrl", event.altKey && "alt", event.shiftKey && "shift", event.metaKey && "super"].filter(Boolean)
    act("key", { keys: [...held, named].join("+") })
  }
  return (
    <div
      class="page"
      style={{ "aspect-ratio": `${WIDTH} / ${HEIGHT}` }}
      onClick={(event) => (act("click", at(event)), field.focus())}
      onWheel={(event) => {
        event.preventDefault()
        if (event.deltaY) act("scroll", { ...at(event), direction: event.deltaY > 0 ? "down" : "up", amount: 1 }, (was, now) => was.direction === now.direction && ((was.amount = Math.min(50, was.amount + 1)), true))
      }}
    >
      <Show when={picture()}>
        <img src={picture()} alt="The page" draggable={false} />
      </Show>
      <input
        ref={field}
        class="keys"
        aria-label="Type on the page"
        autocomplete="off"
        autocapitalize="off"
        spellcheck={false}
        onKeyDown={key}
        onInput={(event) => (type(event.target.value), (event.target.value = ""))}
      />
    </div>
  )
}

function SigningIn(props: { profile: Profile & { signing_in: {} } }) {
  const p = () => props.profile as Required<Profile>
  const done = () => {
    const { profile, signing_in } = p()
    return call("chrome_close", { session: signing_in.session })
      .then(() => toast(`Saved to ${profile}`))
      .catch((error) => toast(error.message))
  }
  return (
    <Section heading={`Signing in to ${p().signing_in.site}`}>
      <Page session={p().signing_in.session} />
      <p class="hint">Click the page and type as you would in Chrome. When you're signed in, tap Done.</p>
      <Button onClick={done}>Done</Button>
    </Section>
  )
}

function Detail(props: { profile: Profile }) {
  const p = () => props.profile
  const act = async ({ value }: { value: string }) => {
    if (value === "rename") return setAsking("rename")
    const yes = await confirm({ heading: `Delete ${p().profile}?`, text: "Every site signed in on it is signed out, for good.", action: "Delete", destructive: true })
    if (yes) await call("chrome-profiles_remove", { profile: p().profile }).then(() => setOpen()).catch((error) => toast(error.message))
  }
  return (
    <Screen
      heading={p().profile}
      back
      onBack={() => setOpen()}
      action={
        <Menu.Root onSelect={act}>
          <Menu.Trigger aria-label={`More for ${p().profile}`} />
          <Menu.Positioner>
            <Menu.Content>
              <Menu.Item value="rename">Rename</Menu.Item>
              <Menu.Separator />
              <Menu.Item value="delete" destructive>Delete</Menu.Item>
            </Menu.Content>
          </Menu.Positioner>
        </Menu.Root>
      }
    >
      <Show when={p().signing_in} fallback={<Button onClick={() => setAsking("sign-in")}>Sign In to a Site</Button>}>
        <SigningIn profile={p() as Required<Profile>} />
      </Show>
      <Section heading="Sites">
        <Show when={p().sites.length} fallback={<Empty>Not signed in to any sites yet.</Empty>}>
          <List>
            <For each={p().sites} keyed={(s) => s.site}>
              {(s) => <Row heading={s().site} text={`Used ${since(s().used)}`} />}
            </For>
          </List>
        </Show>
      </Section>
    </Screen>
  )
}

/// What the person's asked for: a name for a new profile or a renamed one, or the site to sign in to.
function Ask() {
  const [busy, setBusy] = createSignal(false)
  const heading = () => ({ add: "Add Profile", rename: "Rename Profile", "sign-in": "Sign In to a Site" })[asking()!]
  const send = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    const typed = String(new FormData(event.currentTarget).get("value")).trim()
    const [tool, inputs] = ({
      add: ["create", { name: typed }],
      rename: ["rename", { profile: open(), name: typed }],
      "sign-in": ["sign_in", { profile: open(), url: /^https?:\/\//.test(typed) ? typed : `https://${typed}` }],
    } as const)[asking()!]
    setBusy(true)
    await call(`chrome-profiles_${tool}`, inputs)
      .then(() => {
        if (tool !== "sign_in") setOpen(typed)
        setAsking()
      })
      .catch((error) => toast(error.message))
    setBusy(false)
  }
  return (
    <Sheet heading={heading()} open={!!asking()} onClose={() => setAsking()}>
      <form onSubmit={send}>
        <Show
          when={asking() === "sign-in"}
          fallback={
            <Field label="Name">
              <Input name="value" required autofocus placeholder="Personal" value={asking() === "rename" ? open() : ""} />
            </Field>
          }
        >
          <Field label="Site" hint="You sign in yourself. Agents never see your password.">
            <Input name="value" type="url" inputmode="url" required autofocus placeholder="github.com" />
          </Field>
        </Show>
        <Button type="submit" loading={busy()}>{asking() === "sign-in" ? "Open" : asking() === "add" ? "Add" : "Rename"}</Button>
      </form>
    </Sheet>
  )
}

function App() {
  const profiles = createMemo(() => call("chrome-profiles_list").then((text): Profile[] => JSON.parse(text)))
  changed(() => refresh(profiles))
  const opened = () => profiles().find((p) => p.profile === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your profiles" text={(error() as Error).message} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()}><Detail profile={opened()!} /></Show>}>
          <Screen heading="Chrome Profiles" back onBack={leave} action={<Button variant="outline" onClick={() => setAsking("add")}>Add Profile</Button>}>
            <Show when={profiles().length} fallback={<Empty>No profiles. Add one, like Personal or Work, to sign in to sites on.</Empty>}>
              <List>
                <For each={profiles()} keyed={(p) => p.profile}>
                  {(p) => <Row heading={p().profile} text={summary(p())} current={open() === p().profile} onClick={() => setOpen(p().profile)} />}
                </For>
              </List>
            </Show>
          </Screen>
        </Split>
        <Ask />
      </Loading>
    </Errored>
  )
}

render(() => <App />, document.body)
