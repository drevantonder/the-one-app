// The Vault: someone's keys, sign-ins and accounts, or a group's or the network's, for whoever owns them. A list of items, each opening beside it: its fields (a secret
// one is typed and never shown again), the sites its secrets go to, and what may use it. Above them, what's been
// asked for and waits for an answer. Below, the servers that open the Vault, and which devices hold its person's
// key, since only one of those chooses the servers.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import "./app.css"
import { Button, call, changed, Checkbox, confirm, createListCollection, Empty, Field, Input, leave, List, Problem, RadioGroup, Row, Screen, Section, Select, Sheet, showRecoveryKey, Skeleton, Split, Switch, toast, useRecoveryKey } from "ui"

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
  fields: ItemField[]
  hosts: string[]
  plugins: string[]
  agents: boolean
  sign_in?: boolean
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

/** The sheet that's open over the page: a field to type, or a request to answer. */
type Sheet = { kind: "field"; item?: Item; field?: ItemField } | { kind: "request"; request: Request }

const [whose, setWhose] = createSignal<string>()
const ask = <T,>(tool: string, inputs?: object): Promise<T> => call(tool, inputs, { for: whose() }).then(JSON.parse)

/// Which servers open the Vault and which may, what this device holds of its owner's key, and the items.
const opened = createMemo(() => ask<{ servers: string[]; may: { key: string; name?: string }[] }>("vault_unlocked_on"))
const key = createMemo(() => ask<{ owner: string; slot?: unknown; check?: unknown; device: string; devices: Device[]; asking: Device[] }>("encryption_key"))
const contents = createMemo(() => (opened().servers.length ? ask<{ items: Item[]; requests: Request[]; plugins: string[] }>("vault_list") : { items: [], requests: [], plugins: [] }))
const again = () => [opened, key, contents].forEach(refresh)
changed(again)

/// Whose Vaults its person can open here: theirs, each group they own, and the network if they own it.
const owners = createMemo(async () => {
  const me: string | undefined = (await call("sign-in_people").then(JSON.parse, () => []))[0]?.who
  if (!me) return []
  const [groups, network] = await Promise.all([
    call("permissions_groups", {}, { for: me }).then((text): { group: string; called: string; owners: string[] }[] => JSON.parse(text), () => []),
    call("permissions_check", { artifact: "network", relation: "owner" }, { for: me }).then((result): boolean => JSON.parse(result).may, () => false),
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
    await call(tool, inputs, { for: whose() })
    again()
    return true
  } catch (error) {
    toast((error as Error).message)
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
    const answer = picked() ? { item: picked() } : { values: JSON.stringify(Object.fromEntries(request()!.fields.map(({ name }) => [name, typed[name]]))) }
    if (await does("vault_answer", { id: request()!.id, ...answer })) shut()
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
                <Button variant="outline" onClick={() => useRecoveryKey({ for: whose() }).then(again)}>
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
          <Row heading="Recovery Key" text="For when every device is gone" onClick={() => showRecoveryKey({ for: whose() })} />
        </List>
      </Show>
    </Section>
  )
}

/// Making someone's key is the first thing, on the device they open the Vault on first: theirs, or a group's or
/// the network's, by one of its owners. It shows them its recovery key then, which is the one time they're asked
/// to write something down.
async function setUp() {
  if (await does("encryption_approve", {})) await showRecoveryKey({ for: whose() })
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

function Vault() {
  const item = () => contents().items.find(({ name }) => name === open())
  return (
    <>
      <Split open={!!item()} onClose={() => setOpen()} detail={<Show when={item()} fallback={<Show when={contents().items.length}><Empty>Pick an item to see what it holds.</Empty></Show>}>{(item) => <ItemScreen item={item()} />}</Show>}>
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
            <Section>
              <Show when={contents().items.length} fallback={<Empty>Nothing's kept here yet.</Empty>}>
                <List>
                  <For each={contents().items}>
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
          </Show>
          <OpenedBy />
          <YourKey />
        </Show>
        </Screen>
      </Split>
      <FieldSheet />
      <RequestSheet />
    </>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your Vault" text={String((error() as Error).message ?? error())} />}>
      <Loading fallback={<Skeleton rows={4} />}>
        <Vault />
      </Loading>
    </Errored>
  ),
  document.body,
)
