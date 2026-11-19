import { createSignal, flush, For, onSettled, Show, untrack } from "solid-js"
import { Button, ScrollView, confirm, covers, haptic, Icon, keydown, Pressable, type Pressed, View, viewport } from "ui"
import { glide, jump, spring, thrower } from "./pager.ts"
import { grid, TOP_ROW } from "./layout.ts"
import { pack, type Page, spans } from "./machine.ts"
import { looks } from "./looks.ts"
import { home, editing, room, windowWidth } from "./store.ts"
import { type Owner } from "./owners.ts"
import { Minus } from "./tiles.tsx"
import { type Cell, HomePage } from "./pages.tsx"

/// The page overview's thumbnails: a page laid out as a tall phone's, drawn this much smaller.
const MINIATURE = 136

const THUMB = grid(368, 654, 32, 20, 20, 0)

/// The page overview: every Home page small, in order, each with a tick for whether it's shown. Unticked, a page is
/// hidden and can be deleted; dragging one puts it elsewhere, following the finger, and taking another's place as it
/// passes over it, the others gliding aside to make room. Let go, it springs into its place. Done goes back to editing.
export function Overview(props: { owner: Owner; index: number; going?: boolean }) {
  const pages = () => editing()?.pages ?? []
  const [dragging, setDragging] = createSignal<Page>()
  // The one last dragged, which follows the finger and springs back from where it's let go, `moved` from its place.
  const [carried, setCarried] = createSignal<Page>()
  const [moved, setMoved] = createSignal({ x: 0, y: 0 })
  const remove = async (page: number) => {
    if (await confirm({ heading: "Delete Page?", text: "Its apps stay in the App Library.", action: "Delete", destructive: true })) home.send({ type: "delete", page })
  }
  // The thumbnails' grid: as many across as fit, each a miniature with its tick under it.
  const across = () => Math.max(1, Math.floor((windowWidth() - 48 + 16) / (MINIATURE + 16)))
  const pitch = () => ({ x: (windowWidth() - 48 - (across() - 1) * 16) / across() + 16, y: (MINIATURE * 16) / 9 + 12 + 28 + 24 })
  let down: (Pressed & { at: number; laid: { x: number; y: number } }) | undefined
  const moves = thrower()
  const place = (at: number) => ({ x: (at % across()) * pitch().x, y: Math.floor(at / across()) * pitch().y })
  const follow = (event: Pressed) => {
    if (!down) return { x: 0, y: 0 }
    const now = place(pages().indexOf(dragging()!))
    const at = { x: event.clientX - down.clientX - (now.x - down.laid.x), y: event.clientY - down.clientY - (now.y - down.laid.y) }
    setMoved(at)
    return at
  }
  const drag = (page: Page, index: number) => ({
    onPointerDown: (event: Pressed) => {
      if (event.button) return
      down = { ...event, at: index, laid: place(index) }
      moves.reset()
      setMoved({ x: 0, y: 0 })
      setDragging(page)
      setCarried(page)
    },
    onPointerMove: (event: Pressed) => {
      if (!down || event.pointerId !== down.pointerId) return
      follow(event)
      moves.add(event)
      // The thumbnail it's over, by where its middle is.
      const box = event.currentTarget.getBoundingClientRect()
      const [column, row] = [Math.floor((box.x + box.width / 2 - 24 + 8) / pitch().x), Math.floor((box.y + box.height / 2 - (room().safe.top + TOP_ROW + 16) + 12) / pitch().y)]
      const to = Math.max(0, Math.min(pages().length - 1, row * across() + Math.max(0, Math.min(across() - 1, column))))
      const at = pages().indexOf(page)
      if (at < 0 || to === at) return
      home.send({ type: "reorder", from: at, to })
      flush()
      haptic("selection")
      follow(event)
    },
    onPointerUp: (event: Pressed) => end(event),
    onPointerCancel: (event: Pressed) => end(event),
  })
  const end = (event: Pressed) => {
    if (!down || event.pointerId !== down.pointerId) return
    down = undefined
    const at = untrack(moved)
    const thrown = event.timeStamp ? moves.thrown(event.timeStamp) : { x: 0, y: 0 }
    const draw = () => setMoved({ x: x.value, y: y.value })
    const [x, y] = [spring(draw), spring(draw)]
    jump(x, at.x)
    jump(y, at.y)
    glide(x, 0, thrown.x)
    glide(y, 0, thrown.y)
    setTimeout(() => setDragging(), 0)
  }
  onSettled(() => {
    const shut = covers(() => home.send({ type: "dismiss" }))
    const keys = keydown((event) => event.key === "Escape" && home.send({ type: "dismiss" }))
    return () => (shut(), keys())
  })
  const scale = MINIATURE / THUMB.width
  return (
    // Over the blurred Home Screen, it fades out as it closes; its miniatures float, with a window's shadow.
    <View role="dialog" aria-label="Pages" style={[looks.overview, props.going && looks.gone, looks.at(0, 0, windowWidth(), viewport().height)]}>
      <View style={[looks.overviewBar, looks.topRowAt(room().safe.top, room().safe.top + TOP_ROW)]}>
        <Button variant="secondary" onPress={() => home.send({ type: "dismiss" })}>Done</Button>
      </View>
      <ScrollView style={[looks.overviewScroll, looks.padBottom(48 + room().safe.bottom)]}>
        {/* Each in its place by where it is in the pages, so that, as one's dragged past the others, they glide to theirs. */}
        <View style={[looks.slots, looks.tall(Math.ceil(pages().length / across()) * pitch().y - 24)]}>
          <For each={pages()}>
            {(page, index) => (
              <View
                style={[
                  looks.slot,
                  carried() === page ? looks.carried : looks.gliding,
                  looks.wide(pitch().x - 16),
                  looks.moved(place(index()).x + (carried() === page ? moved().x : 0), place(index()).y + (carried() === page ? moved().y : 0)),
                ]}
              >
                <View
                  role="button"
                  aria-label={`Page ${index() + 1}`}
                  {...drag(page, index())}
                  style={[looks.miniature, looks.scaled(MINIATURE, (MINIATURE * 16) / 9, dragging() === page ? 1.05 : 1)]}
                >
                  <View style={[looks.thumb, looks.thumbAt((MINIATURE - THUMB.width) / 2, ((MINIATURE * 16) / 9 - THUMB.height) / 2, scale, page.hidden ? 0.4 : 1)]}>
                    <HomePage owner={props.owner} index={props.index} page={index()} placed={(pack(page.items.map((item, at): Cell => ({ item, index: at })), (cell) => spans(cell.item))[0] ?? [])} laid={THUMB} scale={scale} small />
                  </View>
                  <Show when={page.hidden}>
                    <Minus label={`Delete page ${index() + 1}`} left={6} top={6} onPress={() => remove(index())} />
                  </Show>
                </View>
                <Pressable
                  role="checkbox"
                  aria-checked={page.hidden ? "false" : "true"}
                  aria-label={`Show page ${index() + 1}`}
                  onPress={() => home.send({ type: "hide", page: index(), hidden: !page.hidden })}
                  style={[looks.check, page.hidden && looks.unchecked]}
                >
                  <Show when={!page.hidden}>
                    <Icon name="check" />
                  </Show>
                </Pressable>
              </View>
            )}
          </For>
        </View>
      </ScrollView>
    </View>
  )
}
