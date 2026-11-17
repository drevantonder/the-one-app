// Users & Groups: everyone in the network and whoever's invited and hasn't joined yet, then the groups whoever's
// looking is in, and who's in each. Whoever may invite makes a link someone joins by, shared the device's way, and
// takes someone out; each person changes the name they go by here, from their own row. Anyone makes a group, which
// has a Home Screen of its own that its members see, and its owners add someone in the network to it and take members
// out. Groups are each person's own, so they're asked for as the person, wherever the app was opened.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Field, Icon, Input, leave, List, Menu, Problem, Row, Screen, Section, share, Sheet, Skeleton, Split, toast } from "ui"

/** Someone in the network, or invited to it, as `sign-in_everyone` answers it. */
interface Person {
  who: string
  called?: string
  joined?: boolean
}

/** A group, as `permissions_groups` answers it. */
interface Group {
  group: string
  called: string
  members: string[]
  owners: string[]
}

const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

const all = createMemo(async () => {
  const [everyone, [me], owners] = await Promise.all([
    // A network being built has nobody signing in, so nobody to list.
    json<{ people: Person[] }>("sign-in_everyone").then(({ people }) => people, (): Person[] => []),
    json<{ people: Person[] }>("sign-in_people").then(({ people }) => people),
    // The network's owners invite and take out, and are said to be its owners.
    json<{ tuples: { relation: string; who: string }[] }>("permissions_shared", { artifact: "network" }).then(({ tuples }): string[] => tuples.filter(({ relation }) => relation === "owner").map(({ who }) => who), (): string[] => []),
  ])
  const groups = await json<{ groups: Group[] }>("permissions_groups", {}, me.who).then(({ groups }) => groups)
  const named = (who: string) => everyone.find((person) => person.who === who)?.called ?? who.replace("users/", "")
  const sorted = everyone.toSorted((a, b) => Number(b.who === me.who) - Number(a.who === me.who) || named(a.who).localeCompare(named(b.who)))
  return { me: me.who, owners, invites: owners.includes(me.who), joined: sorted.filter((person) => person.joined), invited: sorted.filter((person) => !person.joined), groups, named }
})
changed(() => refresh(all))

/** Everything Users & Groups shows. */
type All = Awaited<ReturnType<typeof all>>

/// What someone's grey line says: their name in the network, and that they own it, or that this is them.
function about(person: Person, all: All) {
  const name = person.who.replace("users/", "")
  return [person.called && person.called !== name ? name : undefined, all.owners.includes(person.who) ? "Owner" : undefined, person.who === all.me ? "You" : undefined]
    .filter(Boolean)
    .join(" · ")
}

/// Does something as the person, says why if it couldn't be done, and shows everything as it is after. → whether it
/// was done.
async function change(tool: string, inputs: object, me?: string) {
  try {
    await call(tool, inputs, { subject: me })
    return true
  } catch (error) {
    toast(error as Error)
  } finally {
    refresh(all)
  }
}

/// Takes someone out, or ends an invitation, once whoever's looking has said so.
async function remove(person: Person, all: All) {
  const question = person.joined
    ? { heading: `Remove ${all.named(person.who)}?`, text: "Their devices stop working at once. What they made stays.", action: "Remove" }
    : { heading: `Cancel ${all.named(person.who)}'s invitation?`, text: "The link they were sent stops working.", action: "Cancel Invitation" }
  if (await confirm({ ...question, destructive: true })) change("sign-in_remove_person", { who: person.who })
}

const [inviting, setInviting] = createSignal(false)
const [naming, setNaming] = createSignal(false)
const [opened, setOpened] = createSignal<string>()
const [adding, setAdding] = createSignal<"group" | "member">()

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
      refresh(all)
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
      setLink(JSON.parse(await call("sign-in_invite", { name })).link)
      refresh(all)
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
      toast(thrown as Error)
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

/// A sheet with one name to type: a new group's.
function NewGroup(props: { open: boolean; onClose: () => void; me: string }) {
  const send = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    if (await change("permissions_group", { name: String(new FormData(event.currentTarget).get("name")).trim() }, props.me)) props.onClose()
  }
  return (
    <Sheet heading="New Group" open={props.open} onClose={props.onClose}>
      <form onSubmit={send}>
        <Field label="Name">
          <Input name="name" required autofocus autocapitalize="words" autocorrect="off" />
        </Field>
        <Button type="submit">New Group</Button>
      </form>
    </Sheet>
  )
}

/// Someone's row: what they go by, and for whoever may, what can be done to them.
function PersonRow(props: { person: Person; all: All }) {
  const mayRemove = () => props.all.invites && props.person.who !== props.all.me
  return (
    <Row
      heading={props.all.named(props.person.who)}
      text={about(props.person, props.all) || undefined}
      onClick={props.person.who === props.all.me ? () => setNaming(true) : undefined}
      end={
        <Show when={mayRemove()}>
          <Menu.Root onSelect={({ value }) => value === "remove" && remove(props.person, props.all)}>
            <Menu.Trigger aria-label={`More for ${props.all.named(props.person.who)}`} />
            <Menu.Positioner>
              <Menu.Content>
                <Menu.Item value="remove" destructive>
                  {props.person.joined ? `Remove ${props.all.named(props.person.who)}` : "Cancel Invitation"}
                </Menu.Item>
              </Menu.Content>
            </Menu.Positioner>
          </Menu.Root>
        </Show>
      }
    />
  )
}

/// A group: who's in it, each to take out, and someone in the network to add.
function GroupScreen(props: { group: Group; all: All }) {
  const remove = async (member: string) => {
    if (await confirm({ heading: `Remove ${props.all.named(member)} from ${props.group.called}?`, text: "They stop seeing its Home Screen and what's the group's.", action: "Remove", destructive: true }))
      change("permissions_unshare", { artifact: props.group.group, relation: "member", who: member }, props.all.me)
  }
  return (
    <Screen heading={props.group.called} back action={<Button variant="outline" onClick={() => setAdding("member")}>Add Member</Button>}>
      <Section heading="Members">
        <List>
          <For each={props.group.members}>
            {(member) => (
              <Row
                heading={props.all.named(member)}
                text={props.group.owners.includes(member) ? "Owner" : undefined}
                end={
                  <Button variant="ghost" size="icon" aria-label={`Remove ${props.all.named(member)}`} onClick={() => remove(member)}>
                    <Icon name="trash" />
                  </Button>
                }
              />
            )}
          </For>
        </List>
      </Section>
    </Screen>
  )
}

/// Someone in the network, not yet in the group, to add.
function AddMember(props: { all: All; group?: Group }) {
  const outside = () => props.all.joined.filter((person) => !props.group?.members.includes(person.who))
  return (
    <Sheet heading="Add Member" open={adding() === "member"} onClose={() => setAdding()}>
      <Show when={outside().length} fallback={<Empty>Everyone in the network is in it.</Empty>}>
        <List>
          <For each={outside()} keyed={(person) => person.who}>
            {(person) => <Row heading={props.all.named(person().who)} text={person().who.replace("users/", "")} onClick={() => (setAdding(), change("permissions_share", { artifact: opened()!, relation: "member", who: person().who }, props.all.me))} />}
          </For>
        </List>
      </Show>
    </Sheet>
  )
}

function UsersAndGroups(props: { all: All }) {
  const group = () => props.all.groups.find(({ group }) => group === opened())
  return (
    <>
      <Split open={!!group()} onClose={() => setOpened()} detail={<Show when={group()}><GroupScreen group={group()!} all={props.all} /></Show>}>
        <Screen heading="Users & Groups" back onBack={leave} action={props.all.invites ? <Button onClick={() => setInviting(true)}>Invite</Button> : undefined}>
          <Section heading="Users">
            <List>
              <For each={props.all.joined} keyed={(person) => person.who}>
                {(person) => <PersonRow person={person()} all={props.all} />}
              </For>
            </List>
          </Section>
          <Show when={props.all.invited.length}>
            <Section heading="Invited">
              <List>
                <For each={props.all.invited} keyed={(person) => person.who}>
                  {(person) => <PersonRow person={person()} all={props.all} />}
                </For>
              </List>
            </Section>
          </Show>
          <Section heading={<span class="groups"><span>Groups</span><Button variant="secondary" onClick={() => setAdding("group")}>New Group</Button></span>}>
            <Show when={props.all.groups.length} fallback={<Empty>You're in no groups yet.</Empty>}>
              <List>
                <For each={props.all.groups} keyed={({ group }) => group}>
                  {(group) => <Row heading={group().called} text={new Intl.ListFormat("en").format(group().members.map(props.all.named))} current={opened() === group().group} onClick={() => setOpened(group().group)} />}
                </For>
              </List>
            </Show>
          </Section>
        </Screen>
      </Split>
      <Invite open={inviting()} onClose={() => setInviting(false)} />
      <Name open={naming()} onClose={() => setNaming(false)} called={props.all.joined.find((person) => person.who === props.all.me)?.called} />
      <NewGroup open={adding() === "group"} onClose={() => setAdding()} me={props.all.me} />
      <AddMember all={props.all} group={group()} />
    </>
  )
}

render(
  () => (
    <>
      <style>{".groups { display: flex; align-items: center; gap: 0.75rem; max-width: var(--readable); } .groups > span { flex: 1; min-width: 0; }"}</style>
      <Errored fallback={(error) => <Screen heading="Users & Groups" back onBack={leave}><Problem heading="Can't show who's in your network" text={error() as Error} /></Screen>}>
        <Loading fallback={<Screen heading="Users & Groups" back onBack={leave}><Skeleton rows={3} /></Screen>}>
          <UsersAndGroups all={all()} />
        </Loading>
      </Errored>
    </>
  ),
  document.body,
)
