// Recently Deleted: what you deleted in the last 30 days, the latest first, each with how long until it's deleted for
// good. Restore or Delete Now from a row's menu, and Delete All for everything.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, leave, List, Menu, Problem, Row, Screen, Skeleton, Spinner, toast } from "ui"
import "./app.css"

/** A deleted item, as `soft-delete_list` answers it. */
interface Item {
  item: string
  state: "deleted" | "purging"
  artifacts: string[]
  plugin: string | null
  app: string
  installed: boolean
  purges: number
  why: string | null
}

const items = createMemo(() => call("soft-delete_list").then((text): Item[] => JSON.parse(text).deleted))
changed(() => refresh(items))

/// An artifact's name as a person reads it: users/alice/made/git-repos/notes is notes.
const named = (artifact: string) => artifact.split("/").pop()!
/// What a person calls an item: an uninstalled app's name, else its one artifact's.
const heading = (item: Item) => (item.plugin ? item.app : named(item.artifacts[0]))
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
  const needs = item.plugin ? along(item) : item.installed ? item.app : `Install ${item.app} to open it`
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
  else if (!restored.installed) toast(`${heading(item)} is back. Install ${restored.app} to open it.`)
}

async function purge(item: Item) {
  const question = { heading: `Delete ${heading(item)} for good?`, text: "It can't be restored.", action: "Delete", destructive: true }
  if (await confirm(question)) await doing("soft-delete_purge", { item: item.item })
}

async function purgeAll() {
  const question = { heading: "Delete everything for good?", text: "None of it can be restored.", action: "Delete All", destructive: true }
  if (await confirm(question)) await doing("soft-delete_purge_all")
}

function Deleted(props: { items: Item[] }) {
  const deleted = () => props.items.filter((item) => item.state === "deleted")
  return (
    <Screen heading="Recently Deleted" back onBack={leave} action={<Show when={deleted().length}><Button variant="destructive" onClick={purgeAll}>Delete All</Button></Show>}>
      <Show when={props.items.length} fallback={<Empty>Nothing was deleted in the last 30 days.</Empty>}>
        <p class="hint">Deleted for good after 30 days.</p>
        <List>
          <For each={props.items} keyed={(item) => item.item}>
            {(item) => (
              <Row
                heading={heading(item())}
                text={<span class={{ warn: !!item().why }}>{about(item())}</span>}
                end={
                  <Show when={item().state === "deleted"} fallback={<Spinner />}>
                    <Menu.Root onSelect={({ value }) => (value === "restore" ? restore(item()) : purge(item()))}>
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
          </For>
        </List>
      </Show>
    </Screen>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Recently Deleted" back onBack={leave}><Problem heading="Can't show what's deleted" text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading="Recently Deleted" back onBack={leave}><Skeleton rows={3} /></Screen>}>
        <Deleted items={items()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
