// The web's text input plugin, as every other platform's is Flutter's (clients/app/renderer/src/platform/*/text_input*):
// Flutter web's text editing strategy (flutter/engine, lib/web_ui/lib/src/engine/text_editing/text_editing.dart, BSD-3,
// its `DefaultTextEditingStrategy`, `EngineAutofillForm` and `EngineInputType`), translated and cut down to what the
// renderer asks of it. While a field is typed in, a real <input> (or <textarea>) is where the field is drawn, see-through,
// so the browser's own typing, suggestions, dictation, input methods and password manager all work through it, and it
// says the field's whole editing state back as Flutter's engine does (`TextInputClient.updateEditingState`). A field with
// autofill hints is in a <form> with the others of its group, so a password manager fills a username and its password
// together (`updateEditingStateWithTag`). Keys that type nothing are the page's first, as on a Mac (`keydown`, in
// renderer.js). A real field rather than EditContext, which Chrome has: it's what a password manager and a passkey fill.

/** @typedef {{ text: string, selectionBase: number, selectionExtent: number, composingBase: number, composingExtent: number }} Editing */

/** Flutter's autofill hints as the DOM's `autocomplete` names them (`BrowserAutofillHints`). */
const AUTOCOMPLETE = {
  username: "username",
  password: "current-password",
  newPassword: "new-password",
  oneTimeCode: "one-time-code",
  email: "email",
  name: "name",
  telephoneNumber: "tel",
  postalCode: "postal-code",
}

/** Flutter's keyboards as the DOM's `inputmode` and `type` have them (`EngineInputType`). */
const KEYBOARDS = { "TextInputType.emailAddress": ["email", "email"], "TextInputType.url": ["url", "url"], "TextInputType.phone": ["tel", "tel"], "TextInputType.number": ["decimal", "text"], "TextInputType.name": ["text", "text"] }

/** Flutter's action keys as the DOM's `enterkeyhint` has them. */
const ACTIONS = { "TextInputAction.go": "go", "TextInputAction.search": "search", "TextInputAction.send": "send", "TextInputAction.next": "next", "TextInputAction.previous": "previous", "TextInputAction.newline": "enter" }

/** See-through, where the field is drawn, with nothing of its own showing: the renderer draws the text and caret. */
const UNSEEN = "position:absolute;top:0;left:0;margin:0;padding:0;border:0;outline:none;resize:none;overflow:hidden;background:transparent;color:transparent;caret-color:transparent;text-shadow:none;transform-origin:0 0 0;font-size:16px;white-space:pre;opacity:1"

/**
 * The plugin: `call` is Flutter's text input channel to it, and `said(method, args)` is it to the renderer.
 * @param {(method: string, args: unknown[]) => void} said
 */
export function textInput(said) {
  /** @type {number | null} */
  let client = null
  /** @type {any} */
  let configuration = null
  /** @type {HTMLInputElement | HTMLTextAreaElement | null} */
  let field = null
  /** @type {HTMLFormElement | null} */
  let form = null
  /** Where a composition started, in the field's text, while one's on. */
  let composingFrom = -1
  let composing = ""
  /** The last editing state either side said, so only a change is said. */
  let last = ""

  /** The field's state now, as Flutter's `EditingState.fromDomElement` reads it. @returns {Editing} */
  const state = () => {
    const element = /** @type {HTMLInputElement | HTMLTextAreaElement} */ (field)
    const forward = element.selectionDirection !== "backward"
    const [start, end] = [element.selectionStart ?? 0, element.selectionEnd ?? 0]
    return {
      text: element.value,
      selectionBase: forward ? start : end,
      selectionExtent: forward ? end : start,
      composingBase: composingFrom >= 0 ? composingFrom : -1,
      composingExtent: composingFrom >= 0 ? composingFrom + composing.length : -1,
    }
  }
  const tell = () => {
    if (client === null || !field) return
    const now = state()
    const said_ = JSON.stringify(now)
    if (said_ === last) return
    last = said_
    said("TextInputClient.updateEditingState", [client, { ...now, selectionAffinity: "TextAffinity.downstream", selectionIsDirectional: false }])
  }

  /** A field for `config`, as Flutter's strategy makes one, its keyboard, action key, autofill and corrections. */
  const make = (/** @type {any} */ config) => {
    const multiline = config.inputType?.name === "TextInputType.multiline"
    const element = /** @type {HTMLInputElement | HTMLTextAreaElement} */ (document.createElement(multiline ? "textarea" : "input"))
    element.style.cssText = UNSEEN
    const [inputmode, type] = KEYBOARDS[config.inputType?.name] ?? ["text", "text"]
    if (!multiline) /** @type {HTMLInputElement} */ (element).type = config.obscureText ? "password" : type
    element.inputMode = config.obscureText ? "text" : inputmode
    if (ACTIONS[config.inputAction]) element.enterKeyHint = ACTIONS[config.inputAction]
    else element.enterKeyHint = "done"
    const hint = config.autofill?.hints?.[0]
    element.autocomplete = config.autocomplete ?? (hint ? (AUTOCOMPLETE[hint] ?? hint) : "off")
    if (config.autofill) element.name = String(config.autofill.uniqueIdentifier)
    element.spellcheck = config.enableSuggestions !== false
    element.setAttribute("autocorrect", config.autocorrect ? "on" : "off")
    element.setAttribute("autocapitalize", { "TextCapitalization.words": "words", "TextCapitalization.sentences": "sentences", "TextCapitalization.characters": "characters" }[config.textCapitalization] ?? "off")
    element.readOnly = !!config.readOnly
    return element
  }

  /** Its group's other fields, in a form with it, as `EngineAutofillForm` makes it, so they're filled together. */
  const grouped = (/** @type {HTMLInputElement | HTMLTextAreaElement} */ element, /** @type {any} */ config) => {
    const others = (config.fields ?? []).filter((/** @type {any} */ other) => other.autofill && other.autofill.uniqueIdentifier !== config.autofill?.uniqueIdentifier)
    if (!config.autofill) return null
    const group = document.createElement("form")
    group.noValidate = true
    group.setAttribute("autocomplete", "on")
    group.style.cssText = "position:absolute;top:0;left:0;width:0;height:0;overflow:visible"
    group.addEventListener("submit", (event) => event.preventDefault())
    for (const other of others) {
      const input = make(other)
      input.value = other.autofill.editingValue?.text ?? ""
      input.style.cssText = `${UNSEEN};width:1px;height:1px;pointer-events:none`
      input.tabIndex = -1
      // What a password manager fills in another of the group is told by its identifier.
      input.addEventListener("input", () => said("TextInputClient.updateEditingStateWithTag", [client, { [other.autofill.uniqueIdentifier]: { text: input.value, selectionBase: input.value.length, selectionExtent: input.value.length, composingBase: -1, composingExtent: -1 } }]))
      group.append(input)
    }
    group.append(element)
    return group
  }

  const detach = () => {
    ;(form ?? field)?.remove()
    ;[field, form, client, configuration, last, composingFrom, composing] = [null, null, null, null, "", -1, ""]
  }

  /** Flutter's text input channel's method calls, as the renderer makes them (`editing::TextInput`). */
  return {
    /** @param {string} method @param {any} args */
    call(method, args) {
      switch (method) {
        case "TextInput.setClient": {
          detach()
          ;[client, configuration] = args
          field = make(configuration)
          form = grouped(field, configuration)
          document.body.append(form ?? field)
          const element = field
          element.addEventListener("input", tell)
          element.addEventListener("compositionstart", () => ((composingFrom = element.selectionStart ?? 0), (composing = "")))
          element.addEventListener("compositionupdate", (event) => ((composing = event.data ?? ""), tell()))
          element.addEventListener("compositionend", () => ((composingFrom = -1), (composing = ""), tell()))
          document.addEventListener("selectionchange", tell)
          // Return in a field of one line is its action, as Flutter's strategy has it; the renderer hears Enter as a
          // key first (renderer.js), so this is what a phone's keyboard's action key does.
          element.addEventListener("keydown", (event) => {
            if (event.key === "Enter" && !(element instanceof HTMLTextAreaElement) && !event.isComposing && event.keyCode !== 229 && navigator.maxTouchPoints > 0) {
              event.preventDefault()
              said("TextInputClient.performAction", [client, configuration.inputAction ?? "TextInputAction.done"])
            }
          })
          break
        }
        case "TextInput.setEditingState": {
          if (!field) break
          last = JSON.stringify({ text: args.text, selectionBase: args.selectionBase, selectionExtent: args.selectionExtent, composingBase: args.composingBase ?? -1, composingExtent: args.composingExtent ?? -1 })
          if (composingFrom >= 0) break
          if (field.value !== args.text) field.value = args.text
          const [base, extent] = [Math.max(0, args.selectionBase), Math.max(0, args.selectionExtent)]
          if (document.activeElement === field) field.setSelectionRange(Math.min(base, extent), Math.max(base, extent), base > extent ? "backward" : "forward")
          break
        }
        case "TextInput.setEditableSizeAndTransform": {
          if (!field) break
          field.style.width = `${args.width}px`
          field.style.height = `${args.height}px`
          field.style.transform = `matrix3d(${args.transform.join(",")})`
          break
        }
        case "TextInput.show": {
          // In the press that asked for it, so a phone brings up its keyboard, with the selection it was told.
          field?.focus({ preventScroll: true })
          const told = last && JSON.parse(last)
          if (field && told) field.setSelectionRange(Math.min(told.selectionBase, told.selectionExtent), Math.max(told.selectionBase, told.selectionExtent))
          break
        }
        case "TextInput.hide":
          field?.blur()
          break
        case "TextInput.clearClient":
          document.removeEventListener("selectionchange", tell)
          detach()
          break
      }
    },
    /** Whether a field is being typed in, and its input method composing. */
    get field() {
      return field
    },
    get composing() {
      return composingFrom >= 0
    },
  }
}
