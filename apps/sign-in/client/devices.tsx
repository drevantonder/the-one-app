// Devices: everything that acts as whoever's looking, and the servers that run their apps. This device says whether an
// update is ready for it, and is updated here; its page says its build under About. The servers take each
// build by themselves, as the network's update says (`core/edge/updater/updater.ts`), which Software Update heads
// them with, in Apple's words: up to date, updating, waiting for a quiet moment (Update Now), or failed (Try Again).
// This device becomes a server and stops being one; a server's services are switched on and off; and a device is
// taken out, or this one signs out. Each server says whether it's ready, active and running, or why not, and an
// update it's taking as it moves on (`OneCore.Release`). This device becoming a server, or stopping being one, says
// each stage. Above them all are the devices waiting to sign in as whoever's looking, each let in by typing the
// number it shows, as Microsoft Authenticator has it, or blocked.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { Badge, Button, call, changed, client, confirm, type Element, Empty, Field, haptic, Icon, Input, link, List, Problem, Row, Screen, Section, seen, Sheet, since, Skeleton, Spinner, Split, Steps, styles, Switch, Text, toast, tokens, View } from "ui"
import { on } from "./mine.tsx"

/** A build a machine is on, as `builds` answers it. */
interface Built {
  build?: string
  target?: string
}

/** Where an update of a server is, as the server says it. */
type ServerUpdateStatus = "up_to_date" | "downloading" | "checked" | "verifying"

/** What a server is, as `builds` answers it: whether it's one of the network's, its program, and whether it's ready. */
interface ServerStatus {
  server_membership?: "inactive" | "joining" | "active" | "leaving"
  server_daemon_status?: "starting" | "running" | "stopping" | "stopped"
  server_ready?: boolean
  server_ready_reason?: "joining" | "starting" | "draining" | "stopped"
  server_update_status?: ServerUpdateStatus
  went_back?: string | null
}

/** The network's update, as its updater says it (`builds`' `network`). */
interface NetworkUpdate {
  network_update_status: "up_to_date" | "preparing" | "restarting" | "held"
  build: string | null
  done: number
  of: number
  waiting_for: "quiet" | null
  held: { name?: string; why: string } | null
}

/** What `builds` answers. */
interface Builds {
  network?: NetworkUpdate | null
  devices: (Built & { device: string })[]
  latest: Record<string, { build: string }>
  servers: (Built & ServerStatus & { server: string; name?: string; installed?: boolean; last_seen?: number })[]
}

/** An app's service a server may run, as `services` answers it. */
interface Service {
  app: string
  service: string
  called: string
  needs?: string
  /** The systems it runs on, as npm's `os` names them; nothing for any. */
  os?: string[] | null
  servers: string[]
}

/** A device or a server, as a row says it. */
interface Thing extends ServerStatus {
  id: string
  name?: string
  kind?: "app" | "assistant" | "browser"
  here: boolean
  added?: string
  connected?: boolean
  last_seen?: number | string
  build?: string
  behind?: boolean
  subject?: string
  target?: string
  services?: (Service & { on: boolean })[]
}

/** A device waiting to sign in as whoever's looking, as `sign-in_waiting` lists it. */
interface Waiting {
  request: string
  device: string
  name: string
  at: string
  place: string | null
}

/** What Settings draws of what other apps add to a place a section offers: a device's page (`settings.sections`). */
type Groups = (props: { in: "device"; device: string; kind: string; server?: string; services?: string[] }) => Element

const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { subject: who }).then(JSON.parse)

/// What's known of the devices and servers, each as a row says it. Whoever owns the network sees its servers too.
const mine = createMemo(async () => {
  const [{ devices }, { waiting }, here, builds, network, services, networkServices] = await Promise.all([
    json<{ devices: Omit<Thing, "here">[] }>("sign-in_devices"),
    json<{ waiting: Waiting[] }>("sign-in_waiting"),
    client(),
    json<Builds>("builds"),
    json<Builds>("builds", {}, "network"),
    json<{ services: Service[] }>("services"),
    json<{ services: Service[] }>("services", {}, "network"),
  ])
  const latest = { ...network?.latest, ...builds.latest }
  /// Whether something is behind the latest build for its kind of machine, or `undefined` where there's none.
  const behind = (kind: string, built?: Built) => {
    const out = built?.build && latest[`${kind}/${built.target}`]
    return out ? out.build !== built.build : undefined
  }
  const built = new Map(builds.devices.map((built) => [built.device, built]))
  const rows = devices
    .map((device): Thing => {
      const its = device.id === here.device ? { build: here.build, target: built.get(device.id)?.target } : built.get(device.id)
      return { ...device, here: device.id === here.device, build: its?.build, behind: behind("one", its) }
    })
    .toSorted((a, b) => Number(b.here) - Number(a.here))
  const servers = new Map<string, Thing>()
  for (const [list, theirs, who] of [[builds, services, undefined], [network, networkServices, "network"]] as const) {
    for (const server of list?.servers ?? []) {
      if (servers.has(server.server)) continue
      const runs = (theirs?.services ?? []).map((service) => ({ ...service, on: service.servers.includes(server.server) }))
      servers.set(server.server, { ...server, id: server.server, subject: who, here: server.server === here.server?.key, services: runs })
    }
  }
  const update = network?.network ?? builds.network ?? undefined
  return { waiting, devices: rows, servers: [...servers.values()].toSorted((a, b) => Number(b.here) - Number(a.here)), here, update, who: network ? "network" : undefined }
})
changed(() => refresh(mine))

/// Whether an update of a server is under way.
const updating = (server: ServerStatus) => !!server.server_update_status && server.server_update_status !== "up_to_date"

/// Whether something is connected, or when it was last: nothing for one never seen. A server says whether it's
/// ready, and why not. This device says only that it's this one.
function presence(thing: Thing) {
  if (thing.here) return "This device"
  if (thing.kind === undefined && thing.server_ready !== undefined && thing.server_ready_reason !== "stopped")
    return thing.server_ready ? "Ready" : `Not ready · ${capital(thing.server_ready_reason!)}`
  return thing.connected ? "Online" : thing.last_seen ? seen(thing.last_seen) : undefined
}

const capital = (word: string) => word[0].toUpperCase() + word.slice(1)

/// Each stage of a server's update, as a step and on its row.
const updates: Record<Exclude<ServerUpdateStatus, "up_to_date">, { step: string; row: string }> = {
  downloading: { step: "Downloading the update", row: "Downloading update…" },
  checked: { step: "Ready to install", row: "Ready to install" },
  verifying: { step: "Installing", row: "Installing…" },
}

/// The network's update, as Software Update says it: its line, and whether it's under way.
function softwareUpdate(update: NetworkUpdate) {
  if (update.network_update_status === "up_to_date") return { text: "Up to date" }
  if (update.network_update_status === "held") return { text: `Update failed · ${update.held?.name ?? "A server"}: ${update.held?.why}`, action: "Try Again" }
  if (update.waiting_for === "quiet") return { text: "Waiting for a quiet moment", action: "Update Now" }
  return { text: `Updating… ${update.done} of ${update.of}`, busy: true }
}

/// Each stage of this device becoming a server, and of stopping being one, as a step.
const joining = { registering: "Registering", downloading: "Downloading", installing: "Installing", starting: "Starting" }
const leaving = { draining: "Draining", deregistering: "Deregistering", uninstalling: "Uninstalling" }

/// What's under way for something, as its row says it beside a spinner: an update, a server starting or stopping,
/// or this device updating itself or becoming a server or stopping being one.
function underway(thing: Thing) {
  if (thing.server_update_status === "downloading" || thing.server_update_status === "verifying") return updates[thing.server_update_status].row
  if (thing.server_daemon_status === "starting" || thing.server_daemon_status === "stopping") return `${capital(thing.server_daemon_status)}…`
  if (thing.here && thing.kind && working() === thing.id) return "Downloading update…"
  if (thing.here && thing.kind === "app" && serving() !== undefined) return serving() ? "Starting its server…" : "Stopping its server…"
}

/// A device's or a server's grey line: what's under way for it, or whether it's on, and that an update is ready for
/// it, or didn't take.
function about(thing: Thing) {
  const under = underway(thing)
  if (under) return <><Spinner /> {under}</>
  const what = {
    app: () => presence(thing) ?? `Approved ${on(thing.added!)}`,
    assistant: () => `An assistant, approved ${on(thing.added!)}`,
    browser: () => (thing.here ? "This device" : `A browser, signed in ${on(thing.added!)}`),
    server: () => presence(thing),
  }[thing.kind ?? "server"]()
  const checked = thing.server_update_status === "checked" && updates.checked.row
  return [what, checked].filter(Boolean).join(" · ")
}

/// Does something, says why if it couldn't be done, and shows everything as it is after. → whether it was done.
async function doing(does: () => Promise<unknown>) {
  try {
    await does()
    return true
  } catch (error) {
    toast(error as Error)
  } finally {
    refresh(mine)
  }
}

/// Has this device take the latest build: `one update`.
const update = () => client({ update: true }).then(({ message }) => toast(message))

/// Update Now, or Try Again: the network's update takes every server to the latest build now (`update`).
const updateNow = (who?: string) => call("update", {}, { subject: who })

/// Takes a device out once its person has said so. Taking this one out is signing out: what it kept of theirs goes,
/// and the client opens again where a device that isn't in does. Another that's taken out shows its person where to
/// sign in when it's next on, and keeps what it kept of theirs until someone else signs in on it (`State` in
/// core/client).
async function remove(device: Thing) {
  const question = device.here
    ? { heading: "Sign out of this device?", text: "It stops acting as you, and has to be approved again.", action: "Sign Out" }
    : { heading: `Remove ${device.name}?`, text: "It stops acting as you at once. What it kept of yours is erased if someone else signs in on it.", action: "Remove" }
  if (!(await confirm({ ...question, destructive: true }))) return
  if (await doing(() => call("sign-in_remove_device", { device: device.id })) && device.here) client({ leave: true })
}

/// Takes a server off the list of whoever it acts for. It no longer runs their apps.
async function removeServer(server: Thing) {
  if (!(await confirm({ heading: `Remove ${server.name ?? "this server"}?`, text: "It stops running your apps. Another server takes its work.", action: "Remove", destructive: true }))) return
  doing(() => call("remove_server", { key: server.id }, { subject: server.subject }))
}

const [open, setOpen] = createSignal<string>()
const [working, setWorking] = createSignal<string>()
/// Whether this device is becoming a server (`true`) or stopping being one (`false`), while it is.
const [serving, setServing] = createSignal<boolean>()

/// What `does` does to `thing`, shown on its button while it's under way.
const press = (thing: Thing, does: (thing: Thing) => Promise<unknown>) => async () => {
  setWorking(thing.id)
  await doing(() => does(thing))
  setWorking()
}

const look = styles({
  /// The steps of an update under way, in a box of their own where the update's row was.
  updating: {
    maxWidth: tokens.length.readable,
    paddingBlock: tokens.grid[4],
    borderWidth: tokens.length.hairline,
    borderColor: tokens.colour.quinaryLabel,
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.background,
  },
  /// What isn't a list stops at the width a list does, so a button isn't a line across a wide window.
  readable: { gap: tokens.length.gutter, maxWidth: tokens.length.readable },
  answer: { gap: tokens.length.gutter },
  muted: { color: tokens.colour.secondaryLabel },
  glyph: { width: tokens.length.control, height: tokens.length.control, alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.small, backgroundColor: tokens.colour.quinaryLabel, color: tokens.colour.secondaryLabel },
  /// The number typed, big as figures are.
  number: { height: "auto", paddingBlock: tokens.grid[4], font: tokens.type.largeTitle, fontVariantNumeric: "tabular-nums", textAlign: "center" },
})

/// A device or a server, opened: Update when one's ready, this device's server, its services, and taking it out.
/** Everything Devices shows. */
type All = Awaited<ReturnType<typeof mine>>

function Detail(props: { thing: Thing; all: All; groups?: Groups }) {
  const thing = () => props.thing
  const server = () => (thing().kind ? props.all.servers.find((server) => server.here && thing().here) : thing())
  const serve = async (now: boolean) => {
    setServing(now)
    await doing(() => client({ serve: now }).then(({ message }) => toast(message)))
    setServing()
  }
  // A service that runs on some systems only shows on servers of those.
  const runnable = () => (server()?.services ?? []).filter((service) => !service.os?.length || !system(server()!.target) || service.os.includes(system(server()!.target)!))
  const stage = () => (updating(thing()) ? (thing().server_update_status as keyof typeof updates) : undefined)
  // This device becoming a server, or stopping being one, at the stage the app says.
  const membership = () => (thing().here && thing().kind === "app" ? props.all.here.server : undefined)
  const steps = () => (membership()?.server_membership === "joining" ? joining : membership()?.server_membership === "leaving" ? leaving : undefined)
  return (
    <Screen heading={thing().name ?? "Server"} back>
      <Show when={stage()}>
        <Section heading="Updating">
          <View style={look.updating}>
            <Steps
              steps={Object.values(updates).map(({ step }) => step)}
              at={Object.keys(updates).indexOf(stage()!)}
              text={stage() === "verifying" ? "If it doesn't start, it goes back to the version it had." : stage() === "checked" ? "It installs when Software Update gets to it." : undefined}
            />
          </View>
        </Section>
      </Show>
      <Show when={steps()}>
        <Section heading={membership()!.server_membership === "joining" ? "Becoming a server" : "Stopping being a server"}>
          <View style={look.updating}>
            <Steps steps={Object.values(steps()!)} at={Math.max(0, Object.keys(steps()!).indexOf(membership()!.server_membership_stage ?? ""))} />
          </View>
        </Section>
      </Show>
      <Show when={!thing().kind && thing().server_membership && thing().server_ready_reason === "stopped"}>
        <Problem warn heading="This server isn't running" text={thing().last_seen ? `It was last seen ${seen(thing().last_seen!)}.` : "It hasn't been seen yet."} />
      </Show>
      <Show when={(thing().behind && thing().here) || (thing().here && thing().kind === "app")}>
        <Section heading="Options">
          <List>
            <Show when={thing().behind && thing().here}>
              <Row heading="Update ready" end={<Button variant="outline" loading={working() === thing().id} onPress={press(thing(), update)}>Update</Button>} />
            </Show>
            <Show when={thing().here && thing().kind === "app"}>
              <Row
                heading="Server"
                text={serving() === undefined ? "Runs your apps from this device whenever it's on." : <><Spinner /> {serving() ? "Starting…" : "Stopping…"}</>}
                end={<Switch aria-label="Server" value={serving() ?? !!props.all.here.server} disabled={serving() !== undefined} onValueChange={serve} />}
              />
            </Show>
          </List>
        </Section>
      </Show>
      <Show when={runnable().length}>
        <Section heading="Services">
          <List>
            <For each={runnable()} keyed={(service) => `${service.app}/${service.service}`}>
              {(service) => (
                <Row
                  heading={service().called}
                  end={
                    <Switch
                      aria-label={service().called}
                      value={service().on}
                      onValueChange={async (on) => {
                        // What it needs of the machine is read as it's switched on, where its owner decides.
                        if (on && !(await confirm({ heading: `Switch on ${service().called}?`, text: service().needs, action: "Switch On" }))) return refresh(mine)
                        doing(() => call(on ? "install_service" : "uninstall_service", { app: service().app, service: service().service, server: server()!.id }, { subject: server()!.subject }))
                      }}
                    />
                  }
                />
              )}
            </For>
          </List>
        </Section>
      </Show>
      <Show when={props.groups} keyed>
        {(Groups) => (
          <Groups
            in="device"
            device={thing().id}
            kind={thing().kind ?? "server"}
            server={server()?.id}
            services={server()?.services?.filter((service) => service.on).map((service) => service.app)}
          />
        )}
      </Show>
      <Section heading="About">
        <List>
          <Row heading="Kind" end={<Text style={look.muted}>{kinds[thing().kind ?? "server"]}</Text>} />
          <Show when={presence(thing())}>{(seen) => <Row heading="Status" end={<Text style={look.muted}>{seen()}</Text>} />}</Show>
          <Show when={thing().build}>{(build) => <Row heading="Build" end={<Text style={look.muted}>{build().split("+")[0]}</Text>} />}</Show>
        </List>
      </Section>
      <List>
        <Show when={thing().kind} fallback={<Row heading="Remove Server…" text="It stops running your apps." destructive onPress={() => removeServer(thing())} />}>
          <Row
            heading={thing().here ? "Sign Out…" : "Remove Device…"}
            text={thing().here ? "It stops acting as you." : "Signs it out of your network."}
            destructive
            onPress={() => remove(thing())}
          />
        </Show>
      </List>
    </Screen>
  )
}

/// What kind of thing a device is, as About says it.
const kinds = { app: "the One App", assistant: "Assistant", browser: "Browser", server: "Server" }

/// The device waiting to sign in that's being answered, by its request: tapping its row, or a notification's link to
/// it (`#waiting=<request>`), opens it.
const [answering, setAnswering] = createSignal<string | undefined>(() => /(?:^|&)waiting=([^&]+)/.exec(link() ?? "")?.[1])

/// The person's answer to a device waiting to sign in as them: the number they typed, which lets it in if it's the
/// one it shows and blocks it if not, or Not Me, which blocks it. Once it's in, this device hands it its person's key if
/// it holds it, as `encryption_approve` does, which a device without it, and a browser, leave.
async function answer(device: Waiting, number?: number) {
  setAnswering()
  let unlocked = false
  const done = await doing(async () => {
    if (number === undefined) return call("sign-in_deny", { request: device.request })
    await call("sign-in_approve", { request: device.request, number })
    if (JSON.parse(await call("encryption_key")).slot == null) return
    await call("encryption_approve", { device: device.device, name: device.name })
    unlocked = true
  })
  if (!done) return haptic("error")
  haptic("success")
  toast(number === undefined ? `${device.name} was blocked.` : `${device.name} is signed in as you${unlocked ? ", with your key" : ""}.`)
}

/// Is this you signing in? The number the device shows, typed, as Microsoft Authenticator asks for it: picking one of
/// a few could be a lucky guess. It's typed, never filled in: the device's one-time-code autofill is off. It takes
/// two digits and nothing else, big as figures are. Then Yes, or Not Me.
function Answer(props: { device?: Waiting }) {
  return (
    <Sheet heading="Is this you signing in?" open={!!props.device} onClose={() => setAnswering()}>
      <Show when={props.device}>
        {(device) => {
          // Set again with every key, so what isn't a digit, or is a third, is taken out of the field as it's typed.
          const [number, setNumber] = createSignal("", { equals: false })
          const [error, setError] = createSignal<string>()
          const yes = () => (number().length === 2 ? answer(device(), Number(number())) : setError(`Enter the 2 digits showing on ${device().name}.`))
          return (
            <View style={look.answer}>
              <Field label={`Enter the number showing on ${device().name}`} error={error()}>
                <Input
                  name="number"
                  inputMode="numeric"
                  autoComplete="off"
                  required
                  autoFocus
                  value={number()}
                  onChangeText={(text) => (setNumber(text.replace(/[^0-9]/g, "").slice(0, 2)), setError())}
                  onSubmitEditing={yes}
                  style={look.number}
                />
              </Field>
              <Button onPress={yes}>Yes</Button>
              <Button variant="outline" onPress={() => answer(device())}>Not Me</Button>
            </View>
          )
        }}
      </Show>
    </Sheet>
  )
}

/// A build's target, `macos-aarch64`, as npm names its system: `darwin`.
const system = (target?: string) => ({ linux: "linux", macos: "darwin", windows: "win32" })[target?.split("-")[0] as "linux"]

/// What kind of thing a device is, by its glyph in a well, as Raycast's Settings draws one: an app on a device, a
/// browser, an assistant or a server.
const glyphs = { app: "devices", browser: "globe", assistant: "robot", server: "hard-drives" } as const
const Glyph = (props: { thing: Thing }) => (
  <View style={look.glyph}>
    <Icon name={glyphs[props.thing.kind ?? "server"]} />
  </View>
)

function Things(props: { all: All; groups?: Groups }) {
  const opened = () => [...props.all.devices, ...props.all.servers].find((thing) => thing.id === open())
  const row = (thing: () => Thing) => (
    <Row
      start={<Glyph thing={thing()} />}
      heading={thing().name ?? "Server"}
      text={about(thing())}
      end={<Show when={thing().behind && !underway(thing())}><Badge variant="secondary">Update</Badge></Show>}
      current={open() === thing().id}
      onPress={() => setOpen(thing().id)}
    />
  )
  const said = () => props.all.update && softwareUpdate(props.all.update)
  return (
    <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()}><Detail thing={opened()!} all={props.all} groups={props.groups} /></Show>}>
      <Screen heading="Devices" back>
        <Show when={props.all.waiting.length}>
          <Section heading="Waiting to sign in">
            <List>
              <For each={props.all.waiting} keyed={(device) => device.request}>
                {(device) => (
                  <Row
                    heading={`${device().name} · the One App`}
                    text={[since(device().at), device().place && `near ${device().place}`].filter(Boolean).join(" · ")}
                    current={answering() === device().request}
                    onPress={() => setAnswering(device().request)}
                  />
                )}
              </For>
            </List>
          </Section>
        </Show>
        <Answer device={props.all.waiting.find((device) => device.request === answering())} />
        <Section heading="Your Devices">
          <Show when={props.all.devices.length} fallback={<Empty>No devices yet.</Empty>}>
            <List>
              <For each={props.all.devices} keyed={(device) => device.id}>{row}</For>
            </List>
          </Show>
        </Section>
        <Show when={props.all.servers.length}>
          <Section heading="Servers">
            <List>
              <Show when={said()}>
                <Row
                  start={<View style={look.glyph}><Icon name="gear-six" /></View>}
                  heading="Software Update"
                  text={said()!.busy ? <><Spinner /> {said()!.text}</> : said()!.text}
                  end={
                    <Show when={said()!.action}>
                      <Button variant="outline" loading={working() === "network"} onPress={press({ id: "network", here: false }, () => updateNow(props.all.who))}>
                        {said()!.action}
                      </Button>
                    </Show>
                  }
                />
              </Show>
              <For each={props.all.servers} keyed={(server) => server.id}>{row}</For>
            </List>
          </Section>
        </Show>
      </Screen>
    </Split>
  )
}

/// Devices, as Settings draws it, with what other apps add to a device's page (`groups`).
export default function Devices(props: { groups?: Groups }) {
  return (
  <Errored fallback={(error) => <Screen heading="Devices" back><Problem heading="Can't show your devices" text={error() as Error} /></Screen>}>
    <Loading fallback={<Screen heading="Devices" back><Skeleton rows={3} /></Screen>}>
      <Things all={mine()} groups={props.groups} />
    </Loading>
  </Errored>
)
}
