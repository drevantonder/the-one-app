// The Vault: someone's keys, sign-ins and accounts. A list of items, each opening beside it: its fields (a secret
// one is typed and never shown again), the sites its secrets go to, and what may use it. Above them, what's been
// asked for and waits for an answer. Below, the servers that open the Vault, and which devices hold its person's
// key, since only one of those chooses the servers.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import "./app.css"
import { Button, call, changed, Checkbox, confirm, Empty, Field, Input, leave, List, Problem, RadioGroup, Row, Screen, Section, Sheet, showRecoveryKey, Skeleton, Split, Switch, toast, useRecoveryKey } from "ui"

/// Which servers open the Vault and which may, what this device holds of its person's key, and the items.
const opened = createMemo(() => call("vault_opened_by").then(JSON.parse))
const key = createMemo(() => call("encryption_key").then(JSON.parse))
const kept = createMemo(() => (opened().servers.length ? call("vault_list").then(JSON.parse) : { items: [], requests: [], plugins: [] }))
const again = () => [opened, key, kept].forEach(refresh)
changed(again)

/// Does something that changes the Vault, and says what the server said if it wouldn't. → whether it did.
async function does(tool, inputs) {
  try {
    await call(tool, inputs)
    again()
    return true
  } catch (error) {
    toast(error.message)
    return false
  }
}

/// A field's name as a person reads it: "api_key" is "API key".
const label = (name) =>
  String(name)
    .split(/[_-]+/)
    .map((word, at) => (/^(api|url|id|ssh|aws|ip)$/i.test(word) ? word.toUpperCase() : at ? word.toLowerCase() : word[0].toUpperCase() + word.slice(1).toLowerCase()))
    .join(" ")
/// A few things in a sentence: "fake, gmail and your agents".
const listed = (things) => new Intl.ListFormat("en", { type: "conjunction" }).format(things)
const sites = (typed) => String(typed ?? "").split(/[\s,]+/).filter(Boolean)
/// What may use an item, in its grey line.
const users = (item) => [...item.plugins, ...(item.agents ? ["your agents"] : [])]

/// The item that's open beside the list, by its name, and the sheet that's open over the page.
const [open, setOpen] = createSignal()
const [sheet, setSheet] = createSignal()
const shut = () => setSheet()
const holds = () => !!key().slot

/// A field of an item to type: a new item's first, a new one of an item's, or one that's there to change.
function FieldSheet() {
  const at = () => (sheet()?.kind === "field" ? sheet() : undefined)
  const [secret, setSecret] = createSignal(true)
  const save = async (event) => {
    event.preventDefault()
    const typed = Object.fromEntries(new FormData(event.target))
    const item = at().item?.name ?? typed.item
    const inputs = { item, field: at().field?.name ?? typed.field, value: typed.value, secret: at().field ? at().field.secret : secret() }
    if (!at().item) inputs.hosts = sites(typed.hosts)
    if ((await does("vault_set", inputs)) && !at().item) setOpen(item)
    shut()
  }
  const remove = async () => {
    const { item, field } = at()
    if (!(await confirm({ heading: `Delete ${label(field.name)}?`, text: "It can't be brought back.", action: "Delete", destructive: true }))) return
    await does("vault_remove", { item: item.name, field: field.name })
    shut()
  }
  return (
    <Sheet heading={at()?.field ? label(at().field.name) : at()?.item ? "Add Field" : "Add Item"} open={!!at()} onClose={shut}>
      <Show when={at()}>
        <form onSubmit={save}>
          <Show when={!at().item}>
            <Field label="Name">
              <Input name="item" placeholder="GitHub" required autofocus />
            </Field>
          </Show>
          <Show when={!at().field}>
            <Field label="Field">
              <Input name="field" placeholder="token" pattern="[A-Za-z0-9_]+" autocapitalize="none" spellcheck={false} required />
            </Field>
          </Show>
          <Field label={at().field ? "New value" : "Value"} hint={(at().field ? at().field.secret : secret()) ? "It's encrypted on this device, and never shown again." : undefined}>
            <Input name="value" type={(at().field ? at().field.secret : secret()) ? "password" : "text"} value={at().field?.value ?? ""} autocomplete="off" spellcheck={false} required autofocus={!!at().item} />
          </Field>
          <Show when={!at().field}>
            <Checkbox label="Secret" checked={secret()} onChange={(event) => setSecret(event.target.checked)} />
          </Show>
          <Show when={!at().item}>
            <Field label="Sites" hint="Where its secrets may be sent, like api.github.com.">
              <Input name="hosts" autocapitalize="none" spellcheck={false} />
            </Field>
          </Show>
          <Button type="submit">Save</Button>
          <Show when={at().field}>
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
  const asked = () => (sheet()?.kind === "request" ? sheet().request : undefined)
  const [picked, setPicked] = createSignal("")
  const fits = () => kept().items.filter((item) => asked().fields.every((wanted) => item.fields.some((field) => field.name === wanted.name)))
  const who = () => asked().plugin ?? "Your agents"
  const allow = async (event) => {
    event.preventDefault()
    const typed = Object.fromEntries(new FormData(event.target))
    const answer = picked() ? { item: picked() } : { values: JSON.stringify(Object.fromEntries(asked().fields.map(({ name }) => [name, typed[name]]))) }
    if (await does("vault_answer", { id: asked().id, ...answer })) shut()
  }
  const decline = async () => (await does("vault_answer", { id: asked().id, decline: true })) && shut()
  return (
    <Sheet heading={asked() ? `${who()} ${asked().plugin ? "needs" : "need"} ${asked().title}` : ""} open={!!asked()} onClose={shut}>
      <Show when={asked()}>
        <form onSubmit={allow}>
          <p class="about">
            {asked().why ? `${asked().why}. ` : ""}
            {who()} can use {picked() || asked().title}
            {!picked() && asked().hosts.length ? `, sent only to ${listed(asked().hosts)}` : ""}. Nobody is shown what you type.
          </p>
          <Show when={fits().length}>
            <Field label="Use">
              <RadioGroup name="picked" value={picked()} onChange={setPicked} options={[...fits().map((item) => ({ value: item.name, label: item.name })), { value: "", label: "New Item" }]} />
            </Field>
          </Show>
          <Show when={!picked()}>
            <For each={asked().fields}>
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
function Item(props) {
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
        <Switch label="Your agents" hint="They make requests with it, and never see it." checked={item().agents} onChange={(event) => does("vault_agents", { item: item().name, on: event.target.checked })} />
        <For each={kept().plugins}>
          {(plugin) => <Switch label={plugin} hint="Its code is handed the secret." checked={item().plugins.includes(plugin)} onChange={(event) => does("vault_use", { item: item().name, plugin, on: event.target.checked })} />}
        </For>
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
  const choose = (server, on) => {
    const servers = opened().servers.filter((chosen) => chosen !== server)
    return does("vault_grant", { servers: on ? [...servers, server] : servers })
  }
  return (
    <Section heading="Opened by">
      <For each={opened().may}>
        {(server) => (
          <Switch
            label={server.name ?? "A server"}
            hint="It holds your Vault's key, and runs what uses your secrets."
            checked={opened().servers.includes(server.key)}
            disabled={!holds()}
            onChange={(event) => choose(server.key, event.target.checked)}
          />
        )}
      </For>
    </Section>
  )
}

/// The devices that hold this person's key, the ones asking to, and what this device does about being neither.
function YourKey() {
  const others = () => key().devices.filter((device) => device.device !== key().device)
  return (
    <Section heading="Your key">
      <Show
        when={holds()}
        fallback={
          <Problem
            warn
            heading="This device doesn't hold your key"
            text="It adds and uses items. Choosing the servers that open your Vault takes a device that holds your key."
            action={
              <>
                <Button variant="outline" onClick={() => does("encryption_ask", {}).then((asked) => asked && toast("Asked. Open Vault on a device that holds your key."))}>
                  Ask Another Device
                </Button>
                <Button variant="outline" onClick={() => useRecoveryKey().then(again)}>
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
                text="Asks to hold your key"
                end={
                  <Button variant="secondary" onClick={() => does("encryption_let_in", { device: device.device, name: device.name })}>
                    Let In
                  </Button>
                }
              />
            )}
          </For>
          <Row heading="This device" text="Holds your key" />
          <For each={others()}>{(device) => <Row heading={device.name} text="Holds your key" />}</For>
          <Row heading="Recovery Key" text="Lets a device in when every other is gone" onClick={showRecoveryKey} />
        </List>
      </Show>
    </Section>
  )
}

/// Making someone's key is the first thing, on the device they open the Vault on first. It shows them their
/// recovery key then, which is the one time they're asked to write something down.
async function setUp() {
  if (await does("encryption_let_in", {})) await showRecoveryKey()
}

function Vault() {
  const item = () => kept().items.find(({ name }) => name === open())
  return (
    <>
      <Split open={!!item()} onClose={() => setOpen()} detail={<Show when={item()}>{(item) => <Item item={item()} />}</Show>}>
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
        <Show when={key().check} fallback={<Empty action={<Button onClick={setUp}>Set Up</Button>}>Keys and sign-ins are kept here, encrypted. Yours aren't set up yet.</Empty>}>
          <Show when={!opened().servers.length}>
            <Problem warn heading="No server opens your Vault" text={holds() ? "Choose one below. Until then it keeps nothing." : "Choose one from a device that holds your key. Until then it keeps nothing."} />
          </Show>
          <Show when={kept().requests.length}>
            <Section heading="Asked for">
              <List>
                <For each={kept().requests}>
                  {(request) => <Row heading={request.title} text={[`For ${request.plugin ?? "your agents"}`, request.why].filter(Boolean).join(" · ")} onClick={() => setSheet({ kind: "request", request })} />}
                </For>
              </List>
            </Section>
          </Show>
          <Show when={opened().servers.length}>
            <Section>
              <Show when={kept().items.length} fallback={<Empty>Nothing's kept here yet.</Empty>}>
                <List>
                  <For each={kept().items}>
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
    <Errored fallback={(error) => <Problem heading="Can't show your Vault" text={String(error().message ?? error())} />}>
      <Loading fallback={<Skeleton rows={4} />}>
        <Vault />
      </Loading>
    </Errored>
  ),
  document.body,
)
