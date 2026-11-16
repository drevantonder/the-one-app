// People: everyone in the network, and whoever's invited and hasn't joined yet. Whoever may invite makes a link
// someone joins by, shared the device's way, and takes someone out. Each person changes the name they go by here,
// from their own row.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Field, Input, leave, List, Menu, Problem, Row, Screen, Section, share, Sheet, Skeleton, toast } from "ui"

/** Someone in the network, or invited to it, as `sign-in_everyone` answers it. */
interface Person {
  who: string
  called?: string
  joined?: boolean
}

const json = <T,>(tool: string, inputs?: object): Promise<T> => call(tool, inputs).then(JSON.parse)

const people = createMemo(async () => {
  const [everyone, [me], owners] = await Promise.all([
    json<Person[]>("sign-in_everyone"),
    json<Person[]>("sign-in_people"),
    // The network's owners invite and take out, and are said to be its owners.
    json<{ relation: string; who: string }[]>("permissions_shared", { artifact: "network" }).then((tuples): string[] => tuples.filter(({ relation }) => relation === "owner").map(({ who }) => who), (): string[] => []),
  ])
  const named = (person: Person) => person.called ?? person.who.replace("users/", "")
  const sorted = everyone.toSorted((a, b) => Number(b.who === me.who) - Number(a.who === me.who) || named(a).localeCompare(named(b)))
  return { me: me.who, owners, invites: owners.includes(me.who), joined: sorted.filter((person) => person.joined), invited: sorted.filter((person) => !person.joined), named }
})
changed(() => refresh(people))

/// What someone's grey line says: their name in the network, and that they own it, or that this is them.
/** Everyone People shows. */
type All = Awaited<ReturnType<typeof people>>

function about(person: Person, all: All) {
  const name = person.who.replace("users/", "")
  return [person.called && person.called !== name ? name : undefined, all.owners.includes(person.who) ? "Owner" : undefined, person.who === all.me ? "You" : undefined]
    .filter(Boolean)
    .join(" · ")
}

/// Takes someone out, or ends an invitation, once whoever's looking has said so.
async function remove(person: Person, all: All) {
  const joined = person.joined
  const question = joined
    ? { heading: `Remove ${all.named(person)}?`, text: "Their devices stop working at once. What they made stays.", action: "Remove" }
    : { heading: `Cancel ${all.named(person)}'s invitation?`, text: "The link they were sent stops working.", action: "Cancel Invitation" }
  if (!(await confirm({ ...question, destructive: true }))) return
  await call("sign-in_remove_person", { who: person.who }).catch((error) => toast(error.message))
  refresh(people)
}

const [inviting, setInviting] = createSignal(false)
const [naming, setNaming] = createSignal(false)

/// The name whoever's looking goes by, to change.
function Name(props: { open: boolean; onClose: () => void; called?: string }) {
  const [error, setError] = createSignal<string>()
  const [working, setWorking] = createSignal(false)
  const save = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    setWorking(true)
    setError()
    try {
      await call("sign-in_name", { called: String(new FormData(event.currentTarget).get("called")).trim() })
      refresh(people)
      props.onClose()
    } catch (thrown) {
      setError((thrown as Error).message)
    } finally {
      setWorking(false)
    }
  }
  return (
    <Sheet heading="Your Name" open={props.open} onClose={() => (props.onClose(), setError())}>
      <form onSubmit={save}>
        <Field label="Name" hint="As people here know you." error={error()}>
          <Input name="called" required autofocus autocomplete="name" value={props.called ?? ""} />
        </Field>
        <Button type="submit" loading={working()}>Save</Button>
      </form>
    </Sheet>
  )
}

/// Someone new: their name in the network, then the link they join by, to send them.
function Invite(props: { open: boolean; onClose: () => void }) {
  const [link, setLink] = createSignal<string>()
  const [error, setError] = createSignal<string>()
  const [working, setWorking] = createSignal(false)
  const close = () => (props.onClose(), setLink(), setError())
  const make = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    const name = String(new FormData(event.currentTarget).get("name")).trim().toLowerCase()
    setWorking(true)
    setError()
    try {
      // The tool says who joins where, and the link is its last word.
      setLink((await call("sign-in_invite", { name })).match(/https?:\/\/\S+/)![0])
      refresh(people)
    } catch (thrown) {
      setError((thrown as Error).message)
    } finally {
      setWorking(false)
    }
  }
  const send = async () => {
    try {
      if ((await share(link()!)) === "copied") toast("Link copied")
    } catch (thrown) {
      toast((thrown as Error).message)
    }
  }
  return (
    <Sheet heading={link() ? "Send This Link" : "Invite Someone"} open={props.open} onClose={close}>
      <Show
        when={link()}
        fallback={
          <form onSubmit={make}>
            <Field label="Name" hint="Lowercase letters, digits and hyphens. It's who they are here, for good." error={error()}>
              <Input name="name" required autofocus autocapitalize="none" autocorrect="off" spellcheck={false} pattern="[a-zA-Z][a-zA-Z0-9\-]*" />
            </Field>
            <Button type="submit" loading={working()}>Create Link</Button>
          </form>
        }
      >
        <Field label="Link" hint="It works once, for a week.">
          <Input readonly value={link()} onFocus={(event) => event.currentTarget.select()} />
        </Field>
        <Button onClick={send}>Share Link</Button>
      </Show>
    </Sheet>
  )
}

/// Someone's row: what they go by, and for whoever may, what can be done to them.
function PersonRow(props: { person: Person; all: All }) {
  const mayRemove = () => props.all.invites && props.person.who !== props.all.me
  return (
    <Row
      heading={props.all.named(props.person)}
      text={about(props.person, props.all) || undefined}
      onClick={props.person.who === props.all.me ? () => setNaming(true) : undefined}
      end={
        <Show when={mayRemove()}>
          <Menu.Root onSelect={({ value }) => value === "remove" && remove(props.person, props.all)}>
            <Menu.Trigger aria-label={`More for ${props.all.named(props.person)}`} />
            <Menu.Positioner>
              <Menu.Content>
                <Menu.Item value="remove" destructive>
                  {props.person.joined ? `Remove ${props.all.named(props.person)}` : "Cancel Invitation"}
                </Menu.Item>
              </Menu.Content>
            </Menu.Positioner>
          </Menu.Root>
        </Show>
      }
    />
  )
}

function People(props: { all: All }) {
  return (
    <>
      <Screen heading="People" back onBack={leave} action={props.all.invites ? <Button onClick={() => setInviting(true)}>Invite</Button> : undefined}>
        <List>
          <For each={props.all.joined} keyed={(person) => person.who}>
            {(person) => <PersonRow person={person()} all={props.all} />}
          </For>
        </List>
        <Show when={props.all.invited.length}>
          <Section heading="Invited">
            <List>
              <For each={props.all.invited} keyed={(person) => person.who}>
                {(person) => <PersonRow person={person()} all={props.all} />}
              </For>
            </List>
          </Section>
        </Show>
      </Screen>
      <Invite open={inviting()} onClose={() => setInviting(false)} />
      <Name open={naming()} onClose={() => setNaming(false)} called={props.all.joined.find((person) => person.who === props.all.me)?.called} />
    </>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="People" back onBack={leave}><Problem heading="Can't show who's in your network" text={String((error() as Error).message ?? error())} /></Screen>}>
      <Loading fallback={<Screen heading="People" back onBack={leave}><Skeleton rows={3} /></Screen>}>
        <People all={people()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
