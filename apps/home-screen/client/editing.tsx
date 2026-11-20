// What editing a Home Screen opens over it: the widget gallery, the sheets that ask to install or say no, and + and
// Done.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show, untrack } from "solid-js"
import { Button, ScrollView, call, Empty, Icon, Image, List, Pressable, Problem, type Rect, Row, Sheet, Text, toast, View } from "ui"
import { PILLS, TOP_ROW } from "./layout.ts"
import { type Held } from "./machine.ts"
import { type Entrypoint, type Size, type Widget as WidgetItem, src } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, room, laidPage } from "./store.ts"
import { json, type Owner, owners, and, installedFor, named } from "./owners.ts"
import { Card } from "./tiles.tsx"

const SIZES: Record<Size, string> = { "2x2": "Small", "4x2": "Medium", "4x4": "Large" }

/// How large a widget is drawn in the gallery, small, as it would be.
const PREVIEW: Record<Size, [number, number]> = { "2x2": [120, 120], "4x2": [248, 120], "4x4": [248, 248] }

/// The widget gallery: each widget the owner's apps have, and once one's picked, its sizes, each drawn as it would be,
/// with Add Widget, which places it first on the page shown.
export function Gallery(props: { owner: Owner }) {
  const [picked, setPicked] = createSignal<WidgetItem>()
  const [size, setSize] = createSignal<Size>()
  const close = () => (setPicked(), setSize(), home.send({ type: "dismiss" }))
  const add = () => {
    const widget = picked()!
    home.send({ type: "place", item: { ...widget, size: size() ?? widget.sizes[0] } })
    setPicked()
    setSize()
  }
  return (
    <Sheet heading={picked()?.name ?? "Widgets"} open={state().matches({ editing: "gallery" })} onClose={close}>
      <Show
        when={picked()}
        fallback={
          <Show when={props.owner.widgets.length} fallback={<Empty>No widgets yet.</Empty>}>
            <List each={props.owner.widgets} keyed={(widget) => widget.widget}>
              {(widget) => <Row heading={widget().name} start={widget().opens ? <Image src={src(widget().opens!.icon)} alt="" style={looks.widgetIcon} /> : undefined} onPress={() => (setPicked(widget()), setSize(widget().sizes[0]))} />}
            </List>
          </Show>
        }
      >
        {(widget) => (
          <View style={looks.sizes}>
            <ScrollView role="radiogroup" aria-label="Size" style={looks.sizesRow}>
              <For each={widget().sizes}>
                {(one) => (
                  <Pressable role="radio" aria-checked={size() === one ? "true" : "false"} aria-label={SIZES[one]} onPress={() => setSize(one)} style={[looks.sizeOption, size() === one && looks.sizeChosen]}>
                    <View style={[looks.preview, size() === one && looks.previewChosen, looks.sized(PREVIEW[one][0], PREVIEW[one][1])]}>
                      <Card item={{ ...widget(), size: one }} owner={props.owner} still />
                    </View>
                    <Text style={looks.sizeName}>{SIZES[one]}</Text>
                  </Pressable>
                )}
              </For>
            </ScrollView>
            <Button onPress={add}>Add Widget</Button>
          </View>
        )}
      </Show>
    </Sheet>
  )
}

/// What's carried and stacked on it, each as a person reads its name: an app from the App Library by its app's.
const carriedApps = (held: Held) => [held.item, ...(held.stack ?? [])].flatMap((item) => (item && !item.widget ? [{ item: item as Entrypoint, title: item === held.item ? (held.listed?.title ?? item.name) : item.name }] : []))

/// Apps dropped on an owner they aren't all installed for, who you may install for, wait where they were dropped while
/// one sheet asks about the ones that aren't: Install, as the App Store installs, for them, one after another, their
/// progress shown where they wait, and they stay; or Cancel, and they're back where they came from. If one couldn't be
/// installed they're all back, and a toast says why.
export function Asking(props: { owner: Owner; held: Held }) {
  const missing = () => carriedApps(props.held).filter(({ item }) => !installedFor(item, props.owner))
  const titles = () => and(missing().map(({ title }) => title))
  const install = async () => {
    const [owner, apps] = [props.owner, untrack(missing)]
    home.send({ type: "install" })
    for (const { item, title } of apps) {
      try {
        // Named as it is where it was taken from, so the entrypoint it's placed as is its own for them too.
        const done = JSON.parse(await call("install", { app: item.app, name: item.entrypoint.split("/")[0] }, { subject: owner.subject }))
        if (done.also?.length) toast(done.also.join(" "))
      } catch (error) {
        toast(`Couldn't install ${title} for ${named(owner)}. ${(error as Error).message ?? error}`)
        home.send({ type: "failed" })
        return void refresh(owners)
      }
    }
    home.send({ type: "installed" })
    refresh(owners)
  }
  const reach = () => {
    const it = missing().length > 1 ? "them" : "it"
    return props.owner.subject === "network" ? `Everyone in the network will have ${it}.` : props.owner.subject ? `Everyone in ${props.owner.name} will have ${it}.` : `${missing().length > 1 ? "They'll" : "It'll"} be yours.`
  }
  return (
    <Sheet heading={`Install ${titles()} for ${named(props.owner)}?`} open={state().matches({ editing: "asking" })} onClose={() => state().matches({ editing: "asking" }) && home.send({ type: "cancel" })}>
      <View style={looks.ask}>
        <View style={looks.askIcons}>
          <For each={missing()} keyed={({ item }) => item.entrypoint}>
            {(app) => <Image src={src(app().item.icon)} alt="" style={looks.askIcon} />}
          </For>
        </View>
        <Text style={looks.askWhy}>{reach()}</Text>
        <View style={looks.askButtons}>
          <Button onPress={install}>Install</Button>
          <Button variant="secondary" onPress={() => home.send({ type: "cancel" })}>Cancel</Button>
        </View>
      </View>
    </Sheet>
  )
}

/// An app dropped on an owner whose pages are someone else's to change: a sheet says so, and who can, and it goes back.
export function Refused(props: { owner: Owner; held: Held }) {
  const title = () => and(carriedApps(props.held).map(({ title }) => title))
  const who = createMemo(async () => {
    const [{ users }, { people }] = await Promise.all([
      json<{ users: string[] }>("permissions_users", { artifact: props.owner.subject, relation: "owner" }),
      json<{ people: { who: string; called?: string }[] }>("sign-in_everyone"),
    ])
    const called = (who: string) => people.find((person) => person.who === who)?.called ?? who.replace(/^users\//, "")
    return new Intl.ListFormat("en", { type: "disjunction" }).format(users.map(called))
  })
  const dismiss = () => state().matches({ editing: "refused" }) && home.send({ type: "dismiss" })
  return (
    <Sheet heading={`You can't add apps for ${props.owner.name}`} open={state().matches({ editing: "refused" })} onClose={dismiss}>
      <View style={looks.ask}>
        <Errored fallback={(error) => <Problem heading="Can't show who can" text={error() as Error} />}>
          <Loading fallback={<Text> </Text>}>
            <Text style={looks.askWhy}>{who() ? `Only ${who()} can. Ask them to add ${title()}.` : `Ask one of its owners to add ${title()}.`}</Text>
          </Loading>
        </Errored>
        <View style={looks.askButton}>
          <Button onPress={dismiss}>OK</Button>
        </View>
      </View>
    </Sheet>
  )
}

/// While pages are edited, + and Done float at the top, either side of the owner pill, over the pages, which stay
/// where they are, as on a phone; in the launcher, in its footer's place, each in Liquid Glass. They fade in, and out
/// as editing ends.
export function EditBar(props: { going?: boolean; within?: Rect }) {
  // In the launcher, in the owner switcher's row at the top of its card.
  const top = () => props.within?.y ?? room().safe.top
  const height = () => (props.within ? PILLS : TOP_ROW)
  const width = () => props.within?.width ?? (room().wide ? Math.min(room().width - 2 * room().side, room().fit * (laidPage().width * room().scale + room().gap) - room().gap) : room().width)
  const left = () => props.within?.x ?? (room().width - width()) / 2
  return (
    <View style={[looks.editBar, looks.at(left(), top(), width(), height()), looks.editBarPad(props.within ? 10 : 16), looks.fadeIn, props.going && looks.gone]}>
      <View style={[looks.pill, looks.glass]}>
        <Button variant="ghost" size="icon" aria-label="Add Widget" onPress={() => state().matches({ editing: "still" }) && home.send({ type: "gallery" })}>
          <Icon name="plus" />
        </Button>
      </View>
      <View style={[looks.pill, looks.glass]}>
        <Button variant="ghost" onPress={() => home.send({ type: "done" })}>Done</Button>
      </View>
    </View>
  )
}
