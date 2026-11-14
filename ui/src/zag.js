// How an element gets its behaviour from Zag (zagjs.com): a state machine per element, with the keyboard, focus,
// selection and placement in it, and no look. The element draws the machine's parts and says how they look.
import { noChange } from "lit"
import { Directive, directive } from "lit/directive.js"
import { VanillaMachine, normalizeProps, spreadProps } from "@zag-js/vanilla"

let made = 0

/// A Zag machine that lives while its element is on the page, and draws the element again whenever it changes.
/// `zag` is one of Zag's packages (`@zag-js/menu`); `props` returns what the machine is told, read from the
/// element each time it's asked, so it's always what the element's attributes say now. What it returns must be
/// the same values until the element changes: a new array or object each time reads as a change each time.
export class Machine {
  constructor(host, zag, props) {
    this.host = host
    this.zag = zag
    this.props = props
    this.id = `ui-${++made}`
    host.addController(this)
  }

  /// The machine, made when it's first wanted: as the element is first drawn, so it starts from what the element
  /// says by then.
  get vanilla() {
    if (!this.made) {
      this.told = this.values()
      this.made = new VanillaMachine(this.zag.machine, () => ({
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

  hostDisconnected() {
    this.made?.stop()
    this.made = this.started = undefined
  }

  /// What the machine is told, without the functions it calls back: those are new each time and change nothing.
  values() {
    return Object.values(this.props()).filter((value) => typeof value !== "function")
  }

  /// The machine as it is now: its state, and the attributes and listeners of each part.
  get api() {
    return this.zag.connect(this.vanilla.service, normalizeProps)
  }
}

/// Puts a part's attributes and listeners, as the machine gives them, on an element of a template:
/// `<button ${part(api.getTriggerProps())}>`.
export const part = directive(class extends Directive {
  render() {
    return noChange
  }
  update({ element }, [props]) {
    spreadProps(element, props)
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
