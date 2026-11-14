import { html, nothing } from "lit"
import { Element, look } from "./element.js"
import { icon } from "./icon.js"
import { hold } from "./toast.js"
import css from "./sheet.css"

/// The sheets and alerts that are open, the last opened last.
const open = []

/// Where a sheet is in the middle of the window, not at its foot (sheet.css).
const wide = matchMedia("(min-width: 40rem)")

/// What a sheet and an alert share: the browser's own `<dialog>`, shown over the page while `open`. The browser
/// keeps focus inside it and closes it on Escape and on the device's back gesture; a press outside closes it too.
/// It tells the page `close` once it's shut, however that came about.
class Dialog extends Element {
  static styles = look(css)
  static properties = { heading: {}, open: { type: Boolean, reflect: true } }

  get dialog() {
    return this.renderRoot.querySelector("dialog")
  }

  updated(changed) {
    if (!changed.has("open") || this.open === this.dialog.open) return
    if (!this.open) return this.dialog.close()
    this.dialog.showModal()
    this.topmost(true)
    // The keyboard doesn't come up on a phone unless the app asked for it, with `autofocus` on a field.
    if (!this.querySelector("[autofocus]")) (this.renderRoot.querySelector("[autofocus]") ?? this.dialog).focus()
  }

  /// A press that both starts and ends on the dialog itself was on what's around it.
  outside(event) {
    if (event.type === "pointerdown") this.pressed = event.target
    else if (event.target === this.dialog && this.pressed === this.dialog) this.open = false
  }

  closed() {
    // Opened again before the browser said it had closed: it's open, and that's not news.
    if (this.dialog.open) return
    this.topmost(false)
    this.open = false
    this.emit("close")
  }

  disconnectedCallback() {
    super.disconnectedCallback()
    this.topmost(false)
  }

  /// An open dialog makes everything outside it inert, a toast too. So toasts show from inside the one that's on
  /// top, and from the page again once none is open.
  topmost(is) {
    if (open.includes(this)) open.splice(open.indexOf(this), 1)
    if (is) open.push(this)
    hold(open.at(-1)?.dialog)
  }
}

/// `<ui-sheet heading="Add expense">…</ui-sheet>`: something to fill in or look at, over the page: up from the
/// foot of a narrow window, in the middle of a wide one. Set `open`, or call `show()` and `close()`.
class Sheet extends Dialog {
  /// Told of every move of a finger on the sheet, and able to stop the device scrolling for it.
  touched = { handleEvent: (event) => this.pull(event), passive: false }

  show() {
    this.open = true
  }

  close() {
    this.open = false
  }

  render() {
    return html`
      <dialog class="sheet" aria-labelledby="heading" @pointerdown=${this.outside} @click=${this.outside} @close=${this.closed}
        @touchstart=${this.touched} @touchmove=${this.touched} @touchend=${this.touched} @touchcancel=${this.touched}>
        <div class="panel">
          <header>
            <h2 id="heading">${this.heading}</h2>
            <button class="press ring" aria-label="Close" @click=${this.close}>
              <svg viewBox="0 0 256 256" aria-hidden="true">${icon("x")}</svg>
            </button>
          </header>
          <div class="body"><slot></slot></div>
        </div>
      </dialog>
    `
  }

  /// A sheet at the foot of the window goes down with the finger that pulls it, and on letting go closes if it
  /// was pulled far or flicked, and comes back if not. A finger that's scrolling what's in the sheet isn't
  /// pulling it: only one that goes down with nothing above left to scroll to.
  pull(event) {
    const { dialog } = this
    const finger = event.touches[0] ?? event.changedTouches[0]
    if (event.type === "touchstart") {
      const under = event.composedPath()
      const scrolled = under.slice(0, under.indexOf(dialog)).some((element) => element.scrollTop > 0)
      this.pulling = wide.matches || scrolled || event.touches.length > 1 ? null : { x: finger.clientX, y: finger.clientY, down: 0, speed: 0 }
      return
    }
    const pulling = this.pulling
    if (!pulling) return
    const down = finger.clientY - pulling.y
    if (event.type === "touchmove") {
      if (!pulling.ours) {
        const across = Math.abs(finger.clientX - pulling.x)
        // Not far enough yet to tell which way it's going.
        if (Math.hypot(across, down) < 4) return
        // Going up or sideways, or the device has already started scrolling: it's not a pull.
        if (down < across || !event.cancelable) return void (this.pulling = null)
        pulling.ours = true
        dialog.setAttribute("data-pulled", "")
      }
      event.preventDefault()
      pulling.speed = (down - pulling.down) / Math.max(1, event.timeStamp - pulling.at)
      pulling.down = down
      pulling.at = event.timeStamp
      dialog.style.setProperty("--pulled", `${Math.max(0, down)}px`)
      dialog.style.setProperty("--pulled-part", Math.min(1, Math.max(0, down) / dialog.offsetHeight))
      return
    }
    this.pulling = null
    if (!pulling.ours) return
    dialog.removeAttribute("data-pulled")
    dialog.style.removeProperty("--pulled")
    dialog.style.removeProperty("--pulled-part")
    // Let go a third of the way down, or on the way down at a flick's speed (in px a millisecond).
    if (event.type === "touchend" && (down > dialog.offsetHeight / 3 || pulling.speed > 0.5)) this.open = false
  }
}
customElements.define("ui-sheet", Sheet)

/// A question that stops everything until it's answered: the one the page has, asked through `confirm`.
class Alert extends Dialog {
  static properties = { text: {}, action: {}, destructive: { type: Boolean } }

  render() {
    const answer = (value) => () => ((this.answer = value), (this.open = false))
    return html`
      <dialog class="alert" role="alertdialog" aria-labelledby="heading" aria-describedby="text" @pointerdown=${this.outside} @click=${this.outside} @close=${this.closed}>
        <div class="panel">
          <h2 id="heading">${this.heading}</h2>
          ${this.text ? html`<p id="text">${this.text}</p>` : nothing}
          <footer>
            <ui-button variant="outline" autofocus @click=${answer(false)}>Cancel</ui-button>
            <ui-button variant=${this.destructive ? "destructive" : "default"} @click=${answer(true)}>${this.action}</ui-button>
          </footer>
        </div>
      </dialog>
    `
  }
}
customElements.define("ui-alert", Alert)

const alert = document.createElement("ui-alert")

/// `await confirm({ heading: "Delete this expense?", text: "It can't be brought back.", action: "Delete", destructive: true })`:
/// asks before doing what can't be undone. → whether `action` was chosen; Cancel, Escape and a press outside are no.
export async function confirm({ heading, text = "", action = "OK", destructive = false }) {
  if (!alert.isConnected) document.body.append(alert)
  Object.assign(alert, { heading, text, action, destructive, answer: false })
  // Its buttons are drawn before it opens, so focus starts on Cancel.
  await alert.updateComplete
  alert.open = true
  return new Promise((answered) => alert.addEventListener("close", () => answered(alert.answer), { once: true }))
}
