// The App Library: one page of every app you can reach, by category, with its search, Background Apps and an app's icon
// in it.
import { createMemo, createSignal, For, Show } from "solid-js"
import { ScrollView, Empty, Icon, Image, type Located, Pressable, type Rect, reducedMotion, Search, type Styles, Text, View } from "ui"
import { BAR, corner, TOP_ROW } from "./layout.ts"
import { type Category, type Listed, src } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, launcher, room, tapping, libraryScroll } from "./store.ts"
import { type Owner, badgeOf, findIn, type From, launch, opens, press } from "./owners.ts"
import { Count } from "./tiles.tsx"
import { holds } from "./holds.ts"

/// Whether Background Apps is open, which each device remembers. Its storage may be missing or refused, as in a
/// private window; then it starts closed.
const BACKGROUND = "home-screen:background-apps"

const [unfolded, setUnfolded] = createSignal(
  (() => {
    try {
      return localStorage.getItem(BACKGROUND) === "open"
    } catch {
      return false
    }
  })(),
)

const unfold = (open: boolean) => {
  setUnfolded(open)
  try {
    localStorage.setItem(BACKGROUND, open ? "open" : "closed")
  } catch {}
}

/// An App Library's categories, each only its apps with an entrypoint, or under Background Apps (`background`), only
/// its apps with none.
const sorted = (library: Category[], background: boolean) =>
  library.flatMap((category) => {
    const entrypoints = category.entrypoints.filter((listed) => !listed.entrypoint === background)
    return entrypoints.length ? [{ ...category, entrypoints }] : []
  })

/// A category of the App Library as it's opened, and its place there is registered: under Background Apps, apart from
/// the category of the same name above it.
const BEHIND = "background:"

export const openedCategory = (library: Category[], key: string) =>
  key.startsWith(BEHIND) ? sorted(library, true).find((category) => category.category === key.slice(BEHIND.length)) : sorted(library, false).find((category) => category.category === key)

/// How long app `index` of a category waits to fly, out of its square and back, for the ones before it, in ms, the third
/// and those after it the same: out of a square of two columns into a row, the app in the second row passes under the
/// one above its neighbour once that one has gone ahead, where all at once they cross.
export const wait = (index: number) => Math.min(index, 2) * 40

/// How a category's `count` apps are handed between its square and it: once they've all flown, or at once with less
/// motion, when they jump.
export const handed = (count: number) => [looks.handedAfter(1000 + wait(count - 1)), reducedMotion() && looks.handing]

/// Where each category's place was drawn as it was opened, which it grows out of and shrinks back into, and each of its
/// apps' icons in it (`apps`), which that app grows out of.
export const places: Record<string, Rect & { apps: Rect[] }> = {}

/// The category made ahead: the one a finger or the pointer last went down on, or last opened, its contents made and
/// kept, open or not, so opening it and closing it change how it's drawn, not what's made, and the frame a press lifts
/// is a frame.
export const [made, setMade] = createSignal<string>()

/// The App Library's search, its categories and what it found, laid out for the room it's in: categories as many
/// across as fit at an iPhone's size, two at a phone's least, as iOS's are, each its icons in a square, whose padding
/// and gaps follow its width. The gap between them is points, not the text's size, which a phone's larger text would
/// grow past what's left for it, wrapping each row after one.
function shelf(width: number) {
  const gap = 20
  const across = Math.max(2, Math.floor((width + gap) / (152 + gap)))
  // A row of them wraps after `across`: each a 64th of a pixel narrower at most than its share, so rounding never
  // wraps one early.
  const category = Math.floor(((width - (across - 1) * gap) / across) * 64) / 64
  const padding = { top: category * 0.072, inline: category * 0.066, bottom: category * 0.054 }
  const [columnGap, rowGap] = [category * 0.06, category * 0.036]
  const icon = (category - padding.inline * 2 - columnGap) / 2 - 18 - 4
  return { gap, category, padding, columnGap, rowGap, icon }
}

/// Where each of a category's `count` apps' icons is in its square, drawn at `square`: its first four in its two
/// columns and two rows, each in the middle of its column at its row's top, or with more than four, three and the next
/// four small in the fourth's place, and any after those in that place's middle, too small to see.
function icons({ category, padding, columnGap, rowGap, icon }: ReturnType<typeof shelf>, square: Rect, count: number): Rect[] {
  const drawn = square.width / category
  const at = (x: number, y: number, size: number) => ({ x: square.x + x * drawn, y: square.y + y * drawn, width: size * drawn, height: size * drawn })
  const column = (category - 2 * padding.inline - columnGap) / 2
  const row = (category - padding.top - padding.bottom - rowGap) / 2
  const place = (index: number) => [padding.inline + (index % 2) * (column + columnGap), padding.top + Math.floor(index / 2) * (row + rowGap)]
  // The fourth place's small icons, in two rows of two, in the middle of it.
  const [more, small] = [icon + 14, (icon + 9) / 2]
  const [left, top] = place(3).map((start, along) => start + ((along ? row : column) - more) / 2)
  return Array.from({ length: count }, (_, index) => {
    if (count <= 4 || index < 3) {
      const [x, y] = place(index)
      return at(x + (column - icon) / 2, y, icon)
    }
    const nth = index - 3
    if (nth >= 4) return at(left + more / 2, top + more / 2, 0)
    return at(left + (nth % 2) * (more - small), top + Math.floor(nth / 2) * (more - small), small)
  })
}

/// The App Library: one page, which scrolls, as an iPhone's does, with every app you can reach once, whether it's
/// installed for you, a group or everyone: your own App Library, which has theirs. Its search is at its top; under it,
/// the categories of the apps with a screen of their own, as many across as fit, each its first four apps' icons with
/// their names in a square, or with more than that, three and the rest smaller in the fourth's place, and its name
/// under it. Tapping an icon opens its app; tapping anywhere else in a category opens it. Under them is Background
/// Apps, closed until it's tapped, as iOS's collapsible sections are: a row flush with the background whose chevron
/// turns as it opens the categories of background apps, those with no screen of their own, under it. Typing in the
/// search shows the apps whose names match, A to Z, in place of the categories, and the background apps that match
/// under Background Apps, open. Touching and holding an app gives its menu, and moving one with a screen carries it
/// onto Home's pages.
export function Library(props: { owner: Owner; size?: { width: number; height: number } }) {
  const [query, setQuery] = createSignal("")
  // Tapped while a search shows it open, Background Apps closes only for that search.
  const [shut, setShut] = createSignal(false)
  const searching = () => !!query().trim()
  const inline = () => (props.size ? 0 : room().wide ? room().side : 20)
  const width = () => props.size?.width ?? Math.min(1440, room().width - 2 * inline())
  const laid = createMemo(() => shelf(width()))
  const opening = (key: () => string, count: () => number) => (event: Located) => {
    if (!state().matches("nothing")) return
    ahead(key, count)(event)
    home.send({ type: "open", category: key() })
  }
  // Made as a finger or the pointer goes down on it, before it's let go of.
  const ahead = (key: () => string, count: () => number) => (event: Located) => {
    if (!state().matches("nothing")) return
    const square = event.currentTarget.getBoundingClientRect()
    places[key()] = { ...square, apps: icons(laid(), square, count()) }
    setMade(key())
  }
  const found = (background: boolean) => findIn(props.owner.library, query(), background)
  const apps = () => sorted(props.owner.library, false)
  const background = () => sorted(props.owner.library, true)
  const open = () => (searching() ? !shut() : unfolded())
  // Components, not JSX made again whenever the App Library's asked again: its rows stay, each with its icon.
  const Categories = (inner: { list: Category[]; prefix: string }) => (
    <View style={looks.categories(laid().gap)}>
      <For each={inner.list} keyed={(category) => category.category}>
        {(category) => {
          const key = () => inner.prefix + category().category
          const count = () => category().entrypoints.length
          const shown = () => (count() > 4 ? category().entrypoints.slice(0, 3) : category().entrypoints)
          // Its apps are in it opened while it's open, and on their way back.
          const away = () => state().context.category === key()
          return (
            <View style={[looks.category, looks.wide(laid().category)]}>
              <Pressable
                role="group"
                aria-label={category().category}
                onPointerDown={ahead(key, count)}
                onPress={opening(key, count)}
                style={[looks.folder, looks.folderBox(laid().columnGap, laid().rowGap, laid().category, laid().padding.top, laid().padding.inline, laid().padding.bottom)]}
              >
                <For each={shown()} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                  {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} icon={laid().icon} small away={away()} handed={handed(count())} />}
                </For>
                <Show when={count() > 4}>
                  <Pressable role="button" aria-label={`All of ${category().category}`} onPress={opening(key, count)} style={[looks.more, looks.square(laid().icon + 14), handed(count()), away() && [looks.handing, looks.unseen]]}>
                    <For each={category().entrypoints.slice(3, 7)} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                      {(entrypoint) => <Image src={src(entrypoint().icon)} alt="" style={[looks.iconImage(corner((laid().icon + 9) / 2)), looks.square((laid().icon + 9) / 2)]} />}
                    </For>
                  </Pressable>
                </Show>
              </Pressable>
              {/* Its apps take the focus, and its small icons, which open it, so its name doesn't again: it's for a finger. */}
              <Pressable onPress={opening(key, count)} style={looks.narrow}>
                <Text style={looks.categoryName}>{category().category}</Text>
              </Pressable>
            </View>
          )
        }}
      </For>
    </View>
  )
  const Found = (inner: { list: Listed[] }) => (
    <View style={looks.found}>
      <For each={inner.list} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
        {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} icon={60} />}
      </For>
    </View>
  )
  return (
    <View role="region" aria-label="App Library" style={[looks.library, looks.sized(props.size?.width ?? room().width, props.size?.height ?? room().height)]}>
      <Show when={!launcher()}>
        <View style={[looks.centred, looks.padded(room().safe.top + (TOP_ROW - BAR) / 2, (TOP_ROW - BAR) / 2 - 12)]}>
          <View style={looks.wide(Math.min(room().width - 2 * BAR - 16, 384))}>
            <Search placeholder="App Library" aria-label="Search the App Library" value={query()} onChangeText={(text) => (setQuery(text), setShut(false))} />
          </View>
        </View>
      </Show>
      <ScrollView onScroll={(event) => (libraryScroll.scrollTop = event.currentTarget.scrollTop)} style={[looks.libraryScroll, looks.libraryPadding(inline(), props.size ? 0 : 12, props.size ? 0 : 96 + room().safe.bottom)]}>
        <View style={[looks.libraryColumn, looks.wide(width())]}>
          <Show when={searching() ? found(false).length || found(true).length : props.owner.library.length} fallback={<Empty>{searching() ? "No apps match." : "No apps yet."}</Empty>}>
            <Show when={searching()} fallback={<Show when={apps().length}><Categories list={apps()} prefix="" /></Show>}>
              <Show when={found(false).length}><Found list={found(false)} /></Show>
            </Show>
            <Show when={searching() ? found(true).length : background().length}>
              <View role="region" aria-label="Background Apps" style={looks.background}>
                <Pressable
                  role="button"
                  aria-expanded={open() ? "true" : "false"}
                  onPress={() => (searching() ? setShut(!shut()) : unfold(!unfolded()))}
                  style={looks.fold}
                >
                  <Text style={looks.foldName}>Background Apps</Text>
                  <View style={[looks.chevron, looks.turned(open() ? 90 : 0)]}>
                    <Icon name="caret-right" along />
                  </View>
                </Pressable>
                <Show when={open()}>
                  <Show when={searching()} fallback={<Categories list={background()} prefix={BEHIND} />}>
                    <Found list={found(true)} />
                  </Show>
                </Show>
              </View>
            </Show>
          </Show>
        </View>
      </ScrollView>
    </View>
  )
}

/// An app's icon in the App Library, `icon` across: a tap opens it, or for an app with no entrypoint, the App Store
/// at its page, and a finger that holds it opens its menu; moving it carries it onto whichever owner's pages it's
/// dropped on. In a category's square (`small`) its name is smaller, and its icon is handed over to its category opened
/// as that says (`handed`), and not drawn while it's in it (`away`); its name stays in the square, going as the category
/// opens and coming back as its apps fly home, as the opened one's go and come, so nothing changes as they're handed back.
function LibraryEntrypoint(props: { entrypoint: Listed; owner: Owner; icon: number; small?: boolean; away?: boolean; handed?: Styles }) {
  const badge = () => (opens(props.entrypoint) ? badgeOf(opens(props.entrypoint)!, props.owner) : undefined)
  const from = (event: Located): From => {
    const at = event.currentTarget.getBoundingClientRect()
    return { x: at.x + (at.width - props.icon) / 2, y: at.y, width: props.icon, height: props.icon, radius: props.icon * 0.225 }
  }
  const held = holds(() => ({ item: opens(props.entrypoint), listed: props.entrypoint, page: 0, index: -1, from: "library" }), () => true)
  return (
    <Pressable
      role="button"
      aria-label={props.entrypoint.name}
      aria-description={badge() ? `${badge()} waiting` : undefined}
      {...held}
      onPointerDown={(event) => {
        if (state().matches("nothing") && opens(props.entrypoint)) launch(opens(props.entrypoint)!, props.owner, true)
        held.onPointerDown?.(event)
      }}
      onPress={(event) => {
        if (tapping() && state().matches("nothing")) press(props.entrypoint, props.owner, false, from(event))
      }}
      style={[looks.entry, props.small && looks.entrySmall]}
    >
      <View style={[looks.slots, looks.square(props.icon), props.handed, props.away && [looks.handing, looks.unseen]]}>
        <Image src={src(props.entrypoint.icon)} alt="" style={[looks.iconImage(corner(props.icon)), looks.square(props.icon)]} />
        <Count count={badge()} />
      </View>
      <Text style={[looks.name, props.small && [looks.smallName, props.away ? looks.unnamed : looks.named]]}>{props.entrypoint.name}</Text>
    </Pressable>
  )
}

