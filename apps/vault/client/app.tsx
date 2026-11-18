// The Vault, laid out as Apple's Passwords app is: a sidebar of the kinds of item (All, Passwords & Keys, Accounts and
// a place for each other kind an app keeps here, `vault.types`), then Requests while something waits, Security and
// Recently Deleted; then a searchable list of what's in the place picked, then the item opened from it. On a phone
// the sidebar is the first screen and each is tapped into; on a computer they're side by side, three columns, or two
// where it's narrow. An item shows its details (a secret one as dots, never shown), its websites, and who has access:
// your agents and each app, switched here. One of a type with a component is drawn by its own app. Deleting one is
// soft delete's. Security is everything about the key that keeps it. What's missing before the Vault works shows as
// one box atop the list, with the one thing that fixes it. A request is an alert, as a system's permission prompt is.
// It's written in strict mode (the UI library's skill), so it runs on a native host as it does on the DOM.
import { createEffect, createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch as Choose, untrack } from "solid-js"
import { Alert, Badge, Box, Button, call, changed, Checkbox, component, confirm, createListCollection, Empty, Field, Image, Input, link, List, open as opening, Problem, render, Row, Screen, Search, Section, Select, Sheet, showRecoveryKey, Skeleton, Spinner, Split, Switch, Text, toast, useRecoveryKey } from "ui"

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
    again()
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

/// Where you are, as the sidebar picks it, All until it does; whether it's been tapped into, which on a phone shows
/// it over the sidebar, a step the device's back goes back from (the outer Split's).
const [place, setPlace] = createSignal<Place>("all")
const [entered, setEntered] = createSignal(false)
/// The item open beside the list, by its name; the sheet open over the page; and the request asked about.
const [open, setOpen] = createSignal<string>()
const [sheet, setSheet] = createSignal<Sheet>()
const [asking, setAsking] = createSignal<Request>()
const shut = () => setSheet()

/// Goes to a place in the sidebar.
function enter(to: Place) {
  setOpen()
  setPlace(to)
  setEntered(true)
}

/// An app's icon, or an item's picture, at the start of its row: its flat tile, as on the Home Screen.
const tile = { width: 32, height: 32, borderRadius: "md" } as const

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
  return <Image src={typeOf(props.item)?.icon ?? letter()} alt="" style={tile} />
}

/// Asks a trusted device to let this one hold the key.
const askAnother = async () => (await does("encryption_ask", {})) && toast("Asked. Approve this device on a trusted one, in Vault's Security.")

/// Whether this Vault is kept with its person's own key, which their passkeys unlock: a group's or the network's isn't.
const own = () => security().owner.startsWith("users/")

/// Lets this device hold its person's key with a passkey that unlocks it: it asks as it does another device, and the
/// network's sign-in page, in the device's browser, lets it in once the passkey's used there (`sign-in`'s
/// `unlock_with_passkey`). Coming back, the Vault shows it holds the key.
async function unlockWithPasskey() {
  try {
    await call("encryption_ask", {})
    await opening(JSON.parse(await call("sign-in_unlock_with_passkey", { device: security().device })).link)
  } catch (error) {
    toast(error as Error)
  }
}

/// Has this device, which holds its person's key, encrypt it to each of their passkeys that can unlock it and doesn't
/// yet (`sign-in_encrypt_for_passkeys`), as a passkey's first used or the key's first made.
const encryptForPasskeys = () => own() && security().holds && call("sign-in_encrypt_for_passkeys").catch(toast)

/// What's missing before the Vault works, atop the list: one box, saying what and the one thing that fixes it.
function Readiness() {
  const setUp = async () => {
    if (!(await does("encryption_approve", {}))) return
    await showRecoveryKey({ subject: whose() })
    if (own()) await call("sign-in_encrypt_for_passkeys").catch(toast)
  }
  return (
    <Choose>
      <Match when={security().readiness === "not_set_up"}>
        <Problem warn heading={`Set up ${whom()} Vault`} text="Passwords, keys and sign-ins are kept here, encrypted with a key only your devices hold." action={<Button onClick={setUp}>Set Up</Button>} />
      </Match>
      <Match when={security().readiness === "no_server"}>
        <Problem
          warn
          heading={`No server can open ${whom()} Vault`}
          text={security().holds ? "Until one can, it keeps nothing." : "Until one can, it keeps nothing. Choose one on a trusted device."}
          action={
            <Show when={place() !== "security"}>
              <Button variant="outline" onClick={() => enter("security")}>{security().holds ? "Choose a Server" : "Open Security"}</Button>
            </Show>
          }
        />
      </Match>
      <Match when={security().readiness === "device_lacks_key"}>
        <Problem
          warn
          heading={`This device doesn't hold ${whom()} key`}
          text="It adds and uses items. Approving devices and choosing servers takes a trusted device."
          action={
            // Security lists the same three, under Trust This Device.
            <Show when={place() !== "security"}>
              <Show when={own()}>
                <Button onClick={unlockWithPasskey}>Unlock with Passkey</Button>
              </Show>
              <Button variant="outline" onClick={askAnother}>Ask Another Device</Button>
              <Button variant="outline" onClick={() => useRecoveryKey({ subject: whose() }).then(again)}>Use Recovery Key</Button>
            </Show>
          }
        />
      </Match>
    </Choose>
  )
}

/// The sidebar: the kinds of item, each with how many; Requests while something waits; Security; Recently Deleted.
function Sidebar() {
  const count = (to: Place) => contents().items.filter((item) => to === "all" || placeOf(item) === to).length
  // Where the sidebar is the first screen, nothing in it is current (the Split's).
  const at = (to: Place) => place() === to
  const kinds = (): { to: Place; label: string }[] => [
    { to: "all", label: "All" },
    { to: "passwords", label: "Passwords & Keys" },
    ...(count("accounts") || creatable("accounts").length ? [{ to: "accounts" as Place, label: "Accounts" }] : []),
    ...sections().map((section) => ({ to: `section:${section}` as Place, label: section })),
  ]
  return (
    <Screen heading="Vault">
      <Whose />
      <List>
        <For each={kinds()}>{(kind) => <Row heading={kind.label} end={<Text style={{ color: "muted-foreground" }}>{count(kind.to)}</Text>} current={at(kind.to)} onClick={() => enter(kind.to)} />}</For>
        <Show when={waiting().length}>
          <Row heading="Requests" end={<Badge variant="destructive" aria-label={`${waiting().length} waiting`}>{waiting().length}</Badge>} current={at("requests")} onClick={() => enter("requests")} />
        </Show>
      </List>
      <List>
        <Row heading="Security" end={security().readiness !== "ready" ? <Box role="img" aria-label="Needs you" style={dot} /> : undefined} current={at("security")} onClick={() => enter("security")} />
        <Row heading="Recently Deleted" onClick={() => opening("soft-delete", { subject: whose() })} />
      </List>
    </Screen>
  )
}

/// Which Vault shows, where its person owns more than their own.
function Whose() {
  const collection = createMemo(() => createListCollection({ items: owners().map(({ value, label }) => ({ value, label })) }))
  const pick = ({ value }: { value: string[] }) => {
    setOpen()
    setSheet()
    setWhose(value[0])
  }
  return (
    <Show when={owners().length > 1}>
      <Field label="Owner">
        <Select.Root collection={collection()} value={[security().owner]} onValueChange={pick} />
      </Field>
    </Show>
  )
}

/// The heading of a place of items.
const headingOf = (place: Place) => (place === "all" ? "All" : place === "passwords" ? "Passwords & Keys" : place === "accounts" ? "Accounts" : place.slice("section:".length))

/// The items in a place: searchable, each opening beside the list, with a way to add one.
function Items(props: { place: Place }) {
  const [words, setWords] = createSignal("")
  const found = () => {
    const wanted = words().trim().toLowerCase()
    return contents()
      .items.filter((item) => props.place === "all" || placeOf(item) === props.place)
      .filter((item) => !wanted || [shown(item), ...item.hosts, typeOf(item)?.title ?? ""].some((text) => text.toLowerCase().includes(wanted)))
      .sort((a, b) => shown(a).localeCompare(shown(b)))
  }
  const item = () => contents().items.find(({ name }) => name === open())
  const makes = () => creatable(props.place)
  const add = () => {
    if (props.place === "all" || props.place === "passwords") return setSheet({ kind: "field" })
    if (makes().length === 1) return create(makes()[0])
    setSheet({ kind: "add", section: props.place === "accounts" ? "Accounts" : props.place.slice("section:".length) })
  }
  const addable = () => !!security().servers.length && (props.place === "all" || props.place === "passwords" || makes().length > 0)
  return (
    <Split
      open={!!item()}
      onClose={() => setOpen()}
      detail={<Show when={item()} fallback={<Show when={found().length}><Empty>Pick an item to see what it holds.</Empty></Show>}>{(item) => <Detail item={item()} />}</Show>}
    >
      <Screen heading={headingOf(props.place)} back action={<Show when={addable()}><Button onClick={add}>Add</Button></Show>}>
        <Readiness />
        <Show when={security().servers.length}>
          <Search placeholder="Search" aria-label={`Search ${headingOf(props.place)}`} value={words()} onInput={(event) => setWords(event.currentTarget.value)} />
          <Show when={found().length} fallback={<Empty>{words() ? "Nothing matches." : "None yet."}</Empty>}>
            <List>
              <For each={found()} keyed={(item) => item.artifact}>
                {(item) => (
                  <Row
                    start={<Tile item={item()} />}
                    heading={shown(item())}
                    text={item().status ? <Line status={item().status!} /> : <Summary item={item()} />}
                    current={item().name === open()}
                    onClick={() => choose(item())}
                    action={fixable(item()) ? <Fix item={item()} /> : undefined}
                  />
                )}
              </For>
            </List>
          </Show>
        </Show>
      </Screen>
    </Split>
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

/// An item of a type with a component opens beside the list, but one of an untrusted app's opens in a frame of its
/// own, its page for it (client/<component>.html), where it keeps what an app of its own may do.
function choose(item: Item) {
  const type = typeOf(item)
  if (type?.component && !type.trusted) return opening(`${type.app}/client/${type.component}.html#${encodeURIComponent(shown(item))}`, { subject: whose() })
  setOpen(item.name)
}

/// What's open beside the list: an item of a type with a component, drawn by its trusted app's, or else its own.
function Detail(props: { item: Item }) {
  const drawn = () => {
    const type = typeOf(props.item)
    return type?.component && type.trusted ? type : undefined
  }
  return (
    <Show when={drawn()} keyed fallback={<ItemScreen item={props.item} />}>
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
        <Box style={{ flexDirection: "row", alignItems: "center", gap: 10, minWidth: 0 }}>
          <Tile item={item()} />
          <Text>{shown(item())}</Text>
        </Box>
      }
      back
    >
      <Show when={item().status}>
        {(status) => (
          <Box style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
            <Box style={{ flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
              <Line status={status()} />
            </Box>
            <Fix item={item()} />
          </Box>
        )}
      </Show>
      <Section heading="Details">
        <List>
          <For each={item().fields}>
            {(field) => <Row heading={label(field.name)} end={<Text style={field.secret ? { color: "muted-foreground" } : undefined}>{field.secret ? "••••••••" : field.value}</Text>} onClick={() => setSheet({ kind: "field", item: item(), field })} />}
          </For>
          <Show when={item().added}>{(added) => <Row heading="Added by an agent" text={whence(added())} />}</Show>
        </List>
        <Button variant="secondary" onClick={() => setSheet({ kind: "field", item: item() })}>
          Add Field
        </Button>
      </Section>
      <Section heading={item().hosts.length === 1 ? "Website" : "Websites"}>
        <Show when={item().hosts.length} fallback={<Empty>None: its secrets go nowhere.</Empty>}>
          <List>
            <For each={item().hosts}>{(host) => <Row heading={host} />}</For>
          </List>
        </Show>
      </Section>
      <Section heading="Access">
        <List>
          <Row
            heading="Your agents"
            text="Use it, never see it"
            end={<Switch aria-label="Your agents" checked={has("agents")} onChange={(event) => switchAccess("agents", "Your agents", event.currentTarget.checked)} />}
          />
          <For each={contents().apps}>
            {(app) => (
              <Row
                heading={app.title}
                end={<Switch aria-label={app.title} checked={has(app.name)} onChange={(event) => switchAccess(app.name, app.title, event.currentTarget.checked)} />}
              />
            )}
          </For>
        </List>
        <Show when={problem()}>{(problem) => <Problem heading={problem().heading} text={problem().text} />}</Show>
      </Section>
      <Section>
        <Button variant="destructive" onClick={remove}>
          Delete Item
        </Button>
      </Section>
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
  const ready = () => !!value() && (!!at()?.item || !!name().trim()) && (!!at()?.field || named())
  const save = async () => {
    const { item, field: changing } = at()!
    const called = item?.name ?? name().trim()
    if (!item && contents().items.some((other) => other.name.toLowerCase() === called.toLowerCase())) return void toast(`There's an item named ${called} already.`)
    const inputs: Record<string, unknown> = { item: called, field: changing?.name ?? field(), value: value(), secret: changing ? changing.secret : secret() }
    if (!item) inputs.hosts = sites(hosts())
    if (!(await does("vault_set", inputs))) return
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
            <Input placeholder="GitHub" required autofocus value={name()} onInput={(event) => setName(event.currentTarget.value)} onAction={submit} />
          </Field>
        </Show>
        <Show when={!at()!.field}>
          <Field label="Field" error={field() && !named() ? "Only letters, digits and _." : undefined}>
            <Input placeholder="token" required autocapitalize="none" autocorrect={false} spellcheck={false} value={field()} onInput={(event) => setField(event.currentTarget.value)} onAction={submit} />
          </Field>
        </Show>
        <Field label={at()!.field ? "New value" : "Value"} hint={hidden() ? "It's encrypted on this device, and never shown again." : undefined}>
          <Input type={hidden() ? "password" : "text"} autocomplete="off" autocapitalize="none" autocorrect={false} spellcheck={false} required autofocus={!!at()!.item} value={value()} onInput={(event) => setValue(event.currentTarget.value)} onAction={submit} />
        </Field>
        <Show when={!at()!.field}>
          <Checkbox label="Secret" checked={secret()} onChange={(event) => setSecret(event.currentTarget.checked)} />
        </Show>
        <Show when={!at()!.item}>
          <Field label="Websites" hint="Where its secrets may be sent, like api.github.com.">
            <Input type="url" autocapitalize="none" autocorrect={false} spellcheck={false} value={hosts()} onInput={(event) => setHosts(event.currentTarget.value)} onAction={submit} />
          </Field>
        </Show>
        <Button disabled={!ready()} onClick={save}>
          Save
        </Button>
        <Show when={at()!.field}>
          <Button variant="ghost" onClick={remove}>
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
      <List>
        <For each={types()} keyed={(type) => type.id}>
          {(type) => <Row start={<Image src={type().icon ?? ""} alt="" style={tile} />} heading={titled(type().name)} onClick={() => (shut(), create(type()))} />}
        </For>
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
        <Input required autofocus value={name()} onInput={(event) => setName(event.currentTarget.value)} onAction={() => name().trim() && make()} />
      </Field>
      <Button loading={busy()} disabled={!name().trim()} onClick={make}>
        Add
      </Button>
    </Sheet>
  )
}

/// Who asks, as a person reads it: the app's title, the agent's name, or an agent.
const asker = (request: Request) => request.app?.title ?? request.by ?? "An agent"
/// What it asks for: an account where it wants a name with its secret, else a key.
const wanted = (request: Request) => (request.fields.some((field) => field.secret === false) ? "an account" : "a key")

/// The requests waiting, each opening its alert again, as iOS keeps a place to answer a prompt later.
function Requests() {
  return (
    <Screen heading="Requests" back>
      <Show when={waiting().length} fallback={<Empty>Nothing's waiting.</Empty>}>
        <List>
          <For each={waiting()}>{(request) => <Row heading={`${asker(request)} wants ${request.title}`} text={request.why ?? undefined} onClick={() => setAsking(request)} />}</For>
        </List>
      </Show>
    </Screen>
  )
}

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
  const options = createMemo(() => createListCollection({ items: [...fits().map((item) => ({ value: item.name, label: item.name })), { value: "", label: "New Item" }] }))
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
          <Button variant="outline" disabled={busy()} onClick={() => answer({ decline: true })}>
            Don't Allow
          </Button>
          <Button loading={busy()} disabled={!request() || !filled()} onClick={allow}>
            Allow
          </Button>
        </>
      }
    >
      <Show when={request()}>
        <Field label={request()!.title}>
          <Select.Root collection={options()} value={[item()]} onValueChange={({ value }) => setChosen(value[0] ?? "")} />
        </Field>
        <Show when={!item()}>
          <For each={request()!.fields}>
            {(field, at) => (
              <Field label={label(field.name)}>
                <Input
                  type={field.secret === false ? "text" : "password"}
                  required
                  autofocus={at() === 0}
                  value={typed()[field.name] ?? ""}
                  onInput={(event) => setTyped((all) => ({ ...all, [field.name]: event.currentTarget.value }))}
                  onAction={() => filled() && !busy() && allow()}
                />
              </Field>
            )}
          </For>
        </Show>
        <Show when={problem()}>
          <Text role="alert" style={{ color: "destructive" }}>
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
  createEffect(() => own() && security().holds, (holds) => void (holds && encryptForPasskeys()))
  const choose = (server: string, on: boolean) => {
    const servers = security().servers.filter((chosen) => chosen !== server)
    return does("vault_grant", { servers: on ? [...servers, server] : servers })
  }
  return (
    <Screen heading="Security" back>
      <Readiness />
      <Show when={security().readiness !== "not_set_up"}>
        <Section heading="Trusted Devices">
          <List>
            <For each={security().asking}>
              {(device) => (
                <Row
                  heading={device.name}
                  text="Wants to be trusted"
                  action={
                    security().holds ? (
                      <Button variant="secondary" onClick={() => does("encryption_approve", { device: device.device, name: device.name })}>
                        Approve
                      </Button>
                    ) : undefined
                  }
                />
              )}
            </For>
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
                  <Row heading="Unlock with Passkey" text="Use a passkey that unlocks your key" onClick={unlockWithPasskey} />
                </Show>
                <Row heading="Ask Another Device" text="Approve this one on a trusted device" onClick={askAnother} />
                <Row heading="Use Recovery Key" text="Type the key you wrote down" onClick={() => useRecoveryKey({ subject: whose() }).then(again)} />
              </List>
            </Section>
          }
        >
          <Section heading="Recovery Key">
            <List>
              <Row heading="Show Recovery Key" text="For when every device is gone" onClick={() => showRecoveryKey({ subject: whose() })} />
            </List>
          </Section>
        </Show>
        <Section heading="Servers">
          <Text style={{ color: "muted-foreground" }}>{`A server that can open ${whom()} Vault runs what uses its secrets.${security().holds ? "" : " Choosing them takes a trusted device."}`}</Text>
          <List>
            <For each={security().may}>
              {(server) => (
                <Row
                  heading={server.name ?? "A server"}
                  text={security().servers.includes(server.key) ? `Opens ${whom()} Vault` : undefined}
                  end={<Switch aria-label={server.name ?? "A server"} checked={security().servers.includes(server.key)} disabled={!security().holds} onChange={(event) => choose(server.key, event.currentTarget.checked)} />}
                />
              )}
            </For>
          </List>
        </Section>
      </Show>
    </Screen>
  )
}

/// A dot that says something needs its person.
const dot = { width: 8, height: 8, flexShrink: 0, borderRadius: "full", backgroundColor: "warning" } as const

/// What its app says an item is in, in a row's grey line or atop its screen: a spinner and what's under way, or a
/// dot and why. Ready is nothing: fine is silent.
function Line(props: { status: Status }) {
  return (
    <Box style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
      <Show when={props.status.state === "busy"} fallback={<Box aria-hidden="true" style={dot} />}>
        <Spinner />
      </Show>
      <Text style={{ flexShrink: 1, minWidth: 0 }}>{props.status.message}</Text>
    </Box>
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
      <Button variant="outline" loading={busy()} onClick={run}>
        {action()!.label}
      </Button>
    </Show>
  )
}
/// Whether an item has an action that fixes it.
const fixable = (item: Item) => item.status?.state === "needs_action" && !!item.status.action

function Vault() {
  // A link to a request, `one:vault#request=<id>`, as a request's own link (`/vault/request?id=`) leads to: its alert,
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
  // The sidebar, then what it picked: on a phone one at a time, the sidebar first, as Passwords on iPhone; on a
  // computer side by side, what it picked splitting the rest into the list and the item where there's room (Items'
  // Split), so three columns, or two where it's narrow.
  return (
    <>
      <Split
        open={entered()}
        onClose={() => setEntered(false)}
        detail={
          <Choose>
            <Match when={place() === "security"}>
              <SecurityScreen />
            </Match>
            <Match when={place() === "requests"}>
              <Requests />
            </Match>
            <Match when={place()}>
              <Show when={place()} keyed>
                {(where) => <Items place={where} />}
              </Show>
            </Match>
          </Choose>
        }
      >
        <Sidebar />
      </Split>
      <FieldSheet />
      <CreateSheet />
      <AddSheet />
      <RequestAlert />
    </>
  )
}

render(() => (
  <Errored fallback={(error) => <Problem heading="Can't show your Vault" text={error() as Error} />}>
    <Loading fallback={<Skeleton rows={4} />}>
      <Vault />
    </Loading>
  </Errored>
))
