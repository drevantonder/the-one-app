import { html, nothing } from "lit"
import { Element, look } from "./element.js"
import { icon } from "./icon.js"
import css from "./sheet.css"

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
    this.open = false
    this.emit("close")
  }
}

/// `<ui-sheet heading="Add expense">…</ui-sheet>`: something to fill in or look at, over the page: up from the
/// foot of a narrow window, in the middle of a wide one. Set `open`, or call `show()` and `close()`.
class Sheet extends Dialog {
  show() {
    this.open = true
  }

  close() {
    this.open = false
  }

  render() {
    return html`
      <dialog class="sheet" aria-labelledby="heading" @pointerdown=${this.outside} @click=${this.outside} @close=${this.closed}>
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
