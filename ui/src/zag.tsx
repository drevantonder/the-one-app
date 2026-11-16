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
import { Dynamic, type JSX } from "@solidjs/web"
import type { Accessor, Component } from "solid-js"
import { dir } from "./shared.ts"

/** What Zag's `connect` gives: the api a component's parts read, each by its name (`getTriggerProps`, `open`). Which
 * names there are is the component's, so it's read as Zag's own JavaScript reads it. */
// biome-ignore lint: an api read by name
export type Api = Record<string, any>

/** A component's package with Zag's Solid adapter (`zag/<component>.ts`), once it's loaded. */
interface Zag {
  machine: unknown
  connect: (service: unknown, normalize: unknown) => Api
  normalizeProps: unknown
  mergeProps: (...props: (object | (() => object))[]) => object
  useMachine: (machine: unknown, props: () => object) => unknown
  props: string[]
  [list: string]: unknown
}

/** An element a part draws: a tag, or a component. */
type Element = string | Component<any>

/** A part of a component, Ark's shape: its element's attributes, and what Zag takes for it (`Takes`). */
export type Part<Takes = {}, Of extends HTMLElement = HTMLElement> = Component<Omit<JSX.HTMLAttributes<Of>, keyof Takes> & Takes>

/** A component's root: its machine's props, as Ark's root takes them, those it can't do without (`Needs`) needed. */
export type Root<Props, Needs extends keyof Props = never, Of extends HTMLElement = HTMLDivElement> = Part<Partial<Omit<Props, "id" | "dir" | "getRootNode">> & Pick<Props, Needs>, Of>

/** A part's props, as it's given them. */
type Props = Record<string, unknown> & { children?: JSX.Element; class?: JSX.ClassValue }

/** Zag's attributes for a part and the part's own props, as one. */
type Merged = (attributes: () => object, props: object) => object

/// Once the page has first been drawn. What it didn't need for that is loaded then, so the first screen waits for
/// none of it.
const drawn = new Promise((done) => requestAnimationFrame(() => setTimeout(done)))

/// What of `props` is told to a machine: the props `names` Zag lists for it, as they are now.
const told = (props: Record<string, unknown>, names: string[]) => Object.fromEntries(names.filter((name) => name in props).map((name) => [name, props[name]]))

/// What of a still root's props goes on its element: what can't be for its machine, which isn't here to say.
const own = (props: Record<string, unknown>) => Object.fromEntries(Object.keys(props).filter((name) => name === "class" || name === "style" || /^(aria|data)-/.test(name)).map((name) => [name, props[name]]))

/// What a still part is given for the api's function `get`: which part it is, for its look.
const part = (scope: string, get: string) => ({ [`data-${scope}${get.slice(3, -5).replace(/[A-Z]/g, (letter) => "-" + letter.toLowerCase())}`]: "" })

/// Solid's `class`, a list or an object as well as text, as text: Zag puts classes together as text.
const text = (value: JSX.ClassValue): string | null | undefined =>
  typeof value === "string" || value == null ? value : Array.isArray(value) ? value.map(text).filter(Boolean).join(" ") : typeof value === "object" ? Object.keys(value).filter((name) => value[name]).join(" ") : String(value)

/// A component's parts. `scope` is its name as Zag's attributes say it, and `load` loads its package. `root` is the
/// element its root draws and the api's function that gives its attributes, or nothing for a root that draws none
/// (a menu's). `list` has the other parts, each its element, the api's function that gives its attributes, and the
/// props it takes for that, which aren't put on the element: a list, or a function of the package for one of the
/// package's. `defaults`, called as the root is drawn, gives what the machine is told unless the root's props say
/// otherwise; `still`, given the root's props, is what's known of the api before the machine is here. → `Ark`, the
/// parts, `Root` among them; `use`, which gives a part the api; `live`, whether the machine is here; and `draw`,
/// which draws a part of the UI library's own, as `draw` below does, in the root it's in.
export function machine<Parts extends string, Given = Props>(
  scope: string,
  load: () => Promise<unknown>,
  root: [Element, string] | null,
  list: Record<Parts, [Element, string, (string[] | ((zag: Zag) => unknown))?]>,
  { defaults = () => () => ({}), still = () => ({}) }: { defaults?: () => () => object; still?: (root: Given) => Record<string, unknown> } = {},
) {
  const Api = createContext<{ api: Accessor<Api>; merged: Merged; zag?: Zag }>()
  const use = (): Accessor<Api> => useContext(Api)!.api

  function Root(props: Props) {
    const [zag, loaded] = createSignal<Zag>()
    drawn.then(load).then((zag) => loaded(() => zag as Zag))
    return (
      <Show when={zag()} keyed fallback={<Still props={props} />}>
        {(zag) => <Live zag={zag} props={props} />}
      </Show>
    )
  }

  function Live(given: { zag: Zag; props: Props }) {
    const { zag, props } = given
    const id = createUniqueId()
    const unless = defaults()
    const service = zag.useMachine(zag.machine, () => ({ id, dir: dir(), ...unless(), ...told(props, zag.props) }))
    const api = createMemo(() => zag.connect(service, zag.normalizeProps))
    const merged: Merged = (attributes, props) => zag.mergeProps(attributes, props)
    return (
      <Api value={{ api, merged, zag }}>
        {root ? draw(root[0], () => api()[root[1]](), omit(props, ...zag.props), merged) : props.children}
      </Api>
    )
  }

  function Still(given: { props: Props }) {
    const { props } = given
    const known = still(props as Given)
    const api = new Proxy<Api>({}, { get: (_, name) => (name in known ? known[name as string] : typeof name === "string" && name.startsWith("get") ? () => part(scope, name) : undefined) })
    const merged: Merged = (attributes, props) => merge(attributes(), props)
    return (
      <Api value={{ api: () => api, merged }}>
        {root ? draw(root[0], () => api[root[1]](), merge(own(props), { children: props.children }) as Props, merged) : props.children}
      </Api>
    )
  }

  const made = { Root } as Record<"Root" | Parts, Component<any>>
  for (const [name, [element, get, takes = []]] of Object.entries(list) as [Parts, (typeof list)[Parts]][]) {
    made[name] = (props: Props) => {
      const { api, merged, zag } = useContext(Api)!
      // What opens over the page opens only once the machine is here to open it.
      if (!zag && name === "Positioner") return null
      const taking = zag ? ((typeof takes === "function" ? takes(zag) : takes) as string[]) : []
      return draw(element, () => api()[get](zag ? told(props, taking) : props), omit(props, ...taking), merged)
    }
  }
  return {
    Ark: made,
    use,
    live: () => !!useContext(Api)!.zag,
    draw: (element: Element, attributes: () => object, props: object) => draw(element, attributes, props as Props, useContext(Api)!.merged),
  }
}

/// Draws a part: `element` with the attributes Zag gives it (`attributes`, a function), and the part's own `props`
/// over them, `merged` as Zag merges them, so that both the part's listeners and Zag's are called.
export function draw(element: Element, attributes: () => object, props: Props, merged: Merged): JSX.Element {
  return <Dynamic component={element} {...merged(attributes, merge(omit(props, "children", "class"), { get class() { return text(props.class) } }))}>{props.children}</Dynamic>
}
