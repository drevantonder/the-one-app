// Backups, a part of Settings: the destinations backups go, each saying when it was last backed up and how big it is, or
// what's wrong; how often they're made and which are kept. A destination opens beside the list with its backups,
// Restore, Check and Stop Backing Up Here. Backups are the network's, so every call is for the network.
import { createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, createListCollection, Empty, Field, Input, leave, List, Problem, RadioGroup, Row, Screen, Section, Select, Sheet, Skeleton, Split, Status, toast } from "ui"
import "./app.css"

/** A backup made to a destination. */
interface Backup {
  name: string
  at: number
  bytes: number
  sent_bytes: number
  took: number
}

/** Where backups go, as `backups_destinations` answers it. */
interface Destination {
  name: string
  kind: "folder" | "s3"
  where: string
  machine?: string
  item?: string
  size: number
  last?: { at: number }
  state: "never" | "backing_up" | "backed_up" | "failed" | "changed" | "interrupted"
  running?: { at: number; machine?: string }
  problem?: { kind: string; text: string }
  backups: Backup[]
}

/** What `backups_destinations` answers: the destinations, how often backups are made, which are kept, and the machines that can hold them. */
interface All {
  destinations: Destination[]
  every: string
  retention: string
  machines: { key: string; name?: string }[]
}

const network = { subject: "network" }
const backups = createMemo(() => call("backups_destinations", {}, network).then((text): All => JSON.parse(text)))
changed(() => refresh(backups))

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 hours ago".
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -1) return "just now"
  if (minutes > -60) return ago.format(minutes, "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
const when = (at: number) => new Date(at).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" })
const units = ["bytes", "KB", "MB", "GB", "TB"]
function size(bytes: number) {
  let n = bytes, unit = 0
  while (n >= 1000 && unit < units.length - 1) (n /= 1000), unit++
  return `${n >= 100 || unit === 0 ? Math.round(n) : n.toFixed(1)} ${units[unit]}`
}
/// How long something took: "40 ms", "14 s", "3 min".
const took = (ms: number) => (ms < 1000 ? `${ms} ms` : ms < 60_000 ? `${Math.round(ms / 1000)} s` : `${Math.round(ms / 60_000)} min`)
/// A name as relation tuples say it, short: "users/andre" is "andre".
const short = (who?: string) => String(who ?? "").replace(/^(users|groups)\//, "")

const time = new Intl.DateTimeFormat(undefined, { timeStyle: "short" })
/// A destination's grey line, by its state: what's under way or wrong, or when it was last backed up and how big it is.
const ABOUT: Record<Destination["state"], (destination: Destination) => string> = {
  never: () => "Not backed up yet",
  backing_up: () => "Backing up…",
  backed_up: (destination) => `Backed up ${since(destination.last!.at)} · ${size(destination.size)}`,
  failed: () => "Couldn't back up",
  changed: () => "Changed since it was sent",
  interrupted: () => "Last backup cut short",
}
const about = (destination: Destination) => ABOUT[destination.state](destination)
/// Whether its grey line is a warning.
const warns = (destination: Destination) => ["failed", "changed", "interrupted"].includes(destination.state)

/// Does something, says why if it couldn't, and shows everything as it is after. → whether it was done.
async function doing<T>(does: () => Promise<T>): Promise<T | true | false> {
  try {
    const result = await does()
    return result ?? true
  } catch (error) {
    toast(error as Error)
    return false
  } finally {
    refresh(backups)
  }
}

const [open, setOpen] = createSignal<string>()
const [adding, setAdding] = createSignal(false)
const [restoring, setRestoring] = createSignal<Destination>()
const [working, setWorking] = createSignal<string>()

/// `does` under way on the button `which`, which shows it's working until it's done.
const press = (which: string, does: () => Promise<unknown>) => async () => {
  setWorking(which)
  await doing(does)
  setWorking()
}

const schedules = createListCollection({
  items: [
    { value: "hour", label: "Every Hour" },
    { value: "day", label: "Every Day" },
    { value: "week", label: "Every Week" },
    { value: "never", label: "Manually" },
  ],
})

/// A destination, opened: what's wrong with it, where it is, its backups, Check and Stop Backing Up Here.
function Opened(props: { destination: Destination }) {
  const destination = () => props.destination
  const stop = async () => {
    const question = { heading: `Stop backing up to ${destination().name}?`, text: "What's there now stays there.", action: "Stop Backing Up", destructive: true }
    if (!(await confirm(question))) return
    if (await doing(() => call("backups_remove", { destination: destination().name }, network))) setOpen()
  }
  const check = press("check", async () => {
    const [checked] = JSON.parse(await call("backups_check", { destination: destination().name }, network)).checks as { backup: string | null; objects?: number }[]
    toast(checked.backup ? `Read back ${checked.objects} objects. Every one is as it was sent.` : "There's no backup here yet.")
  })
  const now = () => <Button variant="secondary" loading={working() === "now"} onClick={press("now", () => call("backups_now", { destination: destination().name }, network))}>Back Up Now</Button>
  const on = () => (destination().running?.machine ? ` on ${destination().running!.machine}` : "")
  return (
    <Screen heading={destination().name} back>
      <Switch>
        <Match when={destination().state === "backing_up"}>
          <Status heading="Backing up…" text={`Since ${time.format(destination().running!.at)}${on()}.`} />
        </Match>
        <Match when={destination().state === "interrupted"}>
          <Problem warn heading="The last backup was cut short" text={`It stopped${on()} before it finished, at ${time.format(destination().running!.at)}. Backing up again sends only what it didn't.`} action={now()} />
        </Match>
        <Match when={destination().state === "changed"}>
          <Problem heading={`What's at ${destination().name} was changed`} text="Part of the newest backup isn't there as it was sent. Back up now to send it again." action={now()} />
        </Match>
        <Match when={destination().state === "failed"}>
          <Problem heading={`Couldn't back up to ${destination().name}`} text={destination().problem!.text} action={now()} />
        </Match>
      </Switch>
      <List>
        <Row heading={destination().kind === "folder" ? "Folder" : "Bucket"} text={destination().where} />
        <Show when={destination().machine}>
          <Row heading="On" text={destination().machine} />
        </Show>
        <Show when={destination().item}>
          <Row heading="Keys" text={`${destination().item}, in Everyone's Vault`} />
        </Show>
        <Row heading="Size" text={size(destination().size)} />
      </List>
      <Show when={destination().backups.length}>
        <Section heading="Backups">
          <List>
            <For each={destination().backups} keyed={(backup) => backup.name}>
              {(backup) => <Row heading={when(backup().at)} text={`${size(backup().bytes)} · ${size(backup().sent_bytes)} sent · ${took(backup().took)}`} />}
            </For>
          </List>
        </Section>
      </Show>
      <div class="actions">
        <Show when={destination().backups.length}>
          <Button variant="secondary" onClick={() => setRestoring(destination())}>Restore</Button>
          <Button variant="outline" loading={working() === "check"} onClick={check}>Check</Button>
        </Show>
        <Button variant="destructive" onClick={stop}>Stop Backing Up Here</Button>
      </div>
    </Screen>
  )
}

/// Adds a destination: a folder on one of the network's machines, as on a disk plugged into it, or S3-compatible storage
/// whose keys are an item in Everyone's Vault. It's tried before it's added, so a typo shows here.
function AddSheet(props: { machines: All["machines"] }) {
  const [kind, setKind] = createSignal("folder")
  const machines = () => createListCollection({ items: props.machines.map((machine) => ({ value: machine.key, label: machine.name ?? machine.key.slice(0, 8) })) })
  const add = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    const typed = Object.fromEntries([...new FormData(event.target as HTMLFormElement)].filter(([name, value]) => value !== "" && name !== "kind"))
    setWorking("add")
    if (await doing(() => call("backups_add", { ...typed }, network))) setAdding(false)
    setWorking()
  }
  return (
    <Sheet heading="Add Destination" open={adding()} onClose={() => setAdding(false)}>
      <form onSubmit={add}>
        <RadioGroup name="kind" value={kind()} onChange={setKind} options={[{ value: "folder", label: "A Disk" }, { value: "s3", label: "S3-Compatible Storage" }]} />
        <Show when={kind() === "folder"}>
          <Show
            when={props.machines.length}
            fallback={<Problem warn heading="No machine can hold backups yet" text="Switch on Backups.Disk for the machine the disk is plugged into, in Devices." />}
          >
            <Field label="Machine">
              <Select.Root name="machine" collection={machines()} />
            </Field>
            <Field label="Folder" hint="Its whole path on that machine.">
              <Input name="folder" placeholder="/mnt/disk/backups" autocapitalize="none" spellcheck={false} required />
            </Field>
          </Show>
        </Show>
        <Show when={kind() === "s3"}>
          <Field label="Endpoint">
            <Input name="endpoint" type="url" placeholder="https://<account>.r2.cloudflarestorage.com" autocapitalize="none" spellcheck={false} required />
          </Field>
          <Field label="Bucket">
            <Input name="bucket" placeholder="backups" autocapitalize="none" spellcheck={false} required />
          </Field>
          <Field label="Keys" hint="An item in Everyone's Vault with access_key_id and secret_access_key, switched on for Backups.">
            <Input name="item" placeholder="Backups Storage" required />
          </Field>
        </Show>
        <Field label="Name" hint="What to call it. The folder's or the bucket's name if left empty.">
          <Input name="name" />
        </Field>
        <Button type="submit" loading={working() === "add"} disabled={kind() === "folder" && !props.machines.length}>Add</Button>
      </form>
    </Sheet>
  )
}

/// Brings back what a backup holds that's gone: someone's data, what one app keeps for them, or one file. What's
/// there now stays as it is.
function RestoreSheet() {
  const destination = () => restoring()
  const [what, setWhat] = createSignal("whose")
  const choices = createMemo(async () => {
    if (!destination()) return { whose: createListCollection<{ value: string; label: string }>({ items: [] }), apps: createListCollection<{ value: string; label: string }>({ items: [] }) }
    const listed = <T,>(tool: string, key: string) => call(tool).then((text): T[] => JSON.parse(text)[key], (): T[] => [])
    const [people, groups, store] = await Promise.all([
      listed<{ who: string; called?: string; joined?: boolean }>("sign-in_everyone", "people"),
      listed<{ group: string; called: string }>("permissions_groups", "groups"),
      listed<{ plugin: string; name: string }>("home-screen_store", "plugins"),
    ])
    const whose = [
      ...people.filter((one) => one.joined).map((one) => ({ value: one.who, label: one.called ?? short(one.who) })),
      ...groups.map((group) => ({ value: group.group, label: group.called })),
      { value: "network", label: "Everyone" },
    ]
    const apps = [...new Map(store.map((app) => [app.plugin.split("/").pop()!, app.name])).entries()].map(([value, label]) => ({ value, label }))
    return { whose: createListCollection({ items: whose }), apps: createListCollection({ items: apps }) }
  })
  const backupsOf = () => createListCollection({ items: destination()!.backups.map((backup) => ({ value: backup.name, label: when(backup.at) })) })
  const restore = async (event: SubmitEvent) => {
    event.preventDefault()
    const typed = Object.fromEntries(new FormData(event.target as HTMLFormElement)) as Record<string, string>
    const blobs = what() === "file" ? [typed.file.trim()] : what() === "app" ? [`${typed.whose}/own/${typed.app}/`, `${typed.whose}/made/${typed.app}/`] : [`${typed.whose}/`]
    setWorking("restore")
    const result = await doing(async () => {
      let restored = 0
      for (const one of blobs) restored += JSON.parse(await call("backups_restore", { destination: destination()!.name, backup: typed.backup, blobs: one }, network)).restored.length
      return restored ? `Brought back ${restored === 1 ? "1 file" : `${restored} files`}.` : "Nothing was gone, so nothing changed."
    })
    setWorking()
    if (result) {
      toast(String(result))
      setRestoring()
    }
  }
  return (
    <Sheet heading="Restore" open={!!destination()} onClose={() => setRestoring()}>
      <Show when={destination()}>
        <form onSubmit={restore}>
          <Field label="From">
            <Select.Root name="backup" collection={backupsOf()} defaultValue={[destination()!.backups[0]?.name]} />
          </Field>
          <Field label="What">
            <RadioGroup value={what()} onChange={setWhat} options={[{ value: "whose", label: "Someone's Data" }, { value: "app", label: "One App's Data" }, { value: "file", label: "One File" }]} />
          </Field>
          <Loading fallback={<Skeleton rows={1} />}>
            <Show when={what() !== "file"}>
              <Field label="Owner">
                <Select.Root name="whose" collection={choices().whose} required />
              </Field>
            </Show>
            <Show when={what() === "app"}>
              <Field label="App">
                <Select.Root name="app" collection={choices().apps} required />
              </Field>
            </Show>
          </Loading>
          <Show when={what() === "file"}>
            <Field label="File" hint="Its whole name in the network, as users/alice/made/notes/7/photo.jpg.">
              <Input name="file" autocapitalize="none" spellcheck={false} required />
            </Field>
          </Show>
          <p class="hint">Only what's gone comes back. Nothing that's there now changes.</p>
          <Button type="submit" loading={working() === "restore"}>Restore</Button>
        </form>
      </Show>
    </Sheet>
  )
}

function Backups(props: { all: All }) {
  const destinations = () => props.all.destinations
  const opened = () => destinations().find((destination) => destination.name === open())
  const now = press("now", () => call("backups_now", {}, network))
  const backingUp = () => destinations().some((destination) => destination.state === "backing_up")
  const schedule = ({ value: [every] }: { value: string[] }) => every && every !== props.all.every && doing(() => call("backups_schedule", { every }, network))
  return (
    <>
      <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={destinations().length}><Empty>Pick a destination to see its backups.</Empty></Show>}><Opened destination={opened()!} /></Show>}>
        <Screen heading="Backups" back onBack={leave} action={<Show when={destinations().length}><Button loading={working() === "now" || backingUp()} onClick={now}>Back Up Now</Button></Show>}>
          <Show when={destinations().length} fallback={<Empty action={<Button onClick={() => setAdding(true)}>Add Destination</Button>}>Nothing is backed up yet.</Empty>}>
            <List>
              <For each={destinations()} keyed={(destination) => destination.name}>
                {(destination) => <Row heading={destination().name} text={<span class={{ warn: warns(destination()) }}>{about(destination())}</span>} current={open() === destination().name} onClick={() => setOpen(destination().name)} />}
              </For>
              <Row heading="Add Destination" onClick={() => setAdding(true)} />
            </List>
            <Section heading="Schedule">
              <Field label="Back Up" hint={props.all.retention}>
                <Select.Root collection={schedules} value={[props.all.every]} onValueChange={schedule} />
              </Field>
            </Section>
          </Show>
        </Screen>
      </Split>
      <AddSheet machines={props.all.machines} />
      <RestoreSheet />
    </>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Backups" back onBack={leave}><Problem heading="Can't show backups" text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading="Backups" back onBack={leave}><Skeleton rows={3} /></Screen>}>
        <Backups all={backups()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
