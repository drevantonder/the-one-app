// Devices: everything that acts as whoever's looking, and the servers that run their apps, each saying whether an
// update is ready for it: the build itself is for agents, in `builds`. One that's behind the latest is updated here, and all of them at once; this device becomes a server and
// stops being one; a server's services are switched on and off; and a device is taken out, or this one signs out.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, client, confirm, Empty, leave, List, Problem, Row, Screen, Section, seen, Skeleton, Split, Switch, toast } from "ui"
import { on } from "./mine.tsx"
import "./app.css"

/** A build a machine is on, as `builds` answers it. */
interface Built {
  build?: string
  target?: string
}

/** What `builds` answers. */
interface Builds {
  devices: (Built & { device: string })[]
  latest: Record<string, { build: string }>
  servers: (Built & { server: string; name?: string; installed?: boolean; seen?: "on" | number | null })[]
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
  seen?: "on" | number | null
  build?: string
  behind?: boolean
  for?: string
  services?: (Service & { on: boolean })[]
}

const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { for: who }).then(JSON.parse)

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
      servers.set(server.server, { ...server, id: server.server, for: who, here: server.server === here.server, behind: updates, services: runs })
    }
  }
  return { devices: rows, servers: [...servers.values()].toSorted((a, b) => Number(b.here) - Number(a.here)), here }
})
changed(() => refresh(mine))

/// Whether something is on, or when it was last: nothing for one never seen. This device says only that it's this one.
const presence = (thing: Thing) => (thing.here ? "This device" : thing.seen === "on" ? "Online" : thing.seen ? seen(thing.seen) : undefined)
/// A device's or a server's grey line: whether it's on, and that an update is ready for it.
function about(thing: Thing) {
  const what = {
    app: () => presence(thing) ?? `Approved ${on(thing.added!)}`,
    assistant: () => `An assistant, approved ${on(thing.added!)}`,
    browser: () => (thing.here ? "This device" : `A browser, signed in ${on(thing.added!)}`),
    server: () => presence(thing),
  }[thing.kind ?? "server"]()
  return [what, thing.behind && "Update ready"].filter(Boolean).join(" · ")
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
const update = (thing: Thing) => (thing.kind ? client({ update: true }).then(({ message }) => toast(message)) : call("update", { server: thing.id }, { for: thing.for }))

/// Takes a device out once its person has said so. Taking this one out is signing out, and the client starts again
/// where a device that isn't in does. A device that's taken out forgets what it kept of theirs when it's next on.
async function remove(device: Thing) {
  const question = device.here
    ? { heading: "Sign out of this device?", text: "It stops acting as you, and has to be approved again.", action: "Sign Out" }
    : { heading: `Remove ${device.name}?`, text: "It stops acting as you at once, and erases what it kept of yours when it's next on.", action: "Remove" }
  if (!(await confirm({ ...question, destructive: true }))) return
  if (await doing(() => call("sign-in_remove_device", { device: device.id })) && device.here) client({ leave: true })
}

/// Takes a server off the list of whoever it acts for. It no longer runs their apps.
async function removeServer(server: Thing) {
  if (!(await confirm({ heading: `Remove ${server.name ?? "this server"}?`, text: "It stops running your apps. Another server takes its work.", action: "Remove", destructive: true }))) return
  doing(() => call("remove_server", { key: server.id }, { for: server.for }))
}

const [open, setOpen] = createSignal<string>()
const [working, setWorking] = createSignal<string>()

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
  const serve = (now: boolean) => doing(() => client({ serve: now }).then(({ message }) => toast(message)))
  return (
    <Screen heading={thing().name ?? "Server"} back>
      <List>
        <Show when={thing().behind}>
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
          <Row heading="Server" text="Runs your apps from this device whenever it's on." end={<Switch aria-label="Server" checked={!!props.all.here.server} onChange={(event) => serve(event.currentTarget.checked)} />} />
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
                      onChange={(event) => doing(() => call(event.currentTarget.checked ? "install_service" : "uninstall_service", { plugin: service().plugin, service: service().service, server: server()!.id }, { for: server()!.for }))}
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
  // What's behind and can be updated from here: every server, and this device, last.
  const behind = () => [...props.all.servers, ...props.all.devices.filter((device) => device.here)].filter((thing) => thing.behind)
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
