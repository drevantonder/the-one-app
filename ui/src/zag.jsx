// Zag (zagjs.com) in Ark UI's shape: a component is a namespace of parts (`Menu.Root`, `Menu.Trigger`, ...), whose
// root runs one of Zag's state machines (the keyboard, focus, selection and placement, with no look) and hands it
// to the parts inside, each of which spreads its part's attributes and listeners on its element. They're named and
// shaped as Ark's are, so that once Ark runs on Solid 2.0 each component imports its parts from
// `@ark-ui/solid/<component>` instead. The look is ui.css's, by the attribute Zag names each part with
// (`data-menu-content`), which Ark gives it too.
//
// None of Zag is needed to draw: a component looks the same before its machine is here. So ui.js holds no Zag. Each
// component's machine, with Zag's Solid adapter, is a file of its own (`zag/<component>.js`), loaded once the page
// has first drawn. Until then its parts are drawn still, from what they're given, and what opens over the page
// isn't drawn at all; then they're drawn again, alive.
import { createContext, createMemo, createSignal, createUniqueId, merge, omit, Show, useContext } from "solid-js"
import { Dynamic } from "@solidjs/web"
import { dir } from "./shared.ts"

/// Once the page has first been drawn. What it didn't need for that is loaded then, so the first screen waits for
/// none of it.
const drawn = new Promise((done) => requestAnimationFrame(() => setTimeout(done)))

/// What of `props` is told to a machine: the props `names` Zag lists for it, as they are now.
const told = (props, names) => Object.fromEntries(names.filter((name) => name in props).map((name) => [name, props[name]]))

/// What of a still root's props goes on its element: what can't be for its machine, which isn't here to say.
const own = (props) => Object.fromEntries(Object.keys(props).filter((name) => name === "class" || name === "style" || /^(aria|data)-/.test(name)).map((name) => [name, props[name]]))

/// What a still part is given for the api's function `get`: which part it is, for its look.
const part = (scope, get) => ({ [`data-${scope}${get.slice(3, -5).replace(/[A-Z]/g, (letter) => "-" + letter.toLowerCase())}`]: "" })

/// Solid's `class`, a list or an object as well as text, as text: Zag puts classes together as text.
const text = (value) =>
  typeof value === "string" || value == null ? value : Array.isArray(value) ? value.map(text).filter(Boolean).join(" ") : Object.keys(value).filter((name) => value[name]).join(" ")

/// A component's parts. `scope` is its name as Zag's attributes say it, and `load` loads its package. `root` is the
/// element its root draws and the api's function that gives its attributes, or nothing for a root that draws none
/// (a menu's). `list` has the other parts, each its element, the api's function that gives its attributes, and the
/// props it takes for that, which aren't put on the element: a list, or a function of the package for one of the
/// package's. `defaults`, called as the root is drawn, gives what the machine is told unless the root's props say
/// otherwise; `still`, given the root's props, is what's known of the api before the machine is here. → `Ark`, the
/// parts, `Root` among them; `use`, which gives a part the api; `live`, whether the machine is here; and `draw`,
/// which draws a part of the UI library's own, as `draw` below does, in the root it's in.
export function machine(scope, load, root, list, { defaults = () => () => ({}), still = () => ({}) } = {}) {
  const Api = createContext()
  const use = () => useContext(Api).api

  function Root(props) {
    const [zag, loaded] = createSignal()
    drawn.then(load).then(loaded)
    return (
      <Show when={zag()} keyed fallback={<Still props={props} />}>
        {(zag) => <Live zag={zag} props={props} />}
      </Show>
    )
  }

  function Live(given) {
    const { zag, props } = given
    const id = createUniqueId()
    const unless = defaults()
    const service = zag.useMachine(zag.machine, () => ({ id, dir: dir(), ...unless(), ...told(props, zag.props) }))
    const api = createMemo(() => zag.connect(service, zag.normalizeProps))
    const merged = (attributes, props) => zag.mergeProps(attributes, props)
    return (
      <Api value={{ api, merged, zag }}>
        {root ? draw(root[0], () => api()[root[1]](), omit(props, ...zag.props), merged) : props.children}
      </Api>
    )
  }

  function Still(given) {
    const { props } = given
    const known = still(props)
    const api = new Proxy({}, { get: (_, name) => (name in known ? known[name] : name.startsWith?.("get") ? () => part(scope, name) : undefined) })
    const merged = (attributes, props) => merge(attributes(), props)
    return (
      <Api value={{ api: () => api, merged }}>
        {root ? draw(root[0], () => api[root[1]](), merge(own(props), { children: props.children }), merged) : props.children}
      </Api>
    )
  }

  const made = { Root }
  for (const [name, [element, get, takes = []]] of Object.entries(list)) {
    made[name] = (props) => {
      const { api, merged, zag } = useContext(Api)
      // What opens over the page opens only once the machine is here to open it.
      if (!zag && name === "Positioner") return null
      const taking = zag ? (typeof takes === "function" ? takes(zag) : takes) : []
      return draw(element, () => api()[get](zag ? told(props, taking) : props), omit(props, ...taking), merged)
    }
  }
  return {
    Ark: made,
    use,
    live: () => !!useContext(Api).zag,
    draw: (element, attributes, props) => draw(element, attributes, props, useContext(Api).merged),
  }
}

/// Draws a part: `element` with the attributes Zag gives it (`attributes`, a function), and the part's own `props`
/// over them, `merged` as Zag merges them, so that both the part's listeners and Zag's are called.
export function draw(element, attributes, props, merged) {
  return <Dynamic component={element} {...merged(attributes, merge(omit(props, "children", "class"), { get class() { return text(props.class) } }))}>{props.children}</Dynamic>
}
