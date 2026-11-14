// How an element gets its behaviour from Zag (zagjs.com): a state machine per element, with the keyboard, focus,
// selection and placement in it, and no look. The element draws the machine's parts and says how they look.
//
// None of it is needed to draw: an element looks the same before its machine is here. So ui.js holds no Zag. It
// comes in two parts, loaded by themselves: `zag-late.js` once the page has first drawn, and `zag-mouse.js`, the
// select list and the calendar, only on a device with a mouse: under a finger the device's own pickers open.
import { noChange } from "lit"
import { Directive, directive } from "lit/directive.js"
import { drawn } from "./element.js"

let made = 0
let vanilla
let mouse

/// Zag's first part, once it's loaded: its packages by name (`menu`, `tabs`, `numberInput`, `toast`).
export const late = drawn.then(() => import("./zag-late.js")).then((loaded) => ((vanilla = loaded.vanilla), loaded))

/// Gives `host` its behaviour, as `host.machine`, once Zag is here: `name` is one of Zag's packages, and `props`
/// is as a Machine's. Until then `host.machine` is nothing, and the element draws itself as it will look, not yet
/// answering. → true, so an element that asks as it's drawn can tell it has asked.
export function behaviour(host, name, props) {
  late.then(async (loaded) => {
    // A package that isn't in the first part is in the mouse's.
    const zag = loaded[name] ?? (await (mouse ??= import("./zag-mouse.js")))[name]
    host.machine = new Machine(host, zag, props)
    host.requestUpdate()
  })
  return true
}

/// A Zag machine that lives while its element is on the page, and draws the element again whenever it changes.
/// `zag` is one of Zag's packages, loaded; `props` returns what the machine is told, read from the element each
/// time it's asked, so it's always what the element's attributes say now. It's given the package, for what only
/// the package can make. What it returns must be the same values until the element changes: a new array or
/// object each time reads as a change each time.
export class Machine {
  constructor(host, zag, props) {
    this.host = host
    this.zag = zag
    this.props = () => ({ ...props(zag), dir: host.matches(":dir(rtl)") ? "rtl" : "ltr" })
    this.id = `ui-${++made}`
    host.addController(this)
  }

  /// The machine, made when it's first wanted: as the element is first drawn, so it starts from what the element
  /// says by then.
  get vanilla() {
    if (!this.made) {
      this.told = this.values()
      this.made = new vanilla.VanillaMachine(this.zag.machine, () => ({
        id: this.id,
        getRootNode: () => this.host.renderRoot.getRootNode(),
        ...this.props(),
      }))
      this.made.subscribe(() => this.host.requestUpdate())
    }
    return this.made
  }

  hostConnected() {
    this.host.requestUpdate()
  }

  /// A machine watches some of what it's told (a value set from outside, say), and only looks when it's nudged.
  hostUpdate() {
    const machine = this.vanilla
    const now = this.values()
    if (now.some((value, i) => value !== this.told[i])) {
      this.told = now
      machine.updateProps({})
    }
  }

  /// A machine starts once its parts are drawn: it looks for them as it starts.
  hostUpdated() {
    if (!this.started) this.vanilla.start()
    this.started = true
  }

  /// An element moved to another place on the page keeps its machine. One that's gone stops it.
  hostDisconnected() {
    queueMicrotask(() => {
      if (this.host.isConnected) return
      this.made?.stop()
      this.made = this.started = undefined
    })
  }

  /// What the machine is told, without the functions it calls back: those are new each time and change nothing.
  values() {
    return Object.values(this.props()).filter((value) => typeof value !== "function")
  }

  /// The machine as it is now: its state, and the attributes and listeners of each part.
  get api() {
    return this.zag.connect(this.vanilla.service, vanilla.normalizeProps)
  }
}

/// Puts a part's attributes and listeners, as the machine gives them, on an element of a template:
/// `<button ${part(api?.getTriggerProps())}>`. With no machine yet there are none to put.
export const part = directive(class extends Directive {
  render() {
    return noChange
  }
  update({ element }, [props]) {
    if (props) vanilla.spreadProps(element, props)
    return noChange
  }
})

/// Shows a popover while `open`, over everything else on the page: `<div popover="manual" ${over(api.open)}>`.
export const over = directive(class extends Directive {
  render() {
    return noChange
  }
  update({ element }, [open]) {
    if (element.isConnected && open !== element.matches(":popover-open")) element.togglePopover(open)
    return noChange
  }
})
