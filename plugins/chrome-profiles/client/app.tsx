// Chrome Profiles: your profiles, the sites each was used on, and updating one yourself: its browser, drawn here, which
// you click and type on as you would Chrome, then save.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Field, Input, leave, List, LiveScreen, Menu, Problem, Row, Screen, Section, Sheet, Skeleton, Split, toast } from "ui"
import "./app.css"

/** A profile, as `chrome-profiles_list` answers it. */
interface Profile {
  profile: string
  sites: { site: string; used: number }[]
  saved?: number
  updating?: { session: string; until: number }
}

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 hours ago".
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -60) return ago.format(Math.min(minutes, -1), "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
const time = new Intl.DateTimeFormat(undefined, { timeStyle: "short" })
const summary = (p: Profile) =>
  p.updating
    ? "Updating"
    : [p.sites.length ? p.sites.map((s) => s.site).join(", ") : "No sites yet", p.saved && `Saved ${since(p.saved)}`].filter(Boolean).join(" · ")

const [open, setOpen] = createSignal<string>()
const [asking, setAsking] = createSignal<"add" | "rename">()

/// The profile's browser, open for its person: the browser as it is, with its own address bar and tabs, then Save.
function Updating(props: { profile: Profile }) {
  const p = () => props.profile
  const [busy, setBusy] = createSignal<"save" | "discard">()
  const finish = async (tool: "save" | "discard") => {
    setBusy(tool)
    await call(`chrome-profiles_${tool}`, { profile: p().profile })
      .then(() => toast(tool === "save" ? `Saved ${p().profile}` : `Closed ${p().profile} without saving`))
      .catch((error) => toast(error.message))
    setBusy()
  }
  return (
    <Section heading="Updating">
      <Show when={p().updating!.session} keyed>
        {(session) => <LiveScreen label="The browser" screen={(inputs) => call("chrome_screen", { session, ...inputs })} />}
      </Show>
      <p class="hint">Use it as you would Chrome: sign in to any sites you like. It's saved by itself at {time.format(p().updating!.until)}.</p>
      <div class="actions">
        <Button loading={busy() === "save"} disabled={!!busy()} onClick={() => finish("save")}>Save</Button>
        <Button variant="outline" loading={busy() === "discard"} disabled={!!busy()} onClick={() => finish("discard")}>Discard</Button>
      </div>
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
      <Show when={p().updating} fallback={<Update profile={p().profile} />}>
        <Updating profile={p()} />
      </Show>
      <Section heading="Sites">
        <Show when={p().sites.length} fallback={<Empty>Not used on any sites yet.</Empty>}>
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

/// Opens the profile's browser for its person to update.
function Update(props: { profile: string }) {
  const [busy, setBusy] = createSignal(false)
  const update = async () => {
    setBusy(true)
    await call("chrome-profiles_update", { profile: props.profile }).catch((error) => toast(error.message))
    setBusy(false)
  }
  return (
    <Button loading={busy()} onClick={update}>Update Profile</Button>
  )
}

/// What the person's asked for: a name for a new profile, or a profile's new name.
function Ask() {
  const [busy, setBusy] = createSignal(false)
  const send = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    const typed = String(new FormData(event.currentTarget).get("value")).trim()
    const [tool, inputs] = asking() === "add" ? ["create", { name: typed }] : ["rename", { profile: open(), name: typed }]
    setBusy(true)
    await call(`chrome-profiles_${tool}`, inputs)
      .then(() => (setOpen(typed), setAsking()))
      .catch((error) => toast(error.message))
    setBusy(false)
  }
  return (
    <Sheet heading={asking() === "add" ? "Add Profile" : "Rename Profile"} open={!!asking()} onClose={() => setAsking()}>
      <form onSubmit={send}>
        <Field label="Name">
          <Input name="value" required autofocus placeholder="Personal" value={asking() === "rename" ? open() : ""} />
        </Field>
        <Button type="submit" loading={busy()}>{asking() === "add" ? "Add" : "Rename"}</Button>
      </form>
    </Sheet>
  )
}

function App() {
  const profiles = createMemo(() => call("chrome-profiles_list").then((text): Profile[] => JSON.parse(text).profiles))
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
