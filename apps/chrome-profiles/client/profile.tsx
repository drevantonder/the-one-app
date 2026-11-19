// A Chrome profile, as the Vault draws it beside its list (the item type `vault.types` gives): the extensions installed
// in it and the pages it visited last, as it was last saved, and updating it yourself: its browser, drawn here, which
// you click and type on as you would Chrome, then save. `Summary` is its row's line in the Vault's list.
import { createMemo, createSignal, For, Loading, refresh, Show } from "solid-js"
import { Button, call, changed, confirm, Empty, Field, Image, Input, List, LiveScreen, Menu, Problem, Row, Screen, Section, Sheet, since, Status, Steps, styles, Text, toast, tokens, View } from "ui"

const look = styles({
  note: { color: tokens.colour.secondaryLabel },
  buttons: { flexDirection: "row", gap: tokens.grid[2] },
  extension: { width: tokens.grid[6], height: tokens.grid[6] },
})

/** A profile, as `chrome-profiles_list` answers it. */
interface Profile {
  profile: string
  history: { url: string; title: string; visited: number }[]
  extensions: { id: string; name: string; icon: string | null }[]
  saved?: number
  updating?: { session: string; until: number; state: State; stage?: string; why?: string }
}

/** What the Vault gives the component: the profile by its name, who it's for, and what to do once it's renamed or gone. */
interface Props {
  item: string
  subject?: string
  open?: (item: string) => void
  close?: () => void
}

/** An update's state, as the server keeps it (chrome_profiles.ex's `@steps`). */
type State = "starting" | "opening" | "ready" | "saving" | "discarding" | "failed"
// What a profile's row says while it's being updated.
const SAYS: Record<State, string> = { starting: "Starting", opening: "Starting", ready: "Updating", saving: "Saving", discarding: "Closing", failed: "Couldn't start" }
/// A time of day, as "3:45 pm".
const time = (at: number) => {
  const date = new Date(at)
  return `${date.getHours() % 12 || 12}:${String(date.getMinutes()).padStart(2, "0")} ${date.getHours() < 12 ? "am" : "pm"}`
}
/// A page's site, as its address bar shows it: "github.com".
const site = (url: string) => url.match(/^[a-z][a-z\d+.-]*:\/\/(?:[^/?#@]*@)?(?:www\.)?([^/?#:]*)/i)?.[1] ?? url

/// The profile, as `chrome-profiles_list` answers it now, asked again whenever something changed.
function useProfile(props: Props) {
  const profiles = createMemo(() => call("chrome-profiles_list", {}, { subject: props.subject }).then((text): Profile[] => JSON.parse(text).profiles))
  changed(() => refresh(profiles))
  return () => profiles().find((p) => p.profile === props.item)
}

/// Its row's line in the Vault: what it's doing while it's being updated, else the site it was on last and when it
/// was saved.
export function Summary(props: Props) {
  const profile = useProfile(props)
  const line = () => {
    const p = profile()
    if (!p) return ""
    if (p.updating) return SAYS[p.updating.state]
    return [p.history[0] && site(p.history[0].url), p.saved ? `Saved ${since(p.saved)}` : "Not saved yet"].filter(Boolean).join(" · ")
  }
  return <>{line()}</>
}

/// How far the browsers' image is being built, where a machine is building it, as Containers says it ("Installing
/// packages: 40%"), asked again as it changes. Nothing while none is: then a browser takes a few seconds.
function Building(props: { subject?: string }) {
  const machines = createMemo(() => call("containers_machines", {}, { subject: props.subject }).then((text): { desktop?: string | null }[] => JSON.parse(text).machines))
  changed(() => refresh(machines))
  return <>{machines().find((m) => m.desktop)?.desktop}</>
}

/// The profile's browser, open for its person, in the state its update is in: its steps while it starts, then the
/// browser as it is, with its own address bar and tabs, then Save.
function Updating(props: { profile: Profile; subject?: string }) {
  const p = () => props.profile
  const u = () => p().updating!
  const subject = { subject: props.subject }
  const [busy, setBusy] = createSignal<"save" | "discard">()
  const finish = async (tool: "save" | "discard") => {
    setBusy(tool)
    await call(`chrome-profiles_${tool}`, { profile: p().profile }, subject)
      .then(() => toast(tool === "save" ? `Saved ${p().profile}` : `Closed ${p().profile} without saving`))
      // A save that fails says so on the update itself, with Try Again.
      .catch(tool === "save" ? () => {} : toast)
    setBusy()
  }
  const again = () => call("chrome-profiles_update", { profile: p().profile }, subject).catch(toast)
  const session = u().session
  // One function for as long as it's ready, so the list asked again doesn't connect to it again.
  const screen = (inputs: { action: string }) => call("chrome_screen", { session, ...inputs }, subject)
  return (
    <Section heading="Updating">
      <LiveScreen label="The browser" screen={u().state === "ready" ? screen : undefined}>
        <Show when={u().state === "failed"}>
          <Problem heading="Couldn't start the browser" text={u().why} action={<Button variant="outline" onPress={again}>Try Again</Button>} />
        </Show>
        <Show when={u().state === "starting" || u().state === "opening"}>
          <Steps
            steps={["Getting a browser ready", `Opening ${p().profile}`]}
            at={u().state === "starting" ? 0 : 1}
            text={
              u().state === "starting" ? (
                <Loading>
                  <Building subject={props.subject} />
                </Loading>
              ) : undefined
            }
          />
        </Show>
        <Show when={u().state === "saving" || u().state === "discarding"}>
          <Status heading={u().state === "saving" ? `Saving ${p().profile}…` : "Closing the browser…"} text={u().stage} />
        </Show>
      </LiveScreen>
      <Show when={u().state === "ready" && u().why}>
        {/* The first paragraph is what its person reads; why it failed is under Show Details. */}
        <Problem
          heading={`Couldn't save ${p().profile}`}
          text={`Everything in its browser is kept, so nothing is lost.\n\n${u().why}`}
          action={<Button variant="outline" loading={busy() === "save"} disabled={!!busy()} onPress={() => finish("save")}>Try Again</Button>}
        />
      </Show>
      <Show when={u().state !== "failed" && !(u().state === "ready" && u().why)}>
        <Text style={look.note}>Use it as you would Chrome: sign in to any sites you like. It's saved by itself at {time(u().until)}.</Text>
      </Show>
      <View style={look.buttons}>
        <Show when={u().state !== "failed" && !u().why}>
          <Button loading={busy() === "save" || u().state === "saving"} disabled={u().state !== "ready" || !!busy()} onPress={() => finish("save")}>
            Save
          </Button>
        </Show>
        <Button variant="outline" loading={busy() === "discard" || u().state === "discarding"} disabled={!["ready", "failed"].includes(u().state) || !!busy()} onPress={() => finish("discard")}>
          Discard
        </Button>
      </View>
    </Section>
  )
}

/// Opens the profile's browser for its person to update.
function Update(props: { profile: string; subject?: string }) {
  const [busy, setBusy] = createSignal(false)
  const update = async () => {
    setBusy(true)
    await call("chrome-profiles_update", { profile: props.profile }, { subject: props.subject }).catch(toast)
    setBusy(false)
  }
  return <Button loading={busy()} onPress={update}>Update Profile</Button>
}

/// A new name for the profile.
function Rename(props: Props & { asking: boolean; done: () => void }) {
  const [busy, setBusy] = createSignal(false)
  const [typed, setTyped] = createSignal(props.item)
  const send = async () => {
    const name = typed().trim()
    if (!name) return
    setBusy(true)
    await call("chrome-profiles_rename", { profile: props.item, name }, { subject: props.subject })
      .then(() => (props.done(), props.open?.(name)))
      .catch(toast)
    setBusy(false)
  }
  return (
    <Sheet heading="Rename Profile" open={props.asking} onClose={props.done}>
      <Field label="Name">
        <Input required autoFocus value={typed()} onChangeText={setTyped} onSubmitEditing={send} />
      </Field>
      <Button loading={busy()} onPress={send}>Rename</Button>
    </Sheet>
  )
}

/// The profile: updating it, then what it was last saved with.
export default function ChromeProfile(props: Props) {
  const profile = useProfile(props)
  const [asking, setAsking] = createSignal(false)
  const act = async (value: string) => {
    if (value === "rename") return setAsking(true)
    const yes = await confirm({ heading: `Delete ${props.item}?`, text: "It's in Recently Deleted for 30 days, then gone, with everything signed in on it.", action: "Delete", destructive: true })
    if (yes) await call("chrome-profiles_remove", { profile: props.item }, { subject: props.subject }).then(() => props.close?.()).catch(toast)
  }
  return (
    <Screen
      heading={props.item}
      back
      action={
        <Menu.Root onSelect={act}>
          <Menu.Trigger aria-label={`More for ${props.item}`} />
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
      <Show when={profile()}>
        {(p) => (
          <>
            <Show when={p().updating?.session} keyed fallback={<Update profile={p().profile} subject={props.subject} />}>
              <Updating profile={p()} subject={props.subject} />
            </Show>
            <Section heading="Extensions">
              <Show when={p().extensions.length} fallback={<Empty>No extensions.</Empty>}>
                <List>
                  <For each={p().extensions} keyed={(e) => e.id}>
                    {(e) => <Row start={<Image src={e().icon ?? ""} alt="" style={look.extension} />} heading={e().name} />}
                  </For>
                </List>
              </Show>
            </Section>
            <Section heading="History">
              <Show when={p().history.length} fallback={<Empty>{p().saved ? "Nothing visited yet." : "Not saved yet."}</Empty>}>
                <List>
                  <For each={p().history} keyed={(h) => `${h.url} ${h.visited}`}>
                    {(h) => <Row heading={h().title || site(h().url)} text={`${site(h().url)} · ${since(h().visited)}`} />}
                  </For>
                </List>
              </Show>
            </Section>
          </>
        )}
      </Show>
      <Rename {...props} asking={asking()} done={() => setAsking(false)} />
    </Screen>
  )
}
