// Groups: the groups whoever's looking is in, and who's in each, by the names they go by. A group has a Home Screen page of
// its own, which its members see, and an owner adds someone in the network to it, and takes members out.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Field, Icon, Input, leave, List, Problem, Row, Screen, Section, Sheet, Skeleton, Split, toast } from "ui"

/// The name each person goes by, as last answered.
/** A group, as `permissions_groups` answers it. */
interface Group {
  group: string
  called: string
  members: string[]
  owners: string[]
}

/** Someone in the network, as `sign-in_everyone` answers it. */
interface Person {
  who: string
  called?: string
  joined?: boolean
}

let called = new Map<string, string>()

/// The groups, and everyone in the network by who they are, with the name each goes by. A network where nobody
/// signs in has nobody to say who they are, and each is their name in it.
const groups = createMemo(async () => {
  const [list, people] = await Promise.all([call("permissions_groups").then((text): Group[] => JSON.parse(text).groups), call("sign-in_everyone").then((text): Person[] => JSON.parse(text).people, (): Person[] => [])])
  called = new Map(people.filter((person) => person.called).map((person) => [person.who, person.called!]))
  return Object.assign(list, { people: people.filter((person) => person.joined) })
})
changed(() => refresh(groups))

const handle = (who: string) => who.replace("users/", "")
const named = (who: string) => called.get(who) ?? handle(who)
const [open, setOpen] = createSignal<string>()
const [adding, setAdding] = createSignal<"group" | "member">()

/// Does something to the groups, says why if it couldn't be done, and shows them as they are after. → whether
/// it was done.
async function change(tool: string, inputs: object) {
  try {
    await call(tool, inputs)
    return true
  } catch (error) {
    toast((error as Error).message)
  } finally {
    refresh(groups)
  }
}

/// A sheet with one name to type: a new group's, or a new member's.
function Naming(props: { heading: string; action: string; hint?: string; pattern?: string; open: boolean; onClose: () => void; onName: (name: string) => Promise<boolean | undefined> }) {
  const send = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault()
    if (await props.onName(String(new FormData(event.currentTarget).get("name")).trim())) props.onClose()
  }
  return (
    <Sheet heading={props.heading} open={props.open} onClose={props.onClose}>
      <form onSubmit={send}>
        <Field label="Name" hint={props.hint}>
          <Input name="name" required autofocus autocapitalize={props.pattern ? "none" : "words"} autocorrect="off" pattern={props.pattern} />
        </Field>
        <Button type="submit">{props.action}</Button>
      </form>
    </Sheet>
  )
}

function GroupScreen(props: { group: Group }) {
  const remove = async (member: string) => {
    if (await confirm({ heading: `Remove ${named(member)} from ${props.group.called}?`, text: "They stop seeing its page and what's the group's.", action: "Remove", destructive: true }))
      change("permissions_unshare", { artifact: props.group.group, relation: "member", who: member })
  }
  return (
    <Screen heading={props.group.called} back action={<Button variant="outline" onClick={() => setAdding("member")}>Add Member</Button>}>
      <Section heading="Members">
        <List>
          <For each={props.group.members}>
            {(member) => (
              <Row
                heading={named(member)}
                text={props.group.owners.includes(member) ? "Owner" : undefined}
                end={
                  <Button variant="ghost" size="icon" aria-label={`Remove ${named(member)}`} onClick={() => remove(member)}>
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

function Groups(props: { groups: Group[] & { people: Person[] } }) {
  const group = () => props.groups.find(({ group }) => group === open())
  const outside = () => props.groups.people.filter((person) => !group()?.members.includes(person.who))
  return (
    <>
      <Split open={!!group()} onClose={() => setOpen()} detail={<Show when={group()}><GroupScreen group={group()!} /></Show>}>
        <Screen heading="Groups" back onBack={leave} action={<Button onClick={() => setAdding("group")}>New Group</Button>}>
          <Show when={props.groups.length} fallback={<Empty>You're in no groups yet.</Empty>}>
            <List>
              <For each={props.groups} keyed={({ group }) => group}>
                {(group) => (
                  <Row heading={group().called} text={new Intl.ListFormat("en").format(group().members.map(named))} current={open() === group().group} onClick={() => setOpen(group().group)} />
                )}
              </For>
            </List>
          </Show>
        </Screen>
      </Split>
      <Naming heading="New Group" action="New Group" open={adding() === "group"} onClose={() => setAdding()} onName={(name) => change("permissions_group", { name })} />
      <Show
        when={props.groups.people.length}
        fallback={<Naming heading="Add Member" hint="Their name in this network." pattern="[a-zA-Z][a-zA-Z0-9\-]*" action="Add" open={adding() === "member"} onClose={() => setAdding()} onName={(name) => change("permissions_share", { artifact: open()!, relation: "member", who: `users/${name.toLowerCase()}` })} />}
      >
        <Sheet heading="Add Member" open={adding() === "member"} onClose={() => setAdding()}>
          <Show when={outside().length} fallback={<Empty>Everyone in the network is in it.</Empty>}>
            <List>
              <For each={outside()} keyed={(person) => person.who}>
                {(person) => <Row heading={named(person().who)} text={handle(person().who)} onClick={() => (setAdding(), change("permissions_share", { artifact: open()!, relation: "member", who: person().who }))} />}
              </For>
            </List>
          </Show>
        </Sheet>
      </Show>
    </>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your groups" text={String((error() as Error).message ?? error())} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Groups groups={groups()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
