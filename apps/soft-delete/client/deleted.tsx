// Recently Deleted: what you deleted in the last 30 days, the latest first, each with how long until it's deleted for
// good. Restore or Delete Now from a row's menu, and Delete All for everything.
import { createMemo, Errored, Loading, refresh, Show } from "solid-js"
import { call, changed, confirm, Empty, List, Menu, Problem, Row, Screen, Skeleton, Spinner, styles, Text, toast, tokens } from "ui"

const look = styles({ note: { color: tokens.colour.secondaryLabel, maxWidth: tokens.length.readable }, wrong: { color: tokens.colour.red } })

/** A deleted item, as `soft-delete_list` answers it. */
interface Item {
  item: string
  state: "deleted" | "purging"
  artifacts: string[]
  app: string | null
  title: string
  installed: boolean
  owner: string
  names: string | null
  purges: number
  why: string | null
}

const items = createMemo(() => call("soft-delete_list").then((text): Item[] => JSON.parse(text).deleted))
/// What the apps that name their own artifacts call them, by artifact, asked from this device: the Vault's are ids, as
/// its items' names are encrypted, and only a server its person chose reads them.
const called = createMemo(async () => {
  const asked = Map.groupBy(items().filter((item) => item.names), (item) => `${item.names}_names ${item.owner}`)
  const answers = await Promise.all(
    [...asked.values()].map((some) =>
      call(`${some[0].names}_names`, { artifacts: some.flatMap((item) => item.artifacts) }, { subject: some[0].owner }).then(
        (text): { artifact: string; name: string; kind?: string | null }[] => JSON.parse(text).names,
        () => [],
      ),
    ),
  )
  return new Map(answers.flat().map((one) => [one.artifact, one]))
})
changed(() => refresh(items))

/// An artifact's name as a person reads it: users/alice/made/git-repos/notes is notes.
const named = (artifact: string) => artifact.split("/").pop()!
/// What a person calls an item: an uninstalled app's name, else its one artifact's, as its app says it where it names
/// them itself, or by its app's name where it can't say here: "Vault item".
function heading(item: Item) {
  if (item.app) return item.title
  if (!item.names) return named(item.artifacts[0])
  return called().get(item.artifacts[0])?.name ?? `${item.title} item`
}
/// What an uninstalled app took with it, as the real things: "With notes, site and 10 more".
function along(item: Item) {
  const names = item.artifacts.filter((artifact) => artifact.includes("/made/")).map(named)
  if (!names.length) return ""
  const shown = names.length > 3 ? [...names.slice(0, 2), `${names.length - 2} more`] : names
  return `With ${new Intl.ListFormat(undefined, { type: "conjunction" }).format(shown)}`
}
/// How long until it's deleted for good, in whole days, as iOS's Recently Deleted says it: "29 days".
function left(item: Item) {
  const days = Math.max(1, Math.ceil((item.purges - Date.now()) / 86_400_000))
  return days === 1 ? "1 day" : `${days} days`
}
/// An item's grey line: what's under way or wrong, what it needs to open, and how long it has.
function about(item: Item) {
  if (item.state === "purging") return "Deleting…"
  if (item.why) return "Couldn't delete for good. It's tried again tomorrow."
  const kind = item.names ? called().get(item.artifacts[0])?.kind : undefined
  const needs = item.app ? along(item) : item.installed ? (kind ?? item.title) : `Install ${item.title} to open it`
  return [needs, left(item)].filter(Boolean).join(" · ")
}

/// Does something, says why if it couldn't, and shows everything as it is after. → what it answered, or nothing.
async function doing(tool: string, inputs: object = {}): Promise<any> {
  try {
    return JSON.parse(await call(tool, inputs))
  } catch (error) {
    toast(error as Error)
  } finally {
    refresh(items)
  }
}

async function restore(item: Item) {
  const restored = await doing("soft-delete_restore", { item: item.item })
  if (!restored) return
  const moved = (restored.restored as { artifact: string; as: string }[]).filter((one) => one.as !== one.artifact)
  if (moved.length) toast(`${named(moved[0].artifact)} is back as ${named(moved[0].as)}, beside the one made since.`)
  else if (!restored.installed) toast(`${heading(item)} is back. Install ${restored.title} to open it.`)
}

async function purge(item: Item) {
  const question = { heading: `Delete ${heading(item)} for good?`, text: "It can't be restored.", action: "Delete", destructive: true }
  if (await confirm(question)) await doing("soft-delete_purge", { item: item.item })
}

/// Tapping a row brings it back, after asking, as the menu's Restore does.
async function ask(item: Item) {
  if (item.state === "deleted" && (await confirm({ heading: `Restore ${heading(item)}?`, text: about(item), action: "Restore" }))) restore(item)
}

async function purgeAll() {
  const question = { heading: "Delete everything for good?", text: "None of it can be restored.", action: "Delete All", destructive: true }
  if (await confirm(question)) await doing("soft-delete_purge_all")
}

function Deleted(props: { items: Item[] }) {
  const deleted = () => props.items.filter((item) => item.state === "deleted")
  return (
    <Screen heading="Recently Deleted" back>
      <Show when={props.items.length} fallback={<Empty>Nothing was deleted in the last 30 days.</Empty>}>
        <Text style={look.note}>Deleted for good after 30 days.</Text>
        <List each={props.items} keyed={(item) => item.item}>
          {(item) => (
            <Row
              heading={heading(item())}
              text={<Text style={[!!item().why && look.wrong]}>{about(item())}</Text>}
              onPress={() => ask(item())}
              end={
                <Show when={item().state === "deleted"} fallback={<Spinner />}>
                  <Menu.Root onSelect={(value) => (value === "restore" ? restore(item()) : purge(item()))}>
                    <Menu.Trigger aria-label={`More for ${heading(item())}`} />
                    <Menu.Positioner>
                      <Menu.Content>
                        <Menu.Item value="restore">Restore</Menu.Item>
                        <Menu.Separator />
                        <Menu.Item value="purge" destructive>Delete Now</Menu.Item>
                      </Menu.Content>
                    </Menu.Positioner>
                  </Menu.Root>
                </Show>
              }
            />
          )}
        </List>
        <Show when={deleted().length}>
          <List>
            <Row heading="Delete All…" destructive onPress={purgeAll} />
          </List>
        </Show>
      </Show>
    </Screen>
  )
}

/// Recently Deleted, as Settings draws it.
export default function RecentlyDeleted() {
  return (
    <Errored fallback={(error) => <Screen heading="Recently Deleted" back><Problem heading="Can't show what's deleted" text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading="Recently Deleted" back><Skeleton rows={3} /></Screen>}>
        <Deleted items={items()} />
      </Loading>
    </Errored>
  )
}
