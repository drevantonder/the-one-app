// Backups: the destinations backups go, each saying when it was last backed up and how big it is, or
// what's wrong; how often they're made and which are kept. A destination opens beside the list with its backups,
// Restore, Check and Stop Backing Up Here. Backups are the network's, so every call is for the network.
import { createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch } from "solid-js"
import { Button, call, changed, confirm, Empty, Field, Icon, Input, List, Problem, RadioGroup, Row, Screen, Section, Select, Sheet, since, size, Skeleton, Spinner, Split, Status, styles, Text, toast, tokens, View } from "ui"

const look = styles({
  note: { color: tokens.colour.secondaryLabel },
  warns: { color: tokens.colour.red },
})

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

const when = (at: number) => new Date(at).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" })
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
/// What's being restored from: a destination, and the backup if one was picked.
const [restoring, setRestoring] = createSignal<{ destination: Destination; backup?: string }>()
const [working, setWorking] = createSignal<string>()

/// `does` under way on the button `which`, which shows it's working until it's done.
const press = (which: string, does: () => Promise<unknown>) => async () => {
  setWorking(which)
  await doing(does)
  setWorking()
}

const schedules = [
    { value: "hour", label: "Every Hour" },
    { value: "day", label: "Every Day" },
    { value: "week", label: "Every Week" },
    { value: "never", label: "Manually" },
]

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
  const now = () => <Button variant="secondary" loading={working() === "now"} onPress={press("now", () => call("backups_now", { destination: destination().name }, network))}>Back Up Now</Button>
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
          <Row heading="On" end={destination().machine} />
        </Show>
        <Show when={destination().item}>
          <Row heading="Keys" end={`${destination().item}, in Accounts`} />
        </Show>
        <Row heading="Size" end={size(destination().size)} />
      </List>
      <Show when={destination().backups.length}>
        <Section heading="Backups">
          <List>
            <For each={destination().backups} keyed={(backup) => backup.name}>
              {(backup) => <Row heading={when(backup().at)} text={`${size(backup().bytes)} · ${size(backup().sent_bytes)} sent · ${took(backup().took)}`} onPress={() => setRestoring({ destination: destination(), backup: backup().name })} />}
            </For>
          </List>
        </Section>
      </Show>
      <Show when={destination().backups.length}>
        <List>
          <Row heading="Restore…" onPress={() => setRestoring({ destination: destination() })} />
          <Row heading="Check" end={working() === "check" ? <Spinner /> : undefined} onPress={working() === "check" ? undefined : check} />
        </List>
      </Show>
      <List>
        <Row heading="Stop Backing Up Here…" destructive onPress={stop} />
      </List>
    </Screen>
  )
}

/// Adds a destination: a folder on one of the network's machines, as on a disk plugged into it, or S3-compatible storage
/// whose keys are an item in Accounts for Everyone. It's tried before it's added, so a typo shows here.
function AddSheet(props: { machines: All["machines"] }) {
  const [kind, setKind] = createSignal("folder")
  const [machine, setMachine] = createSignal<string>()
  const [typed, setTyped] = createSignal<Record<string, string>>({})
  const [missing, setMissing] = createSignal<string[]>([])
  const machines = () => props.machines.map((machine) => ({ value: machine.key, label: machine.name ?? machine.key.slice(0, 8) }))
  const type = (name: string) => (text: string) => (setTyped({ ...typed(), [name]: text }), setMissing(missing().filter((one) => one !== name)))
  const required = () => (kind() === "folder" ? (props.machines.length ? ["folder"] : []) : ["endpoint", "bucket", "item"])
  const add = async () => {
    const empty = required().filter((name) => !typed()[name]?.trim())
    setMissing(empty)
    if (empty.length) return
    const sent = Object.fromEntries(
      [...(kind() === "folder" ? [["machine", machine() ?? props.machines[0]?.key]] : []), ...required().map((name) => [name, typed()[name]]), ["name", typed().name]].filter(([, value]) => value),
    )
    setWorking("add")
    if (await doing(() => call("backups_add", sent, network))) (setAdding(false), setTyped({}))
    setWorking()
  }
  const error = (name: string) => (missing().includes(name) ? "Fill in this field." : undefined)
  return (
    <Sheet heading="Add Destination" open={adding()} onClose={() => setAdding(false)}>
      <RadioGroup value={kind()} onValueChange={setKind} options={[{ value: "folder", label: "A Disk" }, { value: "s3", label: "S3-Compatible Storage" }]} />
      <Show when={kind() === "folder"}>
        <Show
          when={props.machines.length}
          fallback={<Problem warn heading="No machine can hold backups yet" text="Switch on Backups to a disk for the machine the disk is plugged into, in Devices." />}
        >
          <Field label="Machine">
            <Select.Root items={machines()} value={machine() ?? props.machines[0].key} onValueChange={setMachine} />
          </Field>
          <Field label="Folder" hint="Its whole path on that machine." error={error("folder")}>
            <Input placeholder="/mnt/disk/backups" autoCapitalize="none" autoCorrect={false} spellCheck={false} value={typed().folder} onChangeText={type("folder")} onSubmitEditing={add} required />
          </Field>
        </Show>
      </Show>
      <Show when={kind() === "s3"}>
        <Field label="Endpoint" error={error("endpoint")}>
          <Input inputMode="url" placeholder="https://<account>.r2.cloudflarestorage.com" value={typed().endpoint} onChangeText={type("endpoint")} onSubmitEditing={add} required />
        </Field>
        <Field label="Bucket" error={error("bucket")}>
          <Input placeholder="backups" autoCapitalize="none" autoCorrect={false} spellCheck={false} value={typed().bucket} onChangeText={type("bucket")} onSubmitEditing={add} required />
        </Field>
        <Field label="Keys" hint="An item in Accounts for Everyone, with an access key ID and a secret access key, switched on for Backups." error={error("item")}>
          <Input placeholder="Backups Storage" value={typed().item} onChangeText={type("item")} onSubmitEditing={add} required />
        </Field>
      </Show>
      <Field label="Name" hint="What to call it. The folder's or the bucket's name if left empty.">
        <Input value={typed().name} onChangeText={type("name")} onSubmitEditing={add} />
      </Field>
      <Button loading={working() === "add"} disabled={kind() === "folder" && !props.machines.length} onPress={add}>Add</Button>
    </Sheet>
  )
}

/// Brings back what a backup holds that's gone: someone's data, what one app keeps for them, or one file. What's
/// there now stays as it is.
function RestoreSheet() {
  return (
    <Sheet heading="Restore" open={!!restoring()} onClose={() => setRestoring()}>
      <Show when={restoring()}>{(from) => <Restore destination={from().destination} from={from().backup} />}</Show>
    </Sheet>
  )
}

function Restore(props: { destination: Destination; from?: string }) {
  const [what, setWhat] = createSignal("whose")
  const [backup, setBackup] = createSignal<string | undefined>(props.from)
  const [whose, setWhose] = createSignal<string>()
  const [app, setApp] = createSignal<string>()
  const [file, setFile] = createSignal("")
  const [missing, setMissing] = createSignal(false)
  const choices = createMemo(async () => {
    const listed = <T,>(tool: string, key: string) => call(tool).then((text): T[] => JSON.parse(text)[key], (): T[] => [])
    const [people, groups, marketplace] = await Promise.all([
      listed<{ who: string; called?: string; joined?: boolean }>("sign-in_everyone", "people"),
      listed<{ group: string; called: string }>("permissions_groups", "groups"),
      listed<{ app: string | null; name: string; title: string }>("app-store_marketplace", "apps"),
    ])
    const owners = [
      ...people.filter((one) => one.joined).map((one) => ({ value: one.who, label: one.called ?? short(one.who) })),
      ...groups.map((group) => ({ value: group.group, label: group.called })),
      { value: "network", label: "Everyone" },
    ]
    const apps = [...new Map(marketplace.filter((app) => app.app).map((app) => [app.name, app.title])).entries()].map(([value, label]) => ({ value, label }))
    return { owners, apps }
  })
  const backupsOf = () => props.destination.backups.map((backup) => ({ value: backup.name, label: when(backup.at) }))
  const restore = async () => {
    // What a select has chosen until someone picks another is its first.
    const owner = whose() ?? choices().owners[0]?.value
    const which = app() ?? choices().apps[0]?.value
    if (what() === "file" ? !file().trim() : false) return setMissing(true)
    const blobs = what() === "file" ? [file().trim()] : what() === "app" ? [`${owner}/own/${which}/`, `${owner}/made/${which}/`] : [`${owner}/`]
    setWorking("restore")
    const result = await doing(async () => {
      let restored = 0
      for (const one of blobs) restored += JSON.parse(await call("backups_restore", { destination: props.destination.name, backup: backup() ?? props.destination.backups[0]?.name, blobs: one }, network)).restored.length
      return restored ? `Brought back ${restored === 1 ? "1 file" : `${restored} files`}.` : "Nothing was gone, so nothing changed."
    })
    setWorking()
    if (result) {
      toast(String(result))
      setRestoring()
    }
  }
  return (
    <>
      <Field label="From">
        <Select.Root items={backupsOf()} value={backup() ?? props.destination.backups[0]?.name} onValueChange={setBackup} />
      </Field>
      <Field label="What">
        <RadioGroup value={what()} onValueChange={setWhat} options={[{ value: "whose", label: "Someone's Data" }, { value: "app", label: "One App's Data" }, { value: "file", label: "One File" }]} />
      </Field>
      <Loading fallback={<Skeleton rows={1} />}>
        <Show when={what() !== "file"}>
          <Field label="Owner">
            <Select.Root items={choices().owners} value={whose() ?? choices().owners[0]?.value} onValueChange={setWhose} />
          </Field>
        </Show>
        <Show when={what() === "app"}>
          <Field label="App">
            <Select.Root items={choices().apps} value={app() ?? choices().apps[0]?.value} onValueChange={setApp} />
          </Field>
        </Show>
      </Loading>
      <Show when={what() === "file"}>
        <Field label="File" hint="Its whole name in the network, as users/alice/made/notes/7/photo.jpg." error={missing() && !file().trim() ? "Fill in this field." : undefined}>
          <Input autoCapitalize="none" autoCorrect={false} spellCheck={false} value={file()} onChangeText={setFile} onSubmitEditing={restore} required />
        </Field>
      </Show>
      <Text style={look.note}>Only what's gone comes back. Nothing that's there now changes.</Text>
      <Button loading={working() === "restore"} onPress={restore}>Restore</Button>
    </>
  )
}

function Backups(props: { all: All }) {
  const destinations = () => props.all.destinations
  const opened = () => destinations().find((destination) => destination.name === open())
  const now = press("now", () => call("backups_now", {}, network))
  const backingUp = () => destinations().some((destination) => destination.state === "backing_up")
  const schedule = (every: string) => every !== props.all.every && doing(() => call("backups_schedule", { every }, network))
  return (
    <>
      <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={destinations().length}><Empty>Pick a destination to see its backups.</Empty></Show>}><Opened destination={opened()!} /></Show>}>
        <Screen heading="Backups" back action={<Show when={destinations().length}><Button loading={working() === "now" || backingUp()} onPress={now}>Back Up Now</Button></Show>}>
          <Show when={destinations().length} fallback={<Empty action={<Button onPress={() => setAdding(true)}>Add Destination</Button>}>Nothing is backed up yet.</Empty>}>
            <List>
              <For each={destinations()} keyed={(destination) => destination.name}>
                {(destination) => <Row heading={destination().name} text={<Text style={[warns(destination()) && look.warns]}>{about(destination())}</Text>} current={open() === destination().name} onPress={() => setOpen(destination().name)} />}
              </For>
            </List>
            <List>
              <Row start={<Icon name="plus" />} heading="Add Destination" onPress={() => setAdding(true)} />
            </List>
            <Section heading="Schedule">
              <List>
                <Row heading="Back Up" end={<Select.Root aria-label="Back Up" items={schedules} value={props.all.every} onValueChange={schedule} />} />
              </List>
              <Text style={look.note}>{props.all.retention}</Text>
            </Section>
          </Show>
        </Screen>
      </Split>
      <AddSheet machines={props.all.machines} />
      <RestoreSheet />
    </>
  )
}

/// Backups, as Settings draws it.
export default function BackupsSection() {
  return (
  <Errored fallback={(error) => <Screen heading="Backups" back><Problem heading="Can't show backups" text={error() as Error} /></Screen>}>
    <Loading fallback={<Screen heading="Backups" back><Skeleton rows={3} /></Screen>}>
      <Backups all={backups()} />
    </Loading>
  </Errored>
)
}
