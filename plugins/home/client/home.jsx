// The Home Screen: yours, each of your groups' and the network's, side by side, then Settings. Each is a grid of
// the widgets put on it and its apps' icons. Swiping moves between them, and so does sliding along the dots.
// Touch and hold, or right-click, for what an app or a widget offers; hold and move to put it somewhere else.
import { createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { call, changed, confirm, Empty, Icon, List, open, Problem, Row, Section, Sheet, toast, Widget } from "ui"
import "./home.css"

const SIZES = { "2x2": "Small", "4x2": "Wide", "4x4": "Large" }
const json = (tool, inputs, who) => call(tool, inputs, { for: who }).then(JSON.parse)

/// Every page this user sees, and the parts of Settings. A network with nothing that keeps groups has none.
const home = createMemo(async () => {
  const groups = await json("relations_groups").catch(() => [])
  // Each page is named for whose it is: a group by what it's called.
  const whos = [{ name: "You" }, ...groups.map(({ group, called }) => ({ for: group, name: called })), { for: "network", name: "Everyone" }]
  const [settings, ...pages] = await Promise.all([json("home_settings"), ...whos.map((who) => json("home_page", {}, who.for).then((page) => ({ ...page, ...who })))])
  return { pages, settings }
})
changed(() => refresh(home))

/// An item of a page as the page is arranged with it.
const kept = (item) => (item.widget ? { widget: item.widget, size: item.size } : { app: item.app })
const same = (a, b) => (a.widget ?? a.app) === (b.widget ?? b.app) && a.size === b.size

/// Arranges a page as `items`, and says why if it couldn't be.
async function arrange(page, items) {
  try {
    await call("home_arrange", { items: items.map(kept) }, { for: page.for })
  } catch (error) {
    toast(error.message)
  }
  refresh(home)
}

/// What's being held: `{ page, item }`, once a finger has stayed on it, and whether it has moved since.
const [menu, setMenu] = createSignal()
const [adding, setAdding] = createSignal()
let quiet = 0

/// Touch and hold, or right-click, opens what `item` offers (with none, what its page does), and holding and
/// moving puts it where it's let go. A mouse drags at once.
function held(page, item) {
  return (element) => {
    let timer, holding, start, moved
    const over = (event) => document.elementsFromPoint(event.clientX, event.clientY).find((under) => under !== element && under.dataset?.item && under.parentElement === element.parentElement)
    const drop = () => {
      clearTimeout(timer)
      element.style.translate = ""
      element.classList.remove("lifted")
      for (const target of element.parentElement?.querySelectorAll(".target") ?? []) target.classList.remove("target")
      holding = start = undefined
    }
    element.addEventListener("contextmenu", (event) => {
      event.preventDefault()
      event.stopPropagation()
      if (!moved && page().arranges) setMenu({ page: page(), item: item?.() })
    })
    if (!item) return
    element.addEventListener("pointerdown", (event) => {
      if (event.button || !page().arranges) return
      start = event
      moved = false
      holding = event.pointerType === "mouse"
      if (!holding) timer = setTimeout(() => ((holding = true), element.classList.add("lifted")), 400)
    })
    element.addEventListener("pointermove", (event) => {
      if (!start) return
      const [x, y] = [event.clientX - start.clientX, event.clientY - start.clientY]
      if (!holding) return void (Math.hypot(x, y) > 10 && drop())
      if (!moved && Math.hypot(x, y) < 8) return
      moved = true
      element.setPointerCapture(event.pointerId)
      element.classList.add("lifted")
      element.style.translate = `${x}px ${y}px`
      for (const target of element.parentElement.querySelectorAll(".target")) target.classList.remove("target")
      over(event)?.classList.add("target")
    })
    element.addEventListener("pointerup", (event) => {
      const [was, target] = [holding && !moved && start?.pointerType !== "mouse", moved && over(event)]
      if (moved || was) quiet = Date.now() + 400
      drop()
      if (was) return setMenu({ page: page(), item: item() })
      if (!target) return
      const items = page().items.filter((other) => other !== item())
      items.splice(items.findIndex((other) => (other.widget ?? other.app) + (other.size ?? "") === target.dataset.item), 0, item())
      arrange(page(), items)
    })
    element.addEventListener("pointercancel", drop)
    // A web view drags an icon as a picture of itself, and cancels the pointer, unless it's told not to.
    element.addEventListener("dragstart", (event) => event.preventDefault())
    // A finger that's holding something moves it, not the page.
    element.addEventListener("touchmove", (event) => holding && event.preventDefault(), { passive: false })
  }
}

/// A widget on a page: what its plugin's tool answers, drawn, and asked again each minute it's in view.
function Card(props) {
  const tool = () => props.item.widget.replace("/", "_")
  const layout = createMemo(() => json(tool(), { size: props.item.size }, props.page.for))
  onSettled(() => {
    const stop = changed(() => refresh(layout))
    const every = setInterval(() => document.visibilityState === "visible" && refresh(layout), 60_000)
    return () => (stop(), clearInterval(every))
  })
  const act = async (name, inputs) => {
    try {
      await call(`${props.item.widget.split("/")[0]}_${name}`, inputs, { for: props.page.for })
    } catch (error) {
      toast(error.message)
    }
    refresh(layout)
  }
  const opens = () => Date.now() > quiet && props.item.opens && open(props.item.opens, { for: props.page.for })
  return (
    <Errored fallback={(error) => <Widget name={props.item.name} size={props.item.size} problem={String(error().message ?? error())} />}>
      <Loading fallback={<Widget name={props.item.name} size={props.item.size} />}>
        <Widget name={props.item.name} size={props.item.size} layout={layout()} onCall={act} onOpen={props.item.opens ? opens : undefined} />
      </Loading>
    </Errored>
  )
}

/// An app's icon with its name under it. It starts as a finger lands on it, and shows as the finger lifts.
function Tile(props) {
  return (
    <button type="button" class="tile" onPointerDown={() => open(props.app.app, { for: props.for, behind: true })} onClick={() => Date.now() > quiet && open(props.app.app, { for: props.for })}>
      <img src={props.app.icon} alt="" />
      <span>{props.app.name}</span>
    </button>
  )
}

function Page(props) {
  return (
    <section class="page" ref={held(() => props.page)}>
      <Show when={props.page.items.length} fallback={<Empty>No apps yet.</Empty>}>
        <div class="grid">
          <For each={props.page.items} keyed={(item) => (item.widget ?? item.app) + (item.size ?? "")}>
            {(item) => (
              <div class="cell" data-item={(item().widget ?? item().app) + (item().size ?? "")} data-size={item().size} ref={held(() => props.page, item)}>
                <Show when={item().widget} fallback={<Tile app={item()} for={props.page.for} />}>
                  <Card item={item()} page={props.page} />
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  )
}

/// What a held app or widget offers, or with nothing held, its page: a widget's sizes, taking it off, and the
/// widgets that could be put on.
function Offers(props) {
  const page = () => menu()?.page
  const item = () => menu()?.item
  const others = () => page().items.filter((other) => other !== item())
  const done = (does) => () => (setMenu(), does())
  const off = done(async () => {
    const plugin = item().app.split("/")[0]
    if (!(await confirm({ heading: `Remove ${item().name} from ${page().name}?`, text: "What it kept stays, and comes back with it.", action: "Remove", destructive: true }))) return
    await call("uninstall", { name: plugin }, { for: page().for }).catch((error) => toast(error.message))
    refresh(home)
  })
  return (
    <Sheet heading={item()?.name ?? page()?.name} open={!!menu()} onClose={() => setMenu()}>
      <Show when={menu()}>
        <List>
          <Show when={item()?.widget}>
            <For each={item().sizes}>
              {(size) => <Row heading={SIZES[size]} end={size === item().size ? <Icon name="check" /> : undefined} onClick={done(() => arrange(page(), page().items.map((other) => (other === item() ? { ...other, size } : other))))} />}
            </For>
            <Row heading="Remove Widget" onClick={done(() => arrange(page(), others()))} />
          </Show>
          <Show when={item()?.app}>
            <Row heading={`Remove from ${page().name}`} onClick={off} />
          </Show>
          <Show when={page().widgets.length}>
            <Row heading="Add Widget" onClick={done(() => setAdding(page()))} />
          </Show>
        </List>
      </Show>
    </Sheet>
  )
}

/// The widgets a page's plugins have, each in the sizes it comes in.
function Adding() {
  const add = (widget, size) => {
    const page = adding()
    setAdding()
    if (!page.items.some((item) => same(item, { ...widget, size }))) arrange(page, [{ ...widget, size }, ...page.items])
  }
  return (
    <Sheet heading="Add Widget" open={!!adding()} onClose={() => setAdding()}>
      <Show when={adding()}>
        <For each={adding().widgets}>
          {(widget) => (
            <Section heading={widget.name}>
              <List>
                <For each={widget.sizes}>{(size) => <Row heading={SIZES[size]} onClick={() => add(widget, size)} />}</For>
              </List>
            </Section>
          )}
        </For>
      </Show>
    </Sheet>
  )
}

function Home(props) {
  let strip
  const [at, setAt] = createSignal(0)
  const names = () => [...props.home.pages.map((page) => page.name), "Settings"]
  const go = (to, smooth = true) => strip.scrollTo({ left: Math.max(0, Math.min(names().length - 1, to)) * strip.clientWidth * (document.dir === "rtl" ? -1 : 1), behavior: smooth ? "smooth" : "instant" })

  onSettled(() => {
    const keys = (event) => {
      if (event.target.closest?.("input, textarea, dialog")) return
      if (event.key === "ArrowRight") go(at() + 1)
      else if (event.key === "ArrowLeft") go(at() - 1)
    }
    addEventListener("keydown", keys)
    return () => removeEventListener("keydown", keys)
  })

  /// A finger sliding along the dots changes page, one for every 28 points, as a phone's page dots do.
  const slide = (down) => {
    const [capsule, from] = [down.currentTarget, at()]
    let sliding = false
    const move = (event) => {
      const across = event.clientX - down.clientX
      if (!sliding && Math.abs(across) < 10) return
      sliding = true
      capsule.setPointerCapture(down.pointerId)
      go(from + Math.trunc(across / 28), false)
    }
    const up = () => {
      capsule.removeEventListener("pointermove", move)
      if (sliding) quiet = Date.now() + 300
    }
    capsule.addEventListener("pointermove", move)
    capsule.addEventListener("pointerup", up, { once: true })
    capsule.addEventListener("pointercancel", up, { once: true })
  }
  const press = (does) => () => Date.now() > quiet && does()

  return (
    <main class="home">
      <div class="strip" ref={strip} onScroll={() => setAt(Math.round(Math.abs(strip.scrollLeft) / strip.clientWidth))}>
        <For each={props.home.pages} keyed={(page) => page.who}>
          {(page) => <Page page={page()} />}
        </For>
        <section class="page">
          <div class="grid">
            <For each={props.home.settings} keyed={(part) => part.app}>
              {(part) => <div class="cell"><Tile app={part()} /></div>}
            </For>
          </div>
        </section>
      </div>
      <footer>
        <p>{names()[at()]}</p>
        <nav class="dots" onPointerDown={slide} aria-label="Pages">
          <For each={props.home.pages} keyed={(page) => page.who}>
            {(page, index) => <button type="button" class="dot" aria-label={page().name} aria-current={at() === index() ? "page" : undefined} onClick={press(() => go(index()))}><i /></button>}
          </For>
          <button type="button" aria-label="Settings" aria-current={at() === names().length - 1 ? "page" : undefined} onClick={press(() => go(names().length - 1))}><Icon name="gear-six" /></button>
        </nav>
      </footer>
      <Offers />
      <Adding />
    </main>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your Home Screen pages" text={String(error().message ?? error())} />}>
      <Loading>
        <Home home={home()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
