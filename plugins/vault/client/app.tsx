// The Vault: everything someone has given access to, or a group or the network has, for whoever owns them. What's
// been asked for and waits for an answer; their passwords and keys, each opening beside the list with its fields (a
// secret one is typed and never shown again), the sites its secrets go to, and what may use it; then a section for each
// kind of item a plugin keeps here (`vault.types`): Accounts, Chrome Profiles. An item shows the state its plugin says
// it's in: nothing when it's ready, a spinner and what's under way, or a dot, why and the one action that fixes it. One
// of a type with a component is drawn beside the list by its own plugin. Deleting one is soft delete's. Below, the
// servers that open the Vault, and which devices hold its person's key, since only one of those chooses the servers.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { Dynamic, render } from "@solidjs/web"
import "./app.css"
import { Button, call, changed, Checkbox, confirm, createListCollection, Empty, Field, Input, leave, List, open as opening, Problem, RadioGroup, Row, Screen, Section, Select, Sheet, showRecoveryKey, Skeleton, Spinner, Split, Switch, toast, useRecoveryKey } from "ui"

/// Whose Vault this is: a person's, a group's or the network's. Left unchosen, it's whoever the app was opened for.
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
  type: string | null
  state: State | null
  fields: ItemField[]
  hosts: string[]
  plugins: string[]
  agents: boolean
  sign_in?: boolean
}

/** What a plugin says its item needs of its person, as it says it, or what adds one of a type: a tool of the
 * plugin's, called as its person, or a page of its client's. */
interface Action {
  label?: string
  tool?: string
  inputs?: Record<string, unknown>
  page?: string
  name?: boolean
}

/** Which of the three states an item is in, other than ready, which is none: busy with a message, or
 * action_required with why and an action. `by` is the plugin that said it. */
interface State {
  state: "busy" | "action_required"
  message: string
  action?: Action
  by: string
}

/** A kind of item a plugin keeps here, as `vault_list` answers it. */
interface ItemType {
  id: string
  plugin: string
  title: string | null
  icon: string | null
  trusted: boolean
  name: string
  prefix: string
  section: string
  component: string | null
  create: Action | null
}

/** What a plugin or an agent asked for, waiting for an answer. */
interface Request {
  id: string
  title: string
  plugin?: string
  why?: string
  hosts: string[]
  fields: { name: string; secret?: boolean }[]
}

/** A device, as `encryption_key` names it. */
interface Device {
  device: string
  name: string
}

/** The sheet that's open over the page: a field to type, a request to answer, a new item of a type to name, or the
 * plugins an account is added to. */
type Sheet = { kind: "field"; item?: Item; field?: ItemField } | { kind: "request"; request: Request } | { kind: "create"; type: ItemType } | { kind: "add"; section: string }

const [whose, setWhose] = createSignal<string>()
const ask = <T,>(tool: string, inputs?: object): Promise<T> => call(tool, inputs, { subject: whose() }).then(JSON.parse)

/// Which servers open the Vault and which may, what this device holds of its owner's key, and the items.
const opened = createMemo(() => ask<{ servers: string[]; may: { key: string; name?: string }[] }>("vault_unlocked_on"))
const key = createMemo(() => ask<{ owner: string; slot?: unknown; check?: unknown; device: string; devices: Device[]; asking: Device[] }>("encryption_key"))
const contents = createMemo(() => (opened().servers.length ? ask<{ items: Item[]; requests: Request[]; plugins: string[]; types: ItemType[] }>("vault_list") : { items: [], requests: [], plugins: [], types: [] }))
const again = () => [opened, key, contents].forEach(refresh)
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
/// Whose the Vault is, in a sentence: "your", "The Smiths's", "the network's".
const whom = () => owners().find((owner) => owner.value === key().owner)?.whose ?? "your"
const Whom = () => whom()[0].toUpperCase() + whom().slice(1)

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
/// A few things in a sentence: "fake, gmail and your agents".
const listed = (things: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(things)
const sites = (typed: unknown) => String(typed ?? "").split(/[\s,]+/).filter(Boolean)
/// What may use an item, in its grey line.
const users = (item: Item) => [...item.plugins, ...(item.agents ? ["your agents"] : [])]
/// An item's type, if it's of one, and its name as a person reads it: "Personal", not "Chrome profile: Personal".
const typeOf = (item: Item) => contents().types.find((type) => type.id === item.type)
const shown = (item: Item) => item.name.slice(typeOf(item)?.prefix.length ?? 0)
/// Each section a type is listed under, by its heading, with the types in it.
const sections = () => [...Map.groupBy(contents().types, (type) => type.section)].sort(([a], [b]) => a.localeCompare(b))
/// Does an action a plugin gave, as its person: a tool of the plugin's, whose `url`, if it gives one, opens in the
/// device's browser, or a page of its client's. → whether it did.
async function act(action: Action, plugin: string, inputs: Record<string, unknown> = {}) {
  try {
    if (action.page) await opening(`${plugin}/${action.page}`, { subject: whose() })
    else {
      const done = await ask<{ url?: string }>(`${plugin}_${action.tool}`, { ...action.inputs, ...inputs })
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
/// A component a plugin exports (client/package.json), beside the Vault's page, where a client serves each plugin's.
const component = (type: ItemType) => import(/* @vite-ignore */ new URL(`../../${type.plugin}/client/${type.component}.js`, document.baseURI).href)

/// The item that's open beside the list, by its name, and the sheet that's open over the page.
const [open, setOpen] = createSignal<string>()
const [sheet, setSheet] = createSignal<Sheet>()
const shut = () => setSheet()
const holds = () => !!key().slot

/// A field of an item to type: a new item's first, a new one of an item's, or one that's there to change.
function FieldSheet() {
  const at = () => { const open = sheet(); return open?.kind === "field" ? open : undefined }
  const [secret, setSecret] = createSignal(true)
  const save = async (event: SubmitEvent) => {
    event.preventDefault()
    const typed = Object.fromEntries(new FormData(event.target as HTMLFormElement)) as Record<string, string>
    const item = at()!.item?.name ?? typed.item
    const inputs: Record<string, unknown> = { item, field: at()!.field?.name ?? typed.field, value: typed.value, secret: at()!.field ? at()!.field!.secret : secret() }
    if (!at()!.item) inputs.hosts = sites(typed.hosts)
    if ((await does("vault_set", inputs)) && !at()!.item) setOpen(item)
    shut()
  }
  const remove = async () => {
    const { item, field } = at() as { item: Item; field: ItemField }
    if (!(await confirm({ heading: `Delete ${label(field.name)}?`, text: "It can't be brought back.", action: "Delete", destructive: true }))) return
    await does("vault_remove", { item: item.name, field: field.name })
    shut()
  }
  return (
    <Sheet heading={at()?.field ? label(at()!.field!.name) : at()?.item ? "Add Field" : "Add Item"} open={!!at()} onClose={shut}>
      <Show when={at()}>
        <form onSubmit={save}>
          <Show when={!at()!.item}>
            <Field label="Name">
              <Input name="item" placeholder="GitHub" required autofocus />
            </Field>
          </Show>
          <Show when={!at()!.field}>
            <Field label="Field">
              <Input name="field" placeholder="token" pattern="[A-Za-z0-9_]+" autocapitalize="none" spellcheck={false} required />
            </Field>
          </Show>
          <Field label={at()!.field ? "New value" : "Value"} hint={(at()!.field ? at()!.field!.secret : secret()) ? "It's encrypted on this device, and never shown again." : undefined}>
            <Input name="value" type={(at()!.field ? at()!.field!.secret : secret()) ? "password" : "text"} value={at()!.field?.value ?? ""} autocomplete="off" spellcheck={false} required autofocus={!!at()!.item} />
          </Field>
          <Show when={!at()!.field}>
            <Checkbox label="Secret" checked={secret()} onChange={(event) => setSecret(event.currentTarget.checked)} />
          </Show>
          <Show when={!at()!.item}>
            <Field label="Sites" hint="Where its secrets may be sent, like api.github.com.">
              <Input name="hosts" autocapitalize="none" spellcheck={false} />
            </Field>
          </Show>
          <Button type="submit">Save</Button>
          <Show when={at()!.field}>
            <Button variant="ghost" onClick={remove}>
              Delete Field
            </Button>
          </Show>
        </form>
      </Show>
    </Sheet>
  )
}

/// A request for an item: who asks, what they'd get and where it goes, then one of this person's items that has
/// what it needs, or new values to type. Allow is the only way to say yes.
function RequestSheet() {
  const request = () => { const open = sheet(); return open?.kind === "request" ? open.request : undefined }
  const [picked, setPicked] = createSignal("")
  const fits = () => contents().items.filter((item) => request()!.fields.every((wanted) => item.fields.some((field) => field.name === wanted.name)))
  const who = () => request()!.plugin ?? "Your agents"
  const allow = async (event: SubmitEvent) => {
    event.preventDefault()
    const typed = Object.fromEntries(new FormData(event.target as HTMLFormElement))
    const response_inputs = picked() ? { item: picked() } : { values: JSON.stringify(Object.fromEntries(request()!.fields.map(({ name }) => [name, typed[name]]))) }
    if (await does("vault_answer", { id: request()!.id, ...response_inputs })) shut()
  }
  const decline = async () => (await does("vault_answer", { id: request()!.id, decline: true })) && shut()
  return (
    <Sheet heading={request() ? `${who()} ${request()!.plugin ? "needs" : "need"} ${request()!.title}` : ""} open={!!request()} onClose={shut}>
      <Show when={request()}>
        <form onSubmit={allow}>
          <p class="about">
            {request()!.why ? `${request()!.why!.trim().replace(/[^.!?]$/, "$&.")} ` : ""}
            {who()} can use {picked() || request()!.title}
            {!picked() && request()!.hosts.length ? `, sent only to ${listed(request()!.hosts)}` : ""}. Nobody is shown what you type.
          </p>
          <Show when={fits().length}>
            <Field label="Use">
              <RadioGroup name="picked" value={picked()} onChange={setPicked} options={[...fits().map((item) => ({ value: item.name, label: item.name })), { value: "", label: "New Item" }]} />
            </Field>
          </Show>
          <Show when={!picked()}>
            <For each={request()!.fields}>
              {(field, at) => (
                <Field label={label(field.name)}>
                  <Input name={field.name} type={field.secret ? "password" : "text"} autocomplete={field.secret ? "new-password" : "off"} spellcheck={false} required autofocus={at() === 0} />
                </Field>
              )}
            </For>
          </Show>
          <Button type="submit">Allow</Button>
          <Button variant="outline" onClick={decline}>
            Decline
          </Button>
        </form>
      </Show>
    </Sheet>
  )
}

/// One item: what it holds, where its secrets go, and what may use it, each switched on here.
function ItemScreen(props: { item: Item }) {
  const item = () => props.item
  // Deleting it is soft delete's, as for anything an app made: it's in Recently Deleted for 30 days.
  const remove = async () => {
    if (!(await confirm({ heading: `Delete ${shown(item())}?`, text: "Whatever uses it stops working. It's in Recently Deleted for 30 days.", action: "Delete", destructive: true }))) return
    if (await does("recently-deleted_remove", { artifact: item().artifact })) setOpen()
  }
  return (
    <Screen heading={shown(item())} back>
      <Show when={item().state}>
        {(state) => <Line state={state()} />}
      </Show>
      <Section heading={item().sign_in ? "Signed in" : "Fields"}>
        <List>
          <For each={item().fields}>{(field) => <Row heading={label(field.name)} text={field.secret ? "Secret" : field.value} onClick={() => setSheet({ kind: "field", item: item(), field })} />}</For>
        </List>
        <Button variant="secondary" onClick={() => setSheet({ kind: "field", item: item() })}>
          Add Field
        </Button>
      </Section>
      <Section heading="Sent only to">
        <Show when={item().hosts.length} fallback={<Empty>Nowhere: it names no site.</Empty>}>
          <List>
            <For each={item().hosts}>{(host) => <Row heading={host} />}</For>
          </List>
        </Show>
      </Section>
      <Section heading="Used by">
        <List>
          <Row
            heading="Your agents"
            text="Use it, never see it"
            end={<Switch aria-label="Your agents" checked={item().agents} onChange={(event) => does("vault_agents", { item: item().name, on: event.currentTarget.checked })} />}
          />
          <For each={contents().plugins}>
            {(plugin) => (
              <Row
                heading={plugin}
                text="Its code is handed it"
                end={<Switch aria-label={plugin} checked={item().plugins.includes(plugin)} onChange={(event) => does("vault_use", { item: item().name, plugin, on: event.currentTarget.checked })} />}
              />
            )}
          </For>
        </List>
      </Section>
      <Section>
        <Button variant="destructive" onClick={remove}>
          Delete Item
        </Button>
      </Section>
    </Screen>
  )
}

/// The servers that open the Vault, each switched on by a device that holds its person's key.
function OpenedBy() {
  const choose = (server: string, on: boolean) => {
    const servers = opened().servers.filter((chosen) => chosen !== server)
    return does("vault_grant", { servers: on ? [...servers, server] : servers })
  }
  return (
    <Section heading="Unlocked On">
      <List>
        <For each={opened().may}>
          {(server) => (
            <Row
              heading={server.name ?? "A server"}
              text={`Holds ${whom()} Vault's key`}
              end={<Switch aria-label={server.name ?? "A server"} checked={opened().servers.includes(server.key)} disabled={!holds()} onChange={(event) => choose(server.key, event.currentTarget.checked)} />}
            />
          )}
        </For>
      </List>
    </Section>
  )
}

/// The devices that hold this person's key, the ones asking to, and what this device does about being neither.
function YourKey() {
  const others = () => key().devices.filter((device) => device.device !== key().device)
  return (
    <Section heading={`${Whom()} key`}>
      <Show
        when={holds()}
        fallback={
          <Problem
            warn
            heading={`This device doesn't hold ${whom()} key`}
            text={`It adds and uses items. Choosing the servers that open ${whom()} Vault takes a trusted device.`}
            action={
              <>
                <Button variant="outline" onClick={() => does("encryption_ask", {}).then((requested) => void (requested && toast("Asked. Open Vault on a trusted device.")))}>
                  Ask Another Device
                </Button>
                <Button variant="outline" onClick={() => useRecoveryKey({ subject: whose() }).then(again)}>
                  Use Recovery Key
                </Button>
              </>
            }
          />
        }
      >
        <List>
          <For each={key().asking}>
            {(device) => (
              <Row
                heading={device.name}
                text="Wants to be trusted"
                end={
                  <Button variant="secondary" onClick={() => does("encryption_approve", { device: device.device, name: device.name })}>
                    Approve
                  </Button>
                }
              />
            )}
          </For>
          <Row heading="This device" text="Trusted device" />
          <For each={others()}>{(device) => <Row heading={device.name} text="Trusted device" />}</For>
          <Row heading="Recovery Key" text="For when every device is gone" onClick={() => showRecoveryKey({ subject: whose() })} />
        </List>
      </Show>
    </Section>
  )
}

/// Making someone's key is the first thing, on the device they open the Vault on first: theirs, or a group's or
/// the network's, by one of its owners. It shows them its recovery key then, which is the one time they're asked
/// to write something down.
async function setUp() {
  if (await does("encryption_approve", {})) await showRecoveryKey({ subject: whose() })
}

/// Which Vault shows, where its person owns more than their own.
function Whose() {
  const collection = createMemo(() => createListCollection({ items: owners() }))
  const pick = ({ value }: { value: string[] }) => {
    setOpen()
    setSheet()
    setWhose(value[0])
  }
  return (
    <Show when={owners().length > 1}>
      <Field label="Owner">
        <Select.Root collection={collection()} value={[key().owner]} onValueChange={pick} />
      </Field>
    </Show>
  )
}

/// What its plugin says an item is in, in a row's grey line or atop its screen: a spinner and what's under way, or a
/// dot, why and the action that fixes it. Ready is nothing: fine is silent.
function Line(props: { state: State }) {
  const [busy, setBusy] = createSignal(false)
  const run = async () => {
    setBusy(true)
    await act(props.state.action!, props.state.by)
    setBusy(false)
  }
  return (
    <span class="state">
      <Show when={props.state.state === "busy"} fallback={<span class="dot" aria-hidden="true" />}>
        <Spinner />
      </Show>
      <span class="message">{props.state.message}</span>
      <Show when={props.state.state === "action_required" && props.state.action}>
        <Button variant="outline" loading={busy()} onClick={(event: MouseEvent) => (event.stopPropagation(), run())}>
          {props.state.action!.label}
        </Button>
      </Show>
    </span>
  )
}

/// A section of a kind of item: its items, each with its plugin's icon and its state, and a way to add one where
/// its types make one.
function Kind(props: { section: string; types: ItemType[] }) {
  const items = () => contents().items.filter((item) => props.types.some((type) => type.id === item.type))
  const creatable = () => props.types.filter((type) => type.create && type.trusted)
  // "Add Chrome Profile" for one kind, or "Add Account" for the section's several.
  const adds = () => `Add ${titled(creatable().length === 1 ? creatable()[0].name : props.section.replace(/s$/, ""))}`
  const add = () => (creatable().length === 1 ? create(creatable()[0]) : setSheet({ kind: "add", section: props.section }))
  return (
    <Section heading={props.section}>
      <Show when={items().length} fallback={<Empty>None yet.</Empty>}>
        <List>
          <For each={items()} keyed={(item) => item.artifact}>
            {(item) => (
              <Row
                start={<img class="logo" src={typeOf(item())?.icon ?? ""} alt="" />}
                heading={shown(item())}
                text={item().state ? <Line state={item().state!} /> : <Summary type={typeOf(item())!} item={item()} />}
                current={item().name === open()}
                onClick={() => choose(item())}
              />
            )}
          </For>
        </List>
      </Show>
      <Show when={creatable().length}>
        <div class="more">
          <Button variant="secondary" onClick={add}>{adds()}</Button>
        </div>
      </Show>
    </Section>
  )
}

/// Adds an item of a type, as the type says: asked for a name first, where it needs one.
const create = (type: ItemType) => (type.create!.name ? setSheet({ kind: "create", type }) : act(type.create!, type.plugin))

/// The kinds of item a section's several types add, each adding one.
function AddSheet() {
  const section = () => { const open = sheet(); return open?.kind === "add" ? open.section : undefined }
  const types = () => contents().types.filter((type) => type.section === section() && type.create && type.trusted)
  return (
    <Sheet heading={section() ? `Add ${titled(section()!.replace(/s$/, ""))}` : ""} open={!!section()} onClose={shut}>
      <List>
        <For each={types()} keyed={(type) => type.id}>
          {(type) => <Row start={<img class="logo" src={type().icon ?? ""} alt="" />} heading={titled(type().name)} onClick={() => (shut(), create(type()))} />}
        </For>
      </List>
    </Sheet>
  )
}

/// A new item of a type: its name, which the type's plugin makes it from (`create`), and then it opens.
function CreateSheet() {
  const type = () => { const open = sheet(); return open?.kind === "create" ? open.type : undefined }
  const [busy, setBusy] = createSignal(false)
  const create = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    const name = String(new FormData(event.currentTarget).get("name")).trim()
    const made = type()!
    setBusy(true)
    const done = await act(made.create!, made.plugin, { name })
    setBusy(false)
    if (done) {
      shut()
      setOpen(made.prefix + name)
    }
  }
  return (
    <Sheet heading={type() ? `Add ${titled(type()!.name)}` : ""} open={!!type()} onClose={shut}>
      <form onSubmit={create}>
        <Field label="Name">
          <Input name="name" required autofocus />
        </Field>
        <Button type="submit" loading={busy()}>Add</Button>
      </form>
    </Sheet>
  )
}

/// An item of a type with a component, drawn by its plugin's own: in this page for a trusted plugin, as one of the
/// Vault's own screens, with what the Vault's page may do.
function Drawn(props: { type: ItemType; item: Item }) {
  const drawn = createMemo(() => component(props.type).then((module) => module.default))
  return <Dynamic component={drawn()} item={shown(props.item)} subject={whose()} open={(name: string) => setOpen(props.type.prefix + name)} close={() => setOpen()} />
}

/// What's open beside the list: an item of a type with a component, drawn by its trusted plugin's, or else its fields.
function Detail(props: { item: Item }) {
  const drawn = () => { const type = typeOf(props.item); return type?.component && type.trusted ? type : undefined }
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

/// An item's grey line in a type's section when it's ready: what its plugin's component says of it, else whose it is.
function Summary(props: { type: ItemType; item: Item }) {
  const summary = createMemo(() => (props.type.trusted && props.type.component ? component(props.type).then((module) => module.Summary) : undefined))
  const plugin = () => props.type.title ?? props.type.name
  return (
    <Errored fallback={plugin()}>
      <Loading fallback={plugin()}>
        <Show when={summary()} fallback={plugin()}>
          {(drawn) => <Dynamic component={drawn()} item={shown(props.item)} subject={whose()} />}
        </Show>
      </Loading>
    </Errored>
  )
}

/// An item of a type with a component opens beside the list, but one of an untrusted plugin's opens in a frame of its
/// own, its page for it (client/<component>.html), where it keeps what an app of its own may do.
function choose(item: Item) {
  const type = typeOf(item)
  if (type?.component && !type.trusted) return opening(`${type.plugin}/client/${type.component}.html#${encodeURIComponent(shown(item))}`, { subject: whose() })
  setOpen(item.name)
}

function Vault() {
  const item = () => contents().items.find(({ name }) => name === open())
  const plain = () => contents().items.filter((item) => !item.type)
  /// A section for each kind of item: those with any here, or that can be added.
  const kinds = () => sections().filter(([, types]) => types.some((type) => (type.create && type.trusted) || contents().items.some((item) => item.type === type.id)))
  return (
    <>
      <Split open={!!item()} onClose={() => setOpen()} detail={<Show when={item()} fallback={<Show when={contents().items.length}><Empty>Pick an item to see what it holds.</Empty></Show>}>{(item) => <Detail item={item()} />}</Show>}>
        <Screen
        heading="Vault"
        back
        onBack={leave}
        action={
          <Show when={opened().servers.length}>
            <Button onClick={() => setSheet({ kind: "field" })}>Add</Button>
          </Show>
        }
      >
        <Whose />
        <Show when={key().check} fallback={<Empty action={<Button onClick={setUp}>Set Up</Button>}>Keys and sign-ins are kept here, encrypted. Set up {whom()} key first.</Empty>}>
          <Show when={!opened().servers.length}>
            <Problem warn heading={`No server opens ${whom()} Vault`} text={holds() ? "Choose one below. Until then it keeps nothing." : "Choose one from a trusted device. Until then it keeps nothing."} />
          </Show>
          <Show when={contents().requests.length}>
            <Section heading="Asked for">
              <List>
                <For each={contents().requests}>
                  {(request) => <Row heading={request.title} text={[`For ${request.plugin ?? "your agents"}`, request.why].filter(Boolean).join(" · ")} onClick={() => setSheet({ kind: "request", request })} />}
                </For>
              </List>
            </Section>
          </Show>
          <Show when={opened().servers.length}>
            <Section heading="Passwords and keys">
              <Show when={plain().length} fallback={<Empty>None yet.</Empty>}>
                <List>
                  <For each={plain()} keyed={(item) => item.artifact}>
                    {(item) => (
                      <Row
                        heading={item().name}
                        text={item().state ? <Line state={item().state!} /> : users(item()).length ? `Used by ${listed(users(item()))}` : "Nothing uses it"}
                        current={item().name === open()}
                        onClick={() => setOpen(item().name)}
                      />
                    )}
                  </For>
                </List>
              </Show>
            </Section>
            <For each={kinds()} keyed={([section]) => section}>
              {(kind) => <Kind section={kind()[0]} types={kind()[1]} />}
            </For>
          </Show>
          <OpenedBy />
          <YourKey />
        </Show>
        </Screen>
      </Split>
      <FieldSheet />
      <RequestSheet />
      <CreateSheet />
      <AddSheet />
    </>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your Vault" text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={4} />}>
        <Vault />
      </Loading>
    </Errored>
  ),
  document.body,
)
