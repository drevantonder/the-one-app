import { createMemo, For } from "solid-js"
import { render } from "@solidjs/web"
import { popover } from "./popover.js"

// Ark UI's toast (ark-ui.com/docs/components/toast) has an app draw each toast. The UI library draws them itself, the
// same in every app, so an app only says what happened: `toast`.

/// Zag's toasts, loaded with the first toast; the toasts there are; and the one element of the page that shows
/// them, from the first toast on.
let zag
let store
let host
/// Where on the page the toaster is: the page itself, or the open sheet or alert that's on top (sheet.jsx).
let home = document.body

/// Moves toasts to `where`, or back to the page. They go on showing as they were.
export function hold(where = document.body) {
  home = where
  if (host && host.parentNode !== home) home.append(host)
}

/// `toast("Deleted", { action: { label: "Undo", run: restore } })`: one line about what just happened, at the foot
/// of the window for a few seconds, with one thing to do about it. What went well needs none: fine is silent.
export async function toast(text, { action } = {}) {
  zag ??= await import("./zag/toast.js")
  if (!store) {
    store = zag.createStore({ placement: "bottom", gap: 8, max: 3, offsets: "1rem" })
    host = document.createElement("div")
    host.className = "ui-toaster"
    render(() => <Toaster />, host)
  }
  if (host.parentNode !== home) home.append(host)
  store.create({ title: text, duration: action ? 8000 : 4000, action: action && { label: action.label, onClick: action.run } })
}

function Toaster() {
  const service = zag.useMachine(zag.group.machine, { id: "ui-toaster", store })
  const api = createMemo(() => zag.group.connect(service, zag.normalizeProps))
  return (
    <div {...api().getGroupProps()} ref={popover(() => api().getCount() > 0)}>
      <For each={api().getToasts()} keyed={(toast) => toast.id}>
        {(toast, index) => <Toast toast={toast()} index={index()} parent={service} />}
      </For>
    </div>
  )
}

function Toast(props) {
  const service = zag.useMachine(zag.machine, () => ({ ...props.toast, index: props.index, parent: props.parent }))
  const api = createMemo(() => zag.connect(service, zag.normalizeProps))
  return (
    <div class="ui-toast" {...api().getRootProps()}>
      <span {...api().getGhostBeforeProps()} />
      <p {...api().getTitleProps()}>{api().title}</p>
      {props.toast.action && (
        <button class="press ring reach" {...api().getActionTriggerProps()}>
          {props.toast.action.label}
        </button>
      )}
      <span {...api().getGhostAfterProps()} />
    </div>
  )
}
