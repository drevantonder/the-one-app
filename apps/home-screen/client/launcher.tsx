// The computer's launcher: its keys, its search floating over its card, and what the search found.
import { For, Show } from "solid-js"
import { Empty, type Keyed, leave, Text, TextInput, View } from "ui"
import { type Item, type Listed } from "./types.ts"
import { looks } from "./looks.ts"
import { state, query, setQuery, chosenResult, setChosenResult } from "./store.ts"
import { type Owner, badgeOf, opens, press } from "./owners.ts"
import { Card, Tile } from "./tiles.tsx"

/// The launcher's keys, wherever in it the keyboard is: with nothing typed, the arrow keys move the highlight over Home
/// (`move`), Return opens what's highlighted, and Escape goes back from the App
/// Library, then puts the launcher away; with something typed, the arrow keys choose among what's found, Return opens
/// the one chosen, and Escape empties the search.
export const launcherKeys = (props: { move: (key: string) => boolean; open: () => void; toggleLibrary: () => void; library: () => boolean; found: () => number }) => (event: Keyed) => {
  if (event.altKey || event.metaKey || event.ctrlKey) return
  const typed = !!query().trim()
  // What's open over it (a category, a held app's menu, the owners' list) has Escape and the arrows itself.
  if (!state().matches("nothing")) return
  if (event.key === "Enter") {
    event.preventDefault()
    props.open()
  } else if (event.key === "Escape") {
    // With something typed, the field empties itself.
    if (query()) return
    if (props.library()) props.toggleLibrary()
    else void leave()
  } else if (!typed) {
    if (props.move(event.key)) event.preventDefault()
  } else {
    const by = ({ ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 } as Record<string, number>)[event.key]
    if (!by) return
    event.preventDefault()
    setChosenResult((now) => Math.max(0, Math.min(props.found() - 1, now + by)))
  }
}

/// The launcher's search, floating over its card: bare and large, with the keyboard as the launcher's summoned.
export function LauncherSearch() {
  return (
    <View style={looks.launcherSearch}>
      <TextInput inputMode="search" enterKeyHint="search" placeholder="Search apps…" aria-label="Search apps" autoFocus value={query()} onChangeText={(text) => (setQuery(text), setChosenResult(0))} style={looks.bare} />
    </View>
  )
}

/// What the launcher's search found, in its card, as the Home Screen draws apps and widgets: under Apps, the widget of
/// the app found first, where it has one, and beside it every app found, chosen for Return in turn; then, under
/// Widgets, the other widgets whose name or app has what's typed. The pointer opens any of them.
export function Results(props: { owner: Owner; found: Listed[]; words: string }) {
  const tile = 60
  const lead = () => leadOf(props.owner, props.found)
  const widgets = () => widgetsOf(props.owner, props.found, props.words)
  const Heading = (inner: { children: string }) => <Text style={[looks.heading, looks.tall(HEADING)]}>{inner.children}</Text>
  return (
    <View style={looks.results}>
      <Show when={props.found.length || widgets().length} fallback={<View style={looks.nothingFound}><Empty>No apps match.</Empty></View>}>
        <Show when={props.found.length}>
          <Heading>Apps</Heading>
          <View style={looks.apps}>
            <Show when={lead()}>
              {(lead) => (
                <View style={[looks.lead, looks.sized(2 * 76 + 8, 2 * 88 - 8)]}>
                  <Card item={{ ...lead(), size: "2x2" }} owner={props.owner} />
                </View>
              )}
            </Show>
            <View style={looks.foundApps}>
              <For each={props.found} keyed={(listed) => listed.entrypoint ?? listed.app}>
                {(listed, at) => (
                  <View onPointerMove={() => setChosenResult(at())} style={looks.result}>
                    <Show when={chosenResult() === at()}>
                      <View aria-hidden="true" style={looks.chosen} />
                    </Show>
                    <Tile name={listed().name} icon={listed().icon} size={tile} scale={1} badge={opens(listed()) ? badgeOf(opens(listed())!, props.owner) : undefined} onPress={(behind, from) => press(listed(), props.owner, behind, from)} />
                  </View>
                )}
              </For>
            </View>
          </View>
        </Show>
        <Show when={widgets().length}>
          <Heading>Widgets</Heading>
          <View style={looks.widgets}>
            <For each={widgets()} keyed={(one) => one.widget}>
              {(one) => (
                <View style={[looks.widgetCard, looks.sized(2 * 76 + 8, 2 * 88 - 8)]}>
                  <Card item={{ ...one(), size: "2x2" }} owner={props.owner} />
                </View>
              )}
            </For>
          </View>
        </Show>
      </Show>
    </View>
  )
}

/// The widget of the app a search found first, where it has one, which leads what it found; and the other widgets whose
/// name or app has what's typed.
const leadOf = (owner: Owner | undefined, found: Listed[]) => owner?.widgets.find((one) => one.opens?.app === found[0]?.app && one.sizes.includes("2x2"))

const widgetsOf = (owner: Owner | undefined, found: Listed[], words: string) => {
  const typed = words.trim().toLowerCase()
  const lead = leadOf(owner, found)
  return (owner?.widgets ?? []).filter((one) => one !== lead && `${one.name} ${one.opens?.name ?? ""}`.toLowerCase().includes(typed))
}

const HEADING = 20

/// What the launcher's Return opens: an app it found, or what's chosen on Home, with where it is on Home, if it's there.
export type Target = { item?: Item; listed?: Listed; place?: { page: number; index: number } }
