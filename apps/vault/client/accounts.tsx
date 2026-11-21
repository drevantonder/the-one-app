// Accounts, the Vault as Settings draws it: what's waiting for you as banners, your accounts, your passwords and keys,
// each other kind of item an app keeps here (`vault.types`) under its app, and Security, each item opening over the
// page. An item shows its details (a secret one as dots, never shown), its websites, and who has access:
// your agents and each app, switched here. One of a type with a component is drawn by its own app. Deleting one is
// soft delete's. Security is everything about the key that keeps it. What's missing before the Vault works shows as
// one box atop the list, with the one thing that fixes it. A request is an alert, as a system's permission prompt is.
// It's written as the UI library's skill says, so it runs on a native host as it does on the DOM.
import { createEffect, createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch as Choose, untrack } from "solid-js"
import { Alert, Button, call, changed, Checkbox, component, confirm, Empty, Field, Frame, Image, Input, link, List, open as opening, Problem, Row, Screen, Section, Select, Sheet, showRecoveryKey, Skeleton, Spinner, Split, styles, Switch, Text, toast, tokens, useRecoveryKey, View } from "ui"

const look = styles({
  fill: { flexGrow: 1, alignSelf: "stretch" },
  heading: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2.5], minWidth: 0 },
  status: { flexDirection: "row", alignItems: "center", gap: tokens.grid[3] },
  grows: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  shrinks: { flexShrink: 1, minWidth: 0 },
  wrong: { color: tokens.colour.red },
  note: { color: tokens.colour.secondaryLabel },
  line: { flexDirection: "row", alignItems: "center", gap: tokens.grid[1.5] },
  /// An app's icon, or an item's picture, at the start of its row: its flat tile, as on the Home Screen.
  tile: { width: tokens.grid[8], height: tokens.grid[8], borderRadius: tokens.radius.medium },
  small: { width: tokens.grid[4], height: tokens.grid[4], borderRadius: tokens.radius.small },
  inline: { flexDirection: "row", alignItems: "center", gap: tokens.grid[1], minWidth: 0 },
  /// A dot that says something needs its person.
  dot: { width: tokens.grid[2], height: tokens.grid[2], flexShrink: 0, borderRadius: tokens.radius.full, backgroundColor: tokens.colour.orange },
})

/** A field of an item: a secret one has no value here. */
interface ItemField {
  name: string
  secret: boolean
  value?: string
}

/** An item, as `vault_list` answers it. */
interface Item {
  name: string
  artifact: string
  /** What it is, as it was made: a password, an account (a sign-in, or what an app keeps for itself), or an app's. */
  kind: "password" | "account" | "app_item"
  type: string | null
  status: Status | null
  fields: ItemField[]
  hosts: string[]
  /** Who may use it: "agents", and the apps switched on for it, by name. */
  access: string[]
  sign_in?: boolean
  /** If a save from a browser made it: the device of the agent that saved it, the site, and when. */
  added?: { by: string | null; from: string; at: number } | null
}

/** What an app says its item needs of its person, or what adds one of a type: a tool of the app's, called as its
 * person, or a page of its client's. */
interface Action {
  label?: string
  tool?: string
  inputs?: Record<string, unknown>
  page?: string
  name?: boolean
}

/** How an item is, other than ready, which is none, as the app that keeps it says. `by` is that app. */
interface Status {
  state: "busy" | "needs_action"
  message: string
  action?: Action
  by: string
}

/** A kind of item an app keeps here, as `vault_list` answers it. */
interface ItemType {
  id: string
  app: string
  title: string | null
  icon: string | null
  trusted: boolean
  name: string
  prefix: string
  section: string
  component: string | null
  create: Action | null
}

/** What an app or an agent asked for, waiting for an answer. */
interface Request {
  id: string
  title: string
  state: "waiting" | "allowed" | "declined" | "expired"
  app: { name: string; title: string } | null
  by: string | null
  why: string | null
  hosts: string[]
  fields: { name: string; secret?: boolean }[]
}

/** An app that uses the Vault: its name, which its tools start with, and its title, which a person reads. */
interface App {
  name: string
  title: string
  icon?: string | null
}

/** A device, as `vault_security` names it. */
interface Device {
  device: string
  name: string
}

/** How ready the Vault is (the server's `@readiness`): the first thing it lacks, in the order they're done. */
type Readiness = "not_set_up" | "no_server" | "device_lacks_key" | "ready"

/** Everything about the key that keeps the Vault, as `vault_security` says it in one answer. */
interface Security {
  readiness: Readiness
  owner: string
  holds: boolean
  device: string | null
  devices: Device[]
  asking: Device[]
  servers: string[]
  may: { key: string; name?: string }[]
}

/** The sheet that's open over the page: a field to type, a new item of a type to name, or the kinds a section adds. */
type Sheet = { kind: "field"; item?: Item; field?: ItemField } | { kind: "create"; type: ItemType } | { kind: "add"; section: string }

/** Where you are, as the sidebar picks it: a kind of item, the requests waiting, or Security. */
type Place = "all" | "passwords" | "accounts" | `section:${string}` | "requests" | "security"

/// Whose Vault this is: a person's, a group's or the network's. Left unchosen, it's whoever the app was opened for.
const [whose, setWhose] = createSignal<string>()
const ask = <T,>(tool: string, inputs?: object): Promise<T> => call(tool, inputs, { subject: whose() }).then(JSON.parse)

const security = createMemo(() => ask<Security>("vault_security"))
const nothing = { items: [] as Item[], requests: [] as Request[], apps: [] as App[], types: [] as ItemType[] }
const contents = createMemo(() => (security().servers.length ? ask<typeof nothing>("vault_list") : nothing))
const again = () => [security, contents].forEach(refresh)
changed(again)

/// Whose Vaults its person can open here: theirs, each group they own, and the network if they own it.
const owners = createMemo(async () => {
  const me: string | undefined = (await call("sign-in_people").then((text) => JSON.parse(text).people, () => []))[0]?.who
  if (!me) return []
  const [groups, network] = await Promise.all([
    call("permissions_groups", {}, { subject: me }).then((text): { group: string; called: string; owners: string[] }[] => JSON.parse(text).groups, () => []),
    call("permissions_check", { artifact: "network", relation: "owner" }, { subject: me }).then((result): boolean => JSON.parse(result).may, () => false),
  ])
  const theirs = groups.filter((group) => group.owners.includes(me)).map((group) => ({ value: group.group, label: group.called, whose: /s$/i.test(group.called) ? `${group.called}'` : `${group.called}'s` }))
  return [{ value: me, label: "Yours", whose: "your" }, ...theirs, ...(network ? [{ value: "network", label: "The network's", whose: "the network's" }] : [])]
})
/// Whose the Vault is, in a sentence: "your", "The Smiths'", "the network's".
const whom = () => owners().find((owner) => owner.value === security().owner)?.whose ?? "your"

/// Does something that changes the Vault, and says what the server said if it wouldn't. → whether it did.
async function does(tool: string, inputs: object) {
  try {
    await call(tool, inputs, { subject: whose() })
    // Read again once what called this has done with it (closed its sheet): a refresh holds what changes while it
    // reads, so a sheet closed after it would wait for it.
    setTimeout(again)
    return true
  } catch (error) {
    toast(error as Error)
    return false
  }
}

/// A field's name as a person reads it: "api_key" is "API key".
const label = (name: string) =>
  String(name)
    .split(/[_-]+/)
    .map((word, at) => (/^(api|url|id|ssh|aws|ip)$/i.test(word) ? word.toUpperCase() : at ? word.toLowerCase() : word[0].toUpperCase() + word.slice(1).toLowerCase()))
    .join(" ")
/// A few things in a sentence: "api.github.com and github.com".
const listed = (things: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(things)
/// The websites typed, which are any case: a phone may have started one with a capital.
const sites = (typed: string) => typed.toLowerCase().split(/[\s,]+/).filter(Boolean)
/// Where an item a save from a browser made came from: "On Andre's Mac, from console.cloud.google.com".
const whence = (added: NonNullable<Item["added"]>) => `${added.by ? `On ${added.by}, from` : "From"} ${added.from}`
/// An item's type, if it's of one, and its name as a person reads it: "Personal", not "Chrome profile: Personal".
const typeOf = (item: Item) => contents().types.find((type) => type.id === item.type)
const shown = (item: Item) => item.name.slice(typeOf(item)?.prefix.length ?? 0)
/// The place in the sidebar an item is in: a section an app's type names, Accounts for a sign-in, or Passwords & Keys.
function placeOf(item: Item): Place {
  const type = typeOf(item)
  if (item.kind === "app_item" && type) return type.section === "Accounts" ? "accounts" : `section:${type.section}`
  return item.kind === "password" ? "passwords" : "accounts"
}
/// The requests still waiting for their person.
const waiting = () => contents().requests.filter((request) => request.state === "waiting")
/// The sections apps' types are listed under, other than Accounts, which has its own place: "Chrome Profiles".
const sections = () => [...new Set(contents().types.map((type) => type.section))].filter((section) => section !== "Accounts").sort()
/// The types an app makes one of from here, in a place.
const creatable = (place: Place) => contents().types.filter((type) => type.create && type.trusted && (place === "accounts" ? type.section === "Accounts" : place === `section:${type.section}`))
/// Does an action an app gave, as its person: a tool of the app's, whose `url`, if it gives one, opens in the
/// device's browser, or a page of its client's. → whether it did.
async function act(action: Action, app: string, inputs: Record<string, unknown> = {}) {
  try {
    if (action.page) await opening(`${app}/${action.page}`, { subject: whose() })
    else {
      const done = await ask<{ url?: string }>(`${app}_${action.tool}`, { ...action.inputs, ...inputs })
      if (done.url) await opening(done.url)
    }
    again()
    return true
  } catch (error) {
    toast(error as Error)
    return false
  }
}
/// A type's name as a menu or a heading says it: "Chrome Profile".
const titled = (name: string) => name.replace(/(^|\s)\S/g, (first) => first.toUpperCase())

/// Whether Security is open over the page.
const [secure, setSecure] = createSignal(false)
/// The item open beside the list, by its name; the sheet open over the page; and the request asked about.
const [open, setOpen] = createSignal<string>()
const [sheet, setSheet] = createSignal<Sheet>()
const [asking, setAsking] = createSignal<Request>()
const shut = () => setSheet()

/// Opens Security over the page.
function enter() {
  setOpen()
  setSecure(true)
}

/// An item's picture at the start of its row: its app's icon, or its first letter on a colour its name picks, as
/// Passwords' tiles are.
function Tile(props: { item: Item }) {
  const letter = () => {
    const name = shown(props.item)
    // The golden angle apart for each letter, so names that start alike still differ.
    const hue = Math.round([...name].reduce((sum, letter, at) => sum + letter.codePointAt(0)! * (at + 1) * 137.508, 0) % 360)
    const first = (name.trim()[0]?.toUpperCase() ?? "?").replace(/[<>&"]/, (character) => `&#${character.codePointAt(0)};`)
    const drawn = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" style="fill:oklch(0.55 0.13 ${hue})"/><text x="16" y="16" dy="0.35em" text-anchor="middle" font-family="system-ui, sans-serif" font-size="16" font-weight="600" fill="#fff">${first}</text></svg>`
    return `data:image/svg+xml,${encodeURIComponent(drawn)}`
  }
  return <Image src={typeOf(props.item)?.icon ?? letter()} alt="" style={look.tile} />
}

/// Whether this Vault is kept with its person's own key, which their passkeys unlock: a group's or the network's isn't.
const own = () => security().owner.startsWith("users/")

/// Unlock with Passkey, every step of a Vault's readiness: the network's sign-in page, in the device's browser, makes
/// its person's key if there's none, lets this device hold it, and hands it to the servers waiting for it, once the
/// passkey's used there (`sign-in`'s `unlock_with_passkey`). A group's or the network's Vault is handed to its
/// servers by the key of whoever owns it. Coming back, the Vault shows what changed.
async function unlockWithPasskey() {
  try {
    if (own()) await call("encryption_ask", {})
    const link = await call("sign-in_unlock_with_passkey", own() ? { device: security().device } : { scope: whose() })
    await opening(JSON.parse(link).link)
  } catch (error) {
    toast(error as Error)
  }
}

/// What's missing before the Vault works, atop the list: one box, saying what and the one thing that fixes it.
function Readiness() {
  return (
    <Choose>
      <Match when={security().readiness === "not_set_up"}>
        <Problem warn heading={`Set up ${whom()} Vault`} text="Passwords, keys and sign-ins are kept here, encrypted with a key your passkey unlocks." action={<Button onPress={unlockWithPasskey}>Set Up with Passkey</Button>} />
      </Match>
      <Match when={security().readiness === "no_server"}>
        <Problem
          warn
          heading={`No server can open ${whom()} Vault`}
          text="Until one can, it keeps nothing. Your passkey hands its key to the servers that run it."
          action={<Button onPress={unlockWithPasskey}>Use Passkey</Button>}
        />
      </Match>
      <Match when={security().readiness === "device_lacks_key"}>
        <Problem
          warn
          heading={`This device doesn't hold ${whom()} key`}
          text="It adds and uses items. Your passkey lets it hold the key, to show the recovery key."
          action={
            // Security lists the same two, under Trust This Device.
            <Show when={!secure()}>
              <Show when={own()}>
                <Button onPress={unlockWithPasskey}>Unlock with Passkey</Button>
              </Show>
              <Button variant="outline" onPress={() => useRecoveryKey({ subject: whose() }).then(again)}>Use Recovery Key</Button>
            </Show>
          }
        />
      </Match>
    </Choose>
  )
}

/// Which Vault shows, where its person owns more than their own.
function Whose() {
  const pick = (owner: string) => {
    setOpen()
    setSheet()
    setWhose(owner)
  }
  return (
    <Show when={owners().length > 1}>
      <Field label="Owner">
        <Select.Root items={owners()} value={security().owner} onValueChange={pick} />
      </Field>
    </Show>
  )
}

/// An item's grey line when it's ready: what its app's component says of it, its websites, or what made it.
function Summary(props: { item: Item }) {
  const type = () => typeOf(props.item)
  const drawn = createMemo(() => {
    const it = type()
    return it?.trusted && it.component ? component(it.app, it.component).then((module) => module.Summary) : undefined
  })
  const plain = () => {
    const it = type()
    if (it) return it.title ?? it.name
    if (props.item.hosts.length) return listed(props.item.hosts)
    return props.item.added ? `Added by an agent from ${props.item.added.from}` : "No website"
  }
  return (
    <Errored fallback={plain()}>
      <Loading fallback={plain()}>
        <Show when={drawn()} keyed fallback={plain()}>
          {(Drawn) => <Drawn item={shown(props.item)} subject={whose()} />}
        </Show>
      </Loading>
    </Errored>
  )
}

/// An item opens beside the list.
function choose(item: Item) {
  setOpen(item.name)
}

/// What's open beside the list: an item of a type with a component, drawn by its trusted app's, one of an untrusted
/// app's by its page for it (client/<component>.html) in a frame of its own, where it keeps what an app of its own may
/// do, or else its own.
function Detail(props: { item: Item }) {
  // The same type while what draws its item is: the list asked again, as it is whenever anything changes, gives new
  // objects, and a new one would draw the item from scratch, losing what's open in it (a live screen's stream).
  const type = createMemo(() => typeOf(props.item), {
    equals: (a, b) => a?.id === b?.id && a?.app === b?.app && a?.component === b?.component && a?.trusted === b?.trusted && a?.prefix === b?.prefix,
  })
  const drawn = () => (type()?.component && type()!.trusted ? type() : undefined)
  const framed = () => {
    const it = type()
    return it?.component && !it.trusted ? `${it.app}/client/${it.component}.html#${encodeURIComponent(shown(props.item))}` : undefined
  }
  return (
    <Show when={drawn()} keyed fallback={<Show when={framed()} keyed fallback={<ItemScreen item={props.item} />}>{(src) => <Frame src={src} subject={whose()} title={shown(props.item)} style={look.fill} />}</Show>}>
      {(type) => (
        <Errored fallback={(error) => <Problem heading={`Can't show ${shown(props.item)}`} text={error() as Error} />}>
          <Loading fallback={<Skeleton rows={4} />}>
            <Drawn type={type} item={props.item} />
          </Loading>
        </Errored>
      )}
    </Show>
  )
}

/// An item of a type with a component, drawn by its app's own, in this page, with what the Vault's page may do.
function Drawn(props: { type: ItemType; item: Item }) {
  const drawn = createMemo(() => component(props.type.app, props.type.component!).then((module) => module.default))
  return (
    <Show when={drawn()} keyed>
      {(Drawn) => <Drawn item={shown(props.item)} subject={whose()} open={(name: string) => setOpen(props.type.prefix + name)} close={() => setOpen()} />}
    </Show>
  )
}

/// One item: its details, its websites, and who has access, each switched here. Then Delete Item.
function ItemScreen(props: { item: Item }) {
  const item = () => props.item
  // Deleting it is soft delete's, as for anything an app made: it's in Recently Deleted for 30 days.
  // Access that wouldn't switch is said under the list, and the switch stays as it is stored.
  const [problem, setProblem] = createSignal<{ heading: string; text: Error }>()
  // How a switch was switched, until what's stored says so too, or it wouldn't switch and goes back.
  const [switched, setSwitched] = createSignal<Record<string, boolean>>({})
  const has = (who: string) => switched()[who] ?? item().access.includes(who)
  const switchAccess = async (who: string, title: string, on: boolean) => {
    setSwitched((all) => ({ ...all, [who]: on }))
    setProblem()
    try {
      await call("vault_access", { item: item().name, who, on }, { subject: whose() })
      again()
    } catch (error) {
      setSwitched(({ [who]: _, ...rest }) => rest)
      setProblem({ heading: `Couldn't switch ${title} ${on ? "on" : "off"}`, text: error as Error })
    }
  }
  const remove = async () => {
    if (!(await confirm({ heading: `Delete ${shown(item())}?`, text: "Whatever uses it stops working. It's in Recently Deleted for 30 days.", action: "Delete", destructive: true }))) return
    if (await does("soft-delete_delete", { artifact: item().artifact })) setOpen()
  }
  return (
    <Screen
      heading={
        <View style={look.heading}>
          <Tile item={item()} />
          <Text>{shown(item())}</Text>
        </View>
      }
      back
    >
      <Show when={item().status}>
        {(status) => (
          <View style={look.status}>
            <View style={look.grows}>
              <Line status={status()} />
            </View>
            <Fix item={item()} />
          </View>
        )}
      </Show>
      <Section heading="Details">
        <List>
          <For each={item().fields}>
            {(field) => <Row heading={label(field.name)} end={<Text style={field.secret && look.note}>{field.secret ? "••••••••" : field.value}</Text>} onPress={() => setSheet({ kind: "field", item: item(), field })} />}
          </For>
          <Show when={item().added}>{(added) => <Row heading="Added by an agent" text={whence(added())} />}</Show>
        </List>
        <Button variant="secondary" onPress={() => setSheet({ kind: "field", item: item() })}>
          Add Field
        </Button>
      </Section>
      <Section heading={item().hosts.length === 1 ? "Website" : "Websites"}>
        <Show when={item().hosts.length} fallback={<Empty>None: its secrets go nowhere.</Empty>}>
          <List each={item().hosts}>{(host) => <Row heading={host()} />}</List>
        </Show>
      </Section>
      <Section heading="Access">
        <List>
          <Row
            heading="Your agents"
            text="Use it, never see it"
            end={<Switch aria-label="Your agents" value={has("agents")} onValueChange={(on) => switchAccess("agents", "Your agents", on)} />}
          />
          <For each={contents().apps}>
            {(app) => (
              <Row
                heading={app.title}
                end={<Switch aria-label={app.title} value={has(app.name)} onValueChange={(on) => switchAccess(app.name, app.title, on)} />}
              />
            )}
          </For>
        </List>
        <Show when={problem()}>{(problem) => <Problem heading={problem().heading} text={problem().text} />}</Show>
      </Section>
      <List>
        <Row heading="Delete Item…" text="It's in Recently Deleted for 30 days." destructive onPress={remove} />
      </List>
    </Screen>
  )
}

/// A field of an item to type: a new item's first, a new one of an item's, or one that's there to change. It stays
/// open until it's saved, so what's typed isn't lost to a refusal.
function FieldSheet() {
  const at = () => {
    const open = sheet()
    return open?.kind === "field" ? open : undefined
  }
  const [secret, setSecret] = createSignal(true)
  const [name, setName] = createSignal("")
  const [field, setField] = createSignal("")
  const [value, setValue] = createSignal("")
  const [hosts, setHosts] = createSignal("")
  // Each time it opens, it starts empty, or with the field's value, and a new field starts secret.
  createEffect(at, (now) => {
    if (!now) return
    setSecret(true)
    setName("")
    setField("")
    setValue(now.field?.value ?? "")
    setHosts("")
  })
  // A field's name is a word of letters, digits and _, as a tool's input names it.
  const named = () => /^[A-Za-z0-9_]+$/.test(field())
  const [busy, setBusy] = createSignal(false)
  createEffect(at, () => void setBusy(false))
  const ready = () => !busy() && !!value() && (!!at()?.item || !!name().trim()) && (!!at()?.field || named())
  const save = async () => {
    const { item, field: changing } = at()!
    const called = item?.name ?? name().trim()
    if (!item && contents().items.some((other) => other.name.toLowerCase() === called.toLowerCase())) return void toast(`There's an item named ${called} already.`)
    const inputs: Record<string, unknown> = { item: called, field: changing?.name ?? field(), value: value(), secret: changing ? changing.secret : secret() }
    if (!item) inputs.hosts = sites(hosts())
    setBusy(true)
    const saved = await does("vault_set", inputs)
    setBusy(false)
    if (!saved) return
    shut()
    if (!item) setOpen(called)
  }
  const remove = async () => {
    const { item, field } = at() as { item: Item; field: ItemField }
    if (!(await confirm({ heading: `Delete ${label(field.name)}?`, text: "It can't be brought back.", action: "Delete", destructive: true }))) return
    if (await does("vault_remove_field", { item: item.name, field: field.name })) shut()
  }
  /** Return saves what's ready, as Save does. */
  const submit = () => ready() && save()
  const hidden = () => (at()!.field ? at()!.field!.secret : secret())
  return (
    <Sheet heading={at()?.field ? label(at()!.field!.name) : at()?.item ? "Add Field" : "Add Item"} open={!!at()} onClose={shut}>
      <Show when={at()}>
        <Show when={!at()!.item}>
          <Field label="Name">
            <Input placeholder="GitHub" required autoFocus value={name()} onChangeText={setName} onSubmitEditing={submit} />
          </Field>
        </Show>
        <Show when={!at()!.field}>
          <Field label="Field" error={field() && !named() ? "Only letters, digits and _." : undefined}>
            <Input placeholder="token" required autoCapitalize="none" autoCorrect={false} spellCheck={false} value={field()} onChangeText={setField} onSubmitEditing={submit} />
          </Field>
        </Show>
        <Field label={at()!.field ? "New value" : "Value"} hint={hidden() ? "It's encrypted on this device, and never shown again." : undefined}>
          <Input secureTextEntry={hidden()} autoComplete="off" autoCapitalize="none" autoCorrect={false} spellCheck={false} required autoFocus={!!at()!.item} value={value()} onChangeText={setValue} onSubmitEditing={submit} />
        </Field>
        <Show when={!at()!.field}>
          <Checkbox label="Secret" value={secret()} onValueChange={setSecret} />
        </Show>
        <Show when={!at()!.item}>
          <Field label="Websites" hint="Where its secrets may be sent, like api.github.com.">
            <Input inputMode="url" autoCapitalize="none" autoCorrect={false} spellCheck={false} value={hosts()} onChangeText={setHosts} onSubmitEditing={submit} />
          </Field>
        </Show>
        <Button loading={busy()} disabled={!ready()} onPress={save}>
          Save
        </Button>
        <Show when={at()!.field}>
          <Button variant="ghost" onPress={remove}>
            Delete Field
          </Button>
        </Show>
      </Show>
    </Sheet>
  )
}

/// Adds an item of a type, as the type says: asked for a name first, where it needs one.
const create = (type: ItemType) => (type.create!.name ? setSheet({ kind: "create", type }) : act(type.create!, type.app))

/// The kinds of item a section's several types add, each adding one.
function AddSheet() {
  const section = () => {
    const open = sheet()
    return open?.kind === "add" ? open.section : undefined
  }
  const types = () => contents().types.filter((type) => type.section === section() && type.create && type.trusted)
  return (
    <Sheet heading={section() ? `Add ${titled(section()!.replace(/s$/, ""))}` : ""} open={!!section()} onClose={shut}>
      <List each={types()} keyed={(type) => type.id}>
        {(type) => <Row start={<Image src={type().icon ?? ""} alt="" style={look.tile} />} heading={titled(type().name)} onPress={() => (shut(), create(type()))} />}
      </List>
    </Sheet>
  )
}

/// A new item of a type: its name, which the type's app makes it from (`create`), and then it opens.
function CreateSheet() {
  const type = () => {
    const open = sheet()
    return open?.kind === "create" ? open.type : undefined
  }
  const [name, setName] = createSignal("")
  const [busy, setBusy] = createSignal(false)
  createEffect(type, (now) => void (now && setName("")))
  const make = async () => {
    const made = type()!
    const called = name().trim()
    setBusy(true)
    const done = await act(made.create!, made.app, { name: called })
    setBusy(false)
    if (done) {
      shut()
      setOpen(made.prefix + called)
    }
  }
  return (
    <Sheet heading={type() ? `Add ${titled(type()!.name)}` : ""} open={!!type()} onClose={shut}>
      <Field label="Name">
        <Input required autoFocus value={name()} onChangeText={setName} onSubmitEditing={() => name().trim() && make()} />
      </Field>
      <Button loading={busy()} disabled={!name().trim()} onPress={make}>
        Add
      </Button>
    </Sheet>
  )
}

/// Who asks, as a person reads it: the app's title, the agent's name, or an agent.
const asker = (request: Request) => request.app?.title ?? request.by ?? "An agent"
/// What it asks for: an account where it wants a name with its secret, else a key.
const wanted = (request: Request) => (request.fields.some((field) => field.secret === false) ? "an account" : "a key")

/// A request, as a system's permission prompt asks: "<who> wants to use a key", why, where it's sent, which item, and
/// Don't Allow or Allow. Only plain items are offered, as a password manager offers logins: one an app keeps (a Chrome
/// profile) is that app's. The one named as the request is comes first and is chosen, so Allow is one tap; with none,
/// New Item is, and its fields are typed here. It's the UI library's `Alert`, since `confirm` asks only a question.
function RequestAlert() {
  const request = asking
  const named = (item: Item) => item.name.toLowerCase() === request()!.title.toLowerCase()
  const fits = () =>
    request()
      ? contents()
          .items.filter((item) => !item.type && request()!.fields.every((wanted) => item.fields.some((field) => field.name === wanted.name)))
          .sort((a, b) => Number(named(b)) - Number(named(a)))
      : []
  const [chosen, setChosen] = createSignal<string>()
  const [typed, setTyped] = createSignal<Record<string, string>>({})
  const [problem, setProblem] = createSignal<string>()
  const [busy, setBusy] = createSignal(false)
  const item = () => chosen() ?? fits()[0]?.name ?? ""
  const options = createMemo(() => [...fits().map((item) => ({ value: item.name, label: item.name })), { value: "", label: "New Item" }])
  // Each request asked about starts with nothing chosen or typed.
  createEffect(
    () => request()?.id,
    (now) => {
      if (!now) return
      setChosen()
      setTyped({})
      setProblem()
    },
  )
  const answer = async (inputs: object) => {
    setBusy(true)
    setProblem()
    try {
      await call("vault_answer", { id: request()!.id, ...inputs }, { subject: whose() })
      again()
      setAsking()
    } catch (error) {
      setProblem((error as Error).message)
    } finally {
      setBusy(false)
    }
  }
  const filled = () => !!item() || request()!.fields.every(({ name }) => typed()[name])
  const allow = () => answer(item() ? { item: item() } : { values: JSON.stringify(Object.fromEntries(request()!.fields.map(({ name }) => [name, typed()[name]]))) })
  const where = () => {
    const hosts = request()!.hosts
    const sent = hosts.length ? `Sent only to ${listed(hosts)}` : "Sent to no new website"
    return request()!.app ? `${sent}.` : `${sent}, and ${request()!.by ?? "the agent"} never sees it.`
  }
  return (
    <Alert
      heading={request() ? `${asker(request()!)} wants to use ${wanted(request()!)}` : ""}
      text={request() ? `${request()!.why ? `${request()!.why!.trim().replace(/[^.!?]$/, "$&.")} ` : ""}${where()}` : undefined}
      open={!!request()}
      onClose={() => setAsking()}
      action={
        <>
          <Button variant="outline" disabled={busy()} onPress={() => answer({ decline: true })}>
            Don't Allow
          </Button>
          <Button loading={busy()} disabled={!request() || !filled()} onPress={allow}>
            Allow
          </Button>
        </>
      }
    >
      <Show when={request()}>
        <Field label={request()!.title}>
          <Select.Root items={options()} value={item()} onValueChange={setChosen} />
        </Field>
        <Show when={!item()}>
          <For each={request()!.fields}>
            {(field, at) => (
              <Field label={label(field.name)}>
                <Input
                  secureTextEntry={field.secret !== false}
                  required
                  autoFocus={at() === 0}
                  value={typed()[field.name] ?? ""}
                  onChangeText={(text) => setTyped((all) => ({ ...all, [field.name]: text }))}
                  onSubmitEditing={() => filled() && !busy() && allow()}
                />
              </Field>
            )}
          </For>
        </Show>
        <Show when={problem()}>
          <Text role="alert" style={look.wrong}>
            {problem()}
          </Text>
        </Show>
      </Show>
    </Alert>
  )
}

/// Everything about the key that keeps the Vault, in Apple's words: this device and the other trusted ones, with any
/// asking to be; the recovery key; and the servers that can open it.
function SecurityScreen() {
  const others = () => security().devices.filter((device) => device.device !== security().device)
  return (
    <Screen heading="Security" back>
      <Readiness />
      <Show when={security().readiness !== "not_set_up"}>
        <Section heading="Trusted Devices">
          <List>
            <Row heading="This Device" text={security().holds ? "Holds the key" : "Doesn't hold the key"} />
            <For each={others()}>{(device) => <Row heading={device.name} text="Holds the key" />}</For>
          </List>
        </Section>
        <Show
          when={security().holds}
          fallback={
            <Section heading="Trust This Device">
              <List>
                <Show when={own()}>
                  <Row heading="Unlock with Passkey" text="Use a passkey that unlocks your key" onPress={unlockWithPasskey} />
                </Show>
                <Row heading="Use Recovery Key" text="Type the key you wrote down" onPress={() => useRecoveryKey({ subject: whose() }).then(again)} />
              </List>
            </Section>
          }
        >
          <Section heading="Recovery Key">
            <List>
              <Row heading="Show Recovery Key" text="For when every device is gone" onPress={() => showRecoveryKey({ subject: whose() })} />
            </List>
          </Section>
        </Show>
        <Section heading="Servers">
          <Text style={look.note}>{`The servers that run ${whom()} apps open ${whom()} Vault, once your passkey hands them its key.`}</Text>
          <List each={security().may}>
            {(server) => (
              <Row
                heading={server().name ?? "A server"}
                text={security().servers.includes(server().key) ? `Opens ${whom()} Vault` : "Waiting for its key"}
                action={security().servers.includes(server().key) ? undefined : <Button variant="secondary" onPress={unlockWithPasskey}>Use Passkey</Button>}
              />
            )}
          </List>
        </Section>
      </Show>
    </Screen>
  )
}

/// What its app says an item is in, in a row's grey line or atop its screen: a spinner and what's under way, or a
/// dot and why. Ready is nothing: fine is silent.
function Line(props: { status: Status }) {
  return (
    <View style={look.line}>
      <Show when={props.status.state === "busy"} fallback={<View aria-hidden="true" style={look.dot} />}>
        <Spinner />
      </Show>
      <Text style={look.shrinks}>{props.status.message}</Text>
    </View>
  )
}

/// The one action that fixes what its app says an item needs, if it says one.
function Fix(props: { item: Item }) {
  const [busy, setBusy] = createSignal(false)
  const action = () => (props.item.status?.state === "needs_action" ? props.item.status.action : undefined)
  const run = async () => {
    setBusy(true)
    await act(action()!, props.item.status!.by)
    setBusy(false)
  }
  return (
    <Show when={action()}>
      <Button variant="outline" loading={busy()} onPress={run}>
        {action()!.label}
      </Button>
    </Show>
  )
}
/// Whether an item has an action that fixes it.
const fixable = (item: Item) => item.status?.state === "needs_action" && !!item.status.action

/// Where a client reads an icon the server gave as `ui://icons/<sha256>/…`: beside this page's own files.
const src = (icon: string) => icon.replace(/^ui:\/\/([^/]+)\/([^/]+)\//, "../../$1@$2/")

/// A password's or key's grey line: what it is, and who uses it, each app by its icon and name, or agents. The row draws
/// it inside a `Text`, so each app's `View` is a run of it and its `gap` does nothing: the spaces are in the words.
function UsedBy(props: { item: Item }) {
  const apps = () => props.item.access.filter((who) => who !== "agents").map((name) => contents().apps.find((app) => app.name === name) ?? { name, title: name, icon: null })
  const what = () => props.item.fields.find((field) => field.secret)?.name
  return (
    <View style={look.inline}>
      <Text>{[what() && label(what()!), apps().length ? "used by " : props.item.access.includes("agents") ? "agents may use it" : "used by nothing yet"].filter(Boolean).join(" · ")}</Text>
      <For each={apps()}>
        {(app, at) => (
          <View style={look.inline}>
            <Show when={app.icon}>{(icon) => <Image src={src(icon())} alt="" style={look.small} />}</Show>
            <Text>{`${app.title}${at() < apps().length - 1 ? ", " : ""}`}</Text>
          </View>
        )}
      </For>
    </View>
  )
}

/// An item's row: its tile, its name, and its line: what's wrong, who uses it, or what its app says.
function ItemRow(props: { item: Item }) {
  return (
    <Row
      start={<Tile item={props.item} />}
      heading={shown(props.item)}
      text={props.item.status ? <Line status={props.item.status} /> : props.item.kind === "password" ? <UsedBy item={props.item} /> : <Summary item={props.item} />}
      current={props.item.name === open()}
      onPress={() => choose(props.item)}
      action={fixable(props.item) ? <Fix item={props.item} /> : undefined}
    />
  )
}

/// A request waiting for its person, as a banner atop Accounts: who asks for what, why, and Don't Allow or Choose…,
/// which opens its alert.
function Banner(props: { request: Request }) {
  const icon = () => contents().apps.find((app) => app.name === props.request.app?.name)?.icon
  return (
    <List>
      <Row
        start={<Show when={icon()}>{(icon) => <Image src={src(icon())} alt="" style={look.tile} />}</Show>}
        heading={`${asker(props.request)} asks for ${props.request.title}`}
        text={props.request.why ?? undefined}
        action={
          <View style={look.inline}>
            <Button variant="ghost" onPress={() => does("vault_answer", { id: props.request.id, decline: true })}>Don't Allow</Button>
            <Button variant="outline" onPress={() => setAsking(props.request)}>Choose…</Button>
          </View>
        }
      />
    </List>
  )
}

/// Accounts, as Settings draws it, laid out as macOS's Internet Accounts and Passwords are together: the requests
/// waiting, as banners; your accounts, with Add Account; your passwords and keys, each with the apps that use it; then
/// each other kind of item an app keeps here, under its app's icon and name; then Security. An item opens over the
/// page, drawn by its own app where it has a component.
export default function Accounts() {
  return (
    <Errored fallback={(error) => <Screen heading="Vault" back><Problem heading="Can't show the Vault" text={error() as Error} /></Screen>}>
      <Vault />
    </Errored>
  )
}

function Vault() {
  // A link to a request, `one:settings#vault/accounts&request=<id>`, as a request's own link leads to: its alert,
  // once the Vault has listed it.
  const [linked, setLinked] = createSignal<string>()
  createEffect(link, (to) => {
    const id = to && /(?:^|&)request=([^&]*)/.exec(to)?.[1]
    if (id) setLinked(decodeURIComponent(id))
  })
  createEffect(
    () => linked() && waiting().find(({ id }) => id === linked()),
    (request) => {
      if (!untrack(linked)) return
      setLinked()
      if (request) setAsking(request)
      else toast("That request isn't waiting any more.")
    },
  )
  const items = (to: Place) => contents().items.filter((item) => placeOf(item) === to)
  const addAccount = () => (creatable("accounts").length === 1 ? create(creatable("accounts")[0]) : setSheet({ kind: "add", section: "Accounts" }))
  const typeIn = (section: string) => contents().types.find((type) => type.section === section)
  const item = () => contents().items.find(({ name }) => name === open())
  const ready = () => !!security().servers.length
  return (
    <>
      <Split
        open={!!item() || secure()}
        onClose={() => (setOpen(), setSecure(false))}
        pick="Pick an item to see it."
        detail={<Show when={item()} fallback={<Show when={secure()}><SecurityScreen /></Show>}>{(item) => <Detail item={item()} />}</Show>}
      >
        <Screen heading="Vault" back>
          <Whose />
          <Readiness />
          <For each={waiting()} keyed={(request) => request.id}>{(request) => <Banner request={request()} />}</For>
          <Show when={ready()}>
            <Section>
              <Show when={items("accounts").length}>
                <List each={items("accounts")} keyed={(item) => item.artifact}>{(item) => <ItemRow item={item()} />}</List>
              </Show>
              <Show when={creatable("accounts").length}>
                <List>
                  <Row heading="Sign in to your accounts for your apps." action={<Button variant="secondary" onPress={addAccount}>Add Account</Button>} />
                </List>
              </Show>
            </Section>
            <Section heading="Passwords and Keys">
              <Show when={items("passwords").length}>
                <List each={items("passwords")} keyed={(item) => item.artifact}>{(item) => <ItemRow item={item()} />}</List>
              </Show>
              <List>
                <Row heading="Keep a password or key for your apps and agents." action={<Button variant="secondary" onPress={() => setSheet({ kind: "field" })}>Add</Button>} />
              </List>
            </Section>
            <For each={sections()}>
              {(section) => (
                <Section
                  aria-label={section}
                  heading={
                    <View style={look.inline}>
                      <Show when={typeIn(section)?.icon}>{(icon) => <Image src={icon()} alt="" style={look.small} />}</Show>
                      <Text>{section}</Text>
                    </View>
                  }
                >
                  <Show when={items(`section:${section}`).length} fallback={<Empty>None yet.</Empty>}>
                    <List each={items(`section:${section}`)} keyed={(item) => item.artifact}>{(item) => <ItemRow item={item()} />}</List>
                  </Show>
                  <Show when={creatable(`section:${section}`).length}>
                    <List>
                      <Row
                        heading={`Add a ${typeIn(section)!.name} for your apps.`}
                        action={
                          <Button variant="secondary" onPress={() => (creatable(`section:${section}`).length === 1 ? create(creatable(`section:${section}`)[0]) : setSheet({ kind: "add", section }))}>
                            {`Add ${titled(typeIn(section)!.name)}`}
                          </Button>
                        }
                      />
                    </List>
                  </Show>
                </Section>
              )}
            </For>
          </Show>
          <List>
            <Row heading="Security" end={security().readiness !== "ready" ? <View role="img" aria-label="Needs you" style={look.dot} /> : undefined} current={secure()} onPress={enter} />
          </List>
        </Screen>
      </Split>
      <FieldSheet />
      <CreateSheet />
      <AddSheet />
      <RequestAlert />
    </>
  )
}
