// The Vault: everything someone has given access to, or a group or the network has, for whoever owns them. What's
// been asked for and waits for an answer; their passwords and keys, each opening beside the list with its fields (a
// secret one is typed and never shown again), the sites its secrets go to, and what may use it; their accounts, each
// plugin's as its `accounts` lists them, with Sign In and Sign Out; and a section for each other kind of item a plugin
// keeps here (`vault.types`), one of which its plugin's own component draws beside the list. Below, the servers that
// open the Vault, and which devices hold its person's key, since only one of those chooses the servers.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { Dynamic, render } from "@solidjs/web"
import "./app.css"
import { Button, call, changed, Checkbox, confirm, createListCollection, Empty, Field, Icon, Input, leave, List, Menu, open as opening, Problem, RadioGroup, Row, Screen, Section, Select, Sheet, showRecoveryKey, Skeleton, Split, Switch, toast, useRecoveryKey } from "ui"

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
  type: string | null
  fields: ItemField[]
  hosts: string[]
  plugins: string[]
  agents: boolean
  sign_in?: boolean
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
  account: boolean
  plural: string | null
  component: string | null
  create: string | null
  sign_in: string | null
}

/** An outside account, as a plugin's `accounts` lists it. */
interface Account {
  account: string
  ok: boolean
  why?: string
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
type Sheet = { kind: "field"; item?: Item; field?: ItemField } | { kind: "request"; request: Request } | { kind: "create"; type: ItemType } | { kind: "accounts" }

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
/// The plugins with accounts, one each, by their names as a person reads them.
const withAccounts = () => [...new Map(contents().types.filter((type) => type.account).map((type) => [type.plugin, type])).values()].sort((a, b) => (a.title ?? a.plugin).localeCompare(b.title ?? b.plugin))
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
  const remove = async () => {
    if (!(await confirm({ heading: `Delete ${item().name}?`, text: "Whatever uses it stops working, and it can't be brought back.", action: "Delete", destructive: true }))) return
    if (await does("vault_remove", { item: item().name })) setOpen()
  }
  return (
    <Screen heading={item().name} back>
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

/// Signs an account in to a plugin, or with `account`, signs that one in again: on the plugin's own page for it, or at
/// the web address its `sign_in` answers, in the device's browser.
const signIn = (type: ItemType, account?: string) =>
  (type.sign_in
    ? opening(account ? `${type.sign_in}#${encodeURIComponent(account)}` : type.sign_in, { subject: whose() })
    : ask<{ url: string }>(`${type.plugin}_sign_in`, account ? { account } : {}).then(({ url }) => opening(url))
  ).catch(toast)

/// A plugin's accounts, once its `accounts` answers: each with Sign Out, and Sign In where it needs signing in again.
function AccountsOf(props: { type: ItemType }) {
  const accounts = createMemo(() => ask<{ accounts: Account[] }>(`${props.type.plugin}_accounts`).then((listed) => listed.accounts))
  changed(() => refresh(accounts))
  const title = () => props.type.title ?? props.type.plugin
  const signOut = async (account: string) => {
    const sure = await confirm({ heading: `Sign out of ${account}?`, text: `${title()} can't use it until it's signed in again.`, action: "Sign Out", destructive: true })
    if (sure) await call(`${props.type.plugin}_sign_out`, { account }, { subject: whose() }).catch(toast)
    refresh(accounts)
  }
  return (
    <For each={accounts()} keyed={(account) => account.account}>
      {(account) => (
        <Row
          start={<img class="logo" src={props.type.icon ?? ""} alt="" />}
          heading={account().account}
          text={account().ok ? title() : <span class="warn"><Icon name="warning-circle" />{account().why ?? "Sign in again"}</span>}
          end={
            <span class="actions">
              <Show when={!account().ok}>
                <Button variant="outline" onClick={() => signIn(props.type, account().account)}>Sign In</Button>
              </Show>
              <Button variant="ghost" onClick={() => signOut(account().account)}>Sign Out</Button>
            </span>
          }
        />
      )}
    </For>
  )
}

/// Every plugin's accounts, in one list, and Add Account.
function Accounts() {
  const add = () => (withAccounts().length === 1 ? signIn(withAccounts()[0]) : setSheet({ kind: "accounts" }))
  return (
    <Section heading="Accounts">
      <List>
        <For each={withAccounts()} keyed={(type) => type.plugin}>
          {(type) => (
            <Errored fallback={(error) => <Row start={<img class="logo" src={type().icon ?? ""} alt="" />} heading={type().title ?? type().plugin} text={<span class="warn"><Icon name="warning-circle" />{`Can't list its accounts: ${(error() as Error).message ?? error()}`}</span>} />}>
              <Loading fallback={null}>
                <AccountsOf type={type()} />
              </Loading>
            </Errored>
          )}
        </For>
      </List>
      <div class="more">
        <Button variant="secondary" onClick={add}>Add Account</Button>
      </div>
    </Section>
  )
}

/// The plugins an account is added to, each signing one in.
function AccountsSheet() {
  return (
    <Sheet heading="Add Account" open={sheet()?.kind === "accounts"} onClose={shut}>
      <List>
        <For each={withAccounts()} keyed={(type) => type.plugin}>
          {(type) => <Row start={<img class="logo" src={type().icon ?? ""} alt="" />} heading={type().title ?? type().plugin} onClick={() => (shut(), signIn(type()))} />}
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
    const done = await call(`${made.plugin}_${made.create}`, { name }, { subject: whose() }).then(() => true, (error) => (toast(error), false))
    setBusy(false)
    if (done) {
      shut()
      again()
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

/// An item's grey line in a type's section: what its plugin's component says of it, else what it is.
function Summary(props: { type: ItemType; item: Item }) {
  const summary = createMemo(() => (props.type.trusted && props.type.component ? component(props.type).then((module) => module.Summary) : undefined))
  return (
    <Errored fallback={props.type.name}>
      <Loading fallback={props.type.name}>
        <Show when={summary()} fallback={props.type.name}>
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
  /// Each other kind of item with any here, in its own section, by the name of the plugin that keeps it.
  const kinds = () => contents().types.filter((type) => !type.account && contents().items.some((item) => item.type === type.id))
  const creatable = () => contents().types.filter((type) => type.create && type.trusted)
  const add = ({ value }: { value: string }) => {
    const type = contents().types.find(({ id }) => id === value)
    setSheet(type ? { kind: "create", type } : { kind: "field" })
  }
  return (
    <>
      <Split open={!!item()} onClose={() => setOpen()} detail={<Show when={item()} fallback={<Show when={contents().items.length}><Empty>Pick an item to see what it holds.</Empty></Show>}>{(item) => <Detail item={item()} />}</Show>}>
        <Screen
        heading="Vault"
        back
        onBack={leave}
        action={
          <Show when={opened().servers.length}>
            <Show when={creatable().length} fallback={<Button onClick={() => setSheet({ kind: "field" })}>Add</Button>}>
              <Menu.Root onSelect={add}>
                <Menu.Trigger variant="default">Add</Menu.Trigger>
                <Menu.Positioner>
                  <Menu.Content>
                    <Menu.Item value="item">Password or Key</Menu.Item>
                    <For each={creatable()}>{(type) => <Menu.Item value={type.id}>{titled(type.name)}</Menu.Item>}</For>
                  </Menu.Content>
                </Menu.Positioner>
              </Menu.Root>
            </Show>
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
                  <For each={plain()}>
                    {(item) => (
                      <Row
                        heading={item.name}
                        text={users(item).length ? `Used by ${listed(users(item))}` : "Nothing uses it"}
                        current={item.name === open()}
                        onClick={() => setOpen(item.name)}
                      />
                    )}
                  </For>
                </List>
              </Show>
            </Section>
            <Show when={withAccounts().length}>
              <Accounts />
            </Show>
            <For each={kinds()} keyed={(type) => type.id}>
              {(type) => (
                <Section heading={type().plural}>
                  <List>
                    <For each={contents().items.filter((item) => item.type === type().id)}>
                      {(item) => (
                        <Row
                          start={<img class="logo" src={type().icon ?? ""} alt="" />}
                          heading={shown(item)}
                          text={<Summary type={type()} item={item} />}
                          current={item.name === open()}
                          onClick={() => choose(item)}
                        />
                      )}
                    </For>
                  </List>
                </Section>
              )}
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
      <AccountsSheet />
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
