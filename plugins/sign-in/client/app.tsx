// Devices: everything that acts as whoever's looking, and the servers that run their apps, each saying whether an
// update is ready for it: the build itself is for agents, in `builds`. One that's behind the latest is updated here, and all of them at once; this device becomes a server and
// stops being one; a server's services are switched on and off; and a device is taken out, or this one signs out.
// An update a server is taking says each stage as the server says it (`OneCore.Release`), and one that didn't take
// is a problem on it, until one does.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, client, confirm, Empty, leave, List, Problem, Row, Screen, Section, seen, Skeleton, Spinner, Split, Steps, Switch, toast } from "ui"
import { on } from "./mine.tsx"
import "./app.css"

/** A build a machine is on, as `builds` answers it. */
interface Built {
  build?: string
  target?: string
}

/** Where an update of a server is, as the server says it: what's under way, in this order. */
type Stage = "downloading" | "restarting" | "starting"

/** Whether something is on, as `builds` and `sign-in_devices` answer it: a server says when it's restarting too. */
type Seen = "on" | "restarting" | number | string | null

/** What `builds` answers. */
interface Builds {
  devices: (Built & { device: string })[]
  latest: Record<string, { build: string }>
  servers: (Built & { server: string; name?: string; installed?: boolean; seen?: Seen; update?: Stage | null; went_back?: string | null })[]
}

/** A plugin's service a server may run, as `services` answers it. */
interface Service {
  plugin: string
  service: string
  called: string
  needs?: string
  servers: string[]
}

/** A device or a server, as a row says it. */
interface Thing {
  id: string
  name?: string
  kind?: "app" | "assistant" | "browser"
  here: boolean
  added?: string
  seen?: Seen
  build?: string
  behind?: boolean
  /** Where an update of it is, while one's under way. */
  update?: Stage | null
  /** Whether its last update didn't take, and it's still behind. */
  failed?: boolean
  subject?: string
  services?: (Service & { on: boolean })[]
}

const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { subject: who }).then(JSON.parse)

/// What's known of the devices and servers, each as a row says it. Whoever owns the network sees its servers too.
const mine = createMemo(async () => {
  const [{ devices }, here, builds, network, services, networkServices] = await Promise.all([
    json<{ devices: Omit<Thing, "here">[] }>("sign-in_devices"),
    client(),
    json<Builds>("builds"),
    json<Builds>("builds", {}, "network").catch(() => undefined),
    json<{ services: Service[] }>("services"),
    json<{ services: Service[] }>("services", {}, "network").catch(() => undefined),
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
      // Only a server that was installed takes the latest build when it's asked.
      const updates = server.installed ? behind("server", server) : undefined
      const failed = !!(updates && !server.update && server.went_back)
      servers.set(server.server, { ...server, id: server.server, subject: who, here: server.server === here.server, behind: updates, failed, services: runs })
    }
  }
  return { devices: rows, servers: [...servers.values()].toSorted((a, b) => Number(b.here) - Number(a.here)), here }
})
changed(() => refresh(mine))

/// Whether something is on, or when it was last: nothing for one never seen. This device says only that it's this one.
const presence = (thing: Thing) => (thing.here ? "This device" : thing.seen === "on" ? "Online" : thing.seen ? seen(thing.seen) : undefined)

/// What each stage of an update says, as a step and on its row.
const stages: Record<Stage, { step: string; row: string }> = {
  downloading: { step: "Downloading the update", row: "Downloading update…" },
  restarting: { step: "Restarting", row: "Restarting…" },
  starting: { step: "Starting the new version", row: "Starting…" },
}

/// What's under way for something, as its row says it beside a spinner: an update, a server restarting, or this device
/// updating itself or becoming a server or stopping being one.
function underway(thing: Thing) {
  if (thing.update) return stages[thing.update].row
  if (thing.seen === "restarting") return "Restarting…"
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
  return [what, thing.failed ? "Couldn't update" : thing.behind && "Update ready"].filter(Boolean).join(" · ")
}

/// Does something, says why if it couldn't be done, and shows everything as it is after. → whether it was done.
async function doing(does: () => Promise<unknown>) {
  try {
    await does()
    return true
  } catch (error) {
    toast((error as Error).message)
  } finally {
    refresh(mine)
  }
}

/// Has a server take the latest builds, or this device: `one update`.
const update = (thing: Thing) => (thing.kind ? client({ update: true }).then(({ message }) => toast(message)) : call("update", { server: thing.id }, { subject: thing.subject }))

/// Takes a device out once its person has said so. Taking this one out is signing out: what it kept of theirs goes,
/// and the client opens again where a device that isn't in does. Another that's taken out shows its person where to
/// sign in when it's next on, and keeps what it kept of theirs until someone else signs in on it (`Session` in
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

/// A device or a server, opened: Update when one's ready, this device's server, its services, and taking it out.
/** Everything Devices shows. */
type All = Awaited<ReturnType<typeof mine>>

function Detail(props: { thing: Thing; all: All }) {
  const thing = () => props.thing
  const server = () => (thing().kind ? props.all.servers.find((server) => server.here && thing().here) : thing())
  const serve = async (now: boolean) => {
    setServing(now)
    await doing(() => client({ serve: now }).then(({ message }) => toast(message)))
    setServing()
  }
  const stage = () => thing().update
  return (
    <Screen heading={thing().name ?? "Server"} back>
      <Show when={stage()}>
        <Section heading="Updating">
          <div class="updating">
            <Steps
              steps={Object.values(stages).map(({ step }) => step)}
              at={Object.keys(stages).indexOf(stage()!)}
              text={stage() === "starting" ? "If it doesn't start, it goes back to the version it had." : undefined}
            />
          </div>
        </Section>
      </Show>
      <Show when={thing().failed}>
        <Problem
          heading="Couldn't update"
          text="It's still on the version it had."
          action={<Button variant="outline" loading={working() === thing().id} onClick={press(thing(), update)}>Try Again</Button>}
        />
      </Show>
      <List>
        <Show when={thing().behind && !stage() && !thing().failed}>
          <Row
            heading="Update ready"
            end={
              <Show when={!thing().kind || thing().here}>
                <Button variant="outline" loading={working() === thing().id} onClick={press(thing(), update)}>Update</Button>
              </Show>
            }
          />
        </Show>
        <Show when={thing().here && thing().kind === "app"}>
          <Row
            heading="Server"
            text={serving() === undefined ? "Runs your apps from this device whenever it's on." : <><Spinner /> {serving() ? "Starting…" : "Stopping…"}</>}
            end={<Switch aria-label="Server" checked={serving() ?? !!props.all.here.server} disabled={serving() !== undefined} onChange={(event) => serve(event.currentTarget.checked)} />}
          />
        </Show>
      </List>
      <Show when={server()?.services?.length}>
        <Section heading="Services">
          <List>
            <For each={server()!.services!} keyed={(service) => `${service.plugin}/${service.service}`}>
              {(service) => (
                <Row
                  heading={service().called}
                  text={service().needs}
                  end={
                    <Switch
                      aria-label={service().called}
                      checked={service().on}
                      onChange={(event) => doing(() => call(event.currentTarget.checked ? "install_service" : "uninstall_service", { plugin: service().plugin, service: service().service, server: server()!.id }, { subject: server()!.subject }))}
                    />
                  }
                />
              )}
            </For>
          </List>
        </Section>
      </Show>
      <div class="readable">
        <Show when={thing().kind} fallback={<Button variant="destructive" onClick={() => removeServer(thing())}>Remove</Button>}>
          <Button variant={thing().here ? "outline" : "destructive"} onClick={() => remove(thing())}>{thing().here ? "Sign Out" : "Remove"}</Button>
        </Show>
      </div>
    </Screen>
  )
}

function Devices(props: { all: All }) {
  const opened = () => [...props.all.devices, ...props.all.servers].find((thing) => thing.id === open())
  // What's behind and can be updated from here, with no update under way already: every server, and this device, last.
  const behind = () => [...props.all.servers, ...props.all.devices.filter((device) => device.here)].filter((thing) => thing.behind && !thing.update)
  const all = async () => {
    setWorking("all")
    for (const thing of behind()) await doing(() => update(thing))
    setWorking()
  }
  const row = (thing: () => Thing) => <Row heading={thing().name ?? "Server"} text={about(thing())} current={open() === thing().id} onClick={() => setOpen(thing().id)} />
  return (
    <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()}><Detail thing={opened()!} all={props.all} /></Show>}>
      <Screen heading="Devices" back onBack={leave} action={behind().length ? <Button loading={working() === "all"} onClick={all}>Update All</Button> : undefined}>
        <Show when={props.all.devices.length} fallback={<Empty>No connected apps yet.</Empty>}>
          <List>
            <For each={props.all.devices} keyed={(device) => device.id}>{row}</For>
          </List>
        </Show>
        <Show when={props.all.servers.length}>
          <Section heading="Servers">
            <List>
              <For each={props.all.servers} keyed={(server) => server.id}>{row}</For>
            </List>
          </Section>
        </Show>
      </Screen>
    </Split>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Devices" back onBack={leave}><Problem heading="Can't show your devices" text={String((error() as Error).message ?? error())} /></Screen>}>
      <Loading fallback={<Screen heading="Devices" back onBack={leave}><Skeleton rows={3} /></Screen>}>
        <Devices all={mine()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
