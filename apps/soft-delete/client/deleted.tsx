// Recently Deleted: what you deleted in the last 30 days, the latest first, each as a person knows it (its name, with
// its app's icon and what it is) and how long until it's deleted for good, as iOS's Photos and Files show theirs. A
// press shows an item beside the list (pushed over it on a phone), where Restore and Delete Now are rows of their own;
// the same two are in its row's menu, and Delete All is for everything.
import { createMemo, createSignal, Errored, Loading, refresh, Show } from "solid-js"
import { call, changed, confirm, Empty, Image, List, Menu, Problem, Row, Screen, Skeleton, Spinner, Split, styles, Text, toast, tokens } from "ui"

const look = styles({
  note: { color: tokens.colour.secondaryLabel, maxWidth: tokens.length.readable },
  wrong: { color: tokens.colour.red },
  icon: { width: tokens.length.sidebarIcon, height: tokens.length.sidebarIcon },
})

/** A deleted item, as `soft-delete_list` answers it. */
interface Item {
  item: string
  state: "deleted" | "purging"
  artifacts: string[]
  app: string | null
  title: string
  icon: string | null
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
/// An older copy of an app, which a newer one replaced: the app's own artifact, not what it made.
const copy = (item: Item) => !item.app && item.artifacts[0].includes("/apps/")
/// What a person calls an item: an app's name, for it uninstalled or an older copy of it, else its one artifact's, as
/// its app says it where it names them itself, or by its app's name where it can't say here: "Vault item".
function heading(item: Item) {
  if (item.app || copy(item)) return item.title
  if (!item.names) return named(item.artifacts[0])
  return called().get(item.artifacts[0])?.name ?? `${item.title} item`
}
/// What kind of thing it is, as a person says it: an app, with what it kept, an older copy of one, or what an app made.
function kind(item: Item) {
  if (item.app) return along(item) ? `App · ${along(item)}` : "App"
  if (copy(item)) return "Older copy of the app"
  return (item.names ? called().get(item.artifacts[0])?.kind : undefined) ?? item.title
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
/// An item's grey line: what's under way or wrong, else what it is and how long it has.
function about(item: Item) {
  if (item.state === "purging") return "Deleting…"
  if (item.why) return "Couldn't delete for good. It's tried again tomorrow."
  return [kind(item), left(item)].join(" · ")
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

async function purgeAll() {
  const question = { heading: "Delete everything for good?", text: "None of it can be restored.", action: "Delete All", destructive: true }
  if (await confirm(question)) await doing("soft-delete_purge_all")
}

/// An item's icon: its app's.
const Tile = (props: { icon: string | null }) => <Show when={props.icon}>{(icon) => <Image src={icon()} alt="" style={look.icon} />}</Show>

/// One item: what it is, how long it has, and Restore and Delete Now as rows of their own.
function Detail(props: { item: Item }) {
  const item = () => props.item
  return (
    <Screen heading={heading(item())} back>
      <List>
        <Row start={<Tile icon={item().icon} />} heading={heading(item())} text={kind(item())} />
        <Row heading="Deleted for good in" end={<Text>{left(item())}</Text>} />
        <Show when={item().state === "deleted" && !item().installed && !item().app}>
          <Row heading={`Install ${item().title} to open it once it's back.`} />
        </Show>
      </List>
      <Show when={item().state === "deleted"}>
        <List>
          <Row heading="Restore" onPress={() => restore(item())} />
          <Row heading="Delete Now…" destructive onPress={() => purge(item())} />
        </List>
      </Show>
    </Screen>
  )
}

function Deleted(props: { items: Item[] }) {
  const [open, setOpen] = createSignal<string>()
  const deleted = () => props.items.filter((item) => item.state === "deleted")
  const opened = () => props.items.find((item) => item.item === open())
  return (
    <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()}>{(item) => <Detail item={item()} />}</Show>}>
      <Screen heading="Recently Deleted" back>
        <Show when={props.items.length} fallback={<Empty>Nothing was deleted in the last 30 days.</Empty>}>
          <Text style={look.note}>Deleted for good after 30 days.</Text>
          <List each={props.items} keyed={(item) => item.item}>
            {(item) => (
              <Row
                start={<Tile icon={item().icon} />}
                heading={heading(item())}
                text={<Text style={[!!item().why && look.wrong]}>{about(item())}</Text>}
                current={open() === item().item}
                onPress={() => setOpen(item().item)}
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
    </Split>
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
