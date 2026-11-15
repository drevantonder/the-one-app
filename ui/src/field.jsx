import { createContext, createMemo, createSignal, createUniqueId, For, omit, Show, useContext } from "solid-js"
import { Icon } from "./button.jsx"
import { said, touch } from "./kit.js"

const Context = createContext(null)

/// What the controls inside a `Field` read of it: the id its label is for (`id`), its label's (`label`), its
/// message's (`message`), and whether what's in it is wrong (`invalid()`). Nothing outside a field.
export const useField = () => useContext(Context)

/// `<Field label="Name" hint="As your card has it"><Input name="name" required /></Field>`: a `label` above
/// whatever's chosen or typed in, a grey `hint` below, and its problem in the hint's place: `error`, which the app
/// sets, or what's wrong with what was typed, once the form was sent, which shows on the field and not in the
/// browser's bubble. Every control below goes in one, the kit's with Zag's too (`<Select.Root>`,
/// `<DatePicker.Root>`, `<NumberInput.Root>`), as a form's `<input>`s do: they're the form's own, in its
/// FormData and its reset.
export function Field(props) {
  const id = createUniqueId()
  const [problem, setProblem] = createSignal("")
  const message = () => props.error || problem()
  // A control that's wrong doesn't say so to what it's in, so the field hears it on its way down to the control.
  const watch = (element) => {
    element.addEventListener("invalid", (event) => {
      event.preventDefault()
      setProblem(event.target.validationMessage)
      if (event.target.form?.querySelector(":invalid:not(fieldset)") === event.target) event.target.focus()
    }, true)
    // Once it's been called wrong, it says what's wrong as it's put right.
    const again = (event) => problem() && event.target.validity && setProblem(event.target.validationMessage)
    element.addEventListener("input", again)
    element.addEventListener("change", again)
  }
  const field = { id: `${id}-control`, label: `${id}-label`, message: `${id}-message`, invalid: () => !!message() }
  return (
    <Context value={field}>
      <div class={["ui-field", props.class]} ref={watch} data-disabled={props.disabled ? "" : undefined}>
        <Show when={props.label}>
          <label id={field.label} for={field.id}>{props.label}</label>
        </Show>
        {props.children}
        <Show when={message() || props.hint}>
          <p id={field.message} class={{ error: !!message() }}>{message() || props.hint}</p>
        </Show>
      </div>
    </Context>
  )
}

/// What every control in a field says of it: which label names it and which message describes it.
function described(field) {
  return { id: field?.id, "aria-describedby": field?.message, "aria-invalid": field?.invalid() ? "true" : undefined }
}

/// The next control of the form that's typed in, after `control`, if there's one.
function next(control) {
  const fields = [...(control.form?.elements ?? [])]
  const typed = (field) => field.matches("input:not([type=hidden], [type=checkbox], [type=radio], [type=file], [type=submit], [type=button]), textarea") && !field.disabled && field.checkVisibility()
  return fields.slice(fields.indexOf(control) + 1).find(typed)
}

/// Enter in a one-line field of a form under a finger goes on to the next field that's typed in, and a phone's
/// keyboard offers that in Enter's place, and from the last offers to send the form, unless the page said
/// (`enterkeyhint`). So a form that ends in pickers is sent from its last typed field.
function enters(control) {
  control.addEventListener("focus", () => {
    if (control.form && !control.hasAttribute("enterkeyhint")) control.enterKeyHint = next(control) ? "next" : "go"
  })
  control.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.isComposing || !touch() || control.hasAttribute("enterkeyhint")) return
    const after = next(control)
    if (!after) return
    event.preventDefault()
    after.focus()
  })
}

/// `<Input name="name" />`: one line of text, in a `Field`. It's an `<input>`, and takes what one does: `type`
/// (`email`, `tel`, `url`, `password`, `search`), `autocomplete`, `inputmode`, `enterkeyhint`, `required`, so the
/// device shows the right keyboard and offers what it knows. Enter sends the form, and a phone's keyboard offers
/// that from the last field of a form that's typed in, and the next field before it.
export function Input(props) {
  const field = useField()
  return <input {...described(field)} {...omit(props, "class", "ref")} class={["ui-box ring", props.class]} ref={[enters, props.ref]} />
}

/// `<Textarea name="note" />`: several lines of text, in a `Field`, growing with what's typed. `rows` is how tall
/// it starts.
export function Textarea(props) {
  const field = useField()
  return <textarea rows={3} {...described(field)} {...omit(props, "class")} class={["ui-box ring", props.class]} />
}

/// `<Search placeholder="Search notes" aria-label="Search notes" onInput={(event) => find(event.target.value)} />`:
/// what's typed to find something in a list: a text field with a magnifier and, once there's text, a button that
/// clears it. Escape clears it too. The list it narrows is the app's.
export function Search(props) {
  let control
  const [text, setText] = createSignal(props.value ?? "")
  const clear = () => {
    control.value = ""
    control.dispatchEvent(new Event("input", { bubbles: true }))
    control.focus()
  }
  return (
    <div class={["ui-search", props.class]}>
      <Icon name="magnifying-glass" />
      <input
        type="search"
        enterkeyhint="search"
        {...omit(props, "class", "ref")}
        class="ui-box ring"
        ref={[(element) => ((control = element), element.addEventListener("input", () => setText(element.value))), props.ref]}
        onKeyDown={(event) => {
          // Escape empties a field with text in it, and only then does what it does elsewhere (closes the sheet).
          if (event.key !== "Escape" || !control.value) return
          event.preventDefault()
          clear()
        }}
      />
      <Show when={text() && !props.disabled}>
        <button class="press ring" type="button" aria-label={said.clear} onClick={clear}>
          <Icon name="x" />
        </button>
      </Show>
    </div>
  )
}

/// `<Checkbox name="receipt" label="I have the receipt" />`: one thing that's so or not, ticked, with what it
/// says beside it and a grey `hint` under that. It's an `<input type="checkbox">` and takes what one does
/// (`checked`, `onChange`); in a form it's sent as `value` ("on" unless set) when checked. It's its own field.
export function Checkbox(props) {
  return (
    <Field error={props.error} disabled={props.disabled}>
      <Choice {...props} type="checkbox" />
    </Field>
  )
}

/// `<Switch name="reimbursable" label="Reimbursable" />`: something that's on or off, and takes effect at once.
/// A checkbox in every other way.
export function Switch(props) {
  return (
    <Field error={props.error} disabled={props.disabled}>
      <Choice {...props} type="checkbox" role="switch" />
    </Field>
  )
}

function Choice(props) {
  const field = useField()
  return (
    <label class={["ui-choice press", { switch: props.role === "switch" }]}>
      <input {...omit(props, "label", "hint", "error")} id={field.id} aria-describedby={field.message} aria-invalid={field.invalid() ? "true" : undefined} class="ring" />
      <Show when={props.label}>
        <span>
          {props.label}
          <Show when={props.hint}>
            <small>{props.hint}</small>
          </Show>
        </span>
      </Show>
    </label>
  )
}

/// `<RadioGroup name="paid" value="card" options={[{ value: "card", label: "My card" }, ...]} />`: one of a few, all
/// in view, in a `Field` whose label names them. For two to five things; a select holds more. The arrow keys move
/// between them, as they do in the browser's own. `onChange` is told the value chosen.
export function RadioGroup(props) {
  const field = useField()
  const name = createUniqueId()
  return (
    <div class="ui-radios" role="radiogroup" aria-labelledby={field?.label} aria-describedby={field?.message} aria-invalid={field?.invalid() ? "true" : undefined}>
      <For each={props.options}>
        {(option, i) => (
          <label class="ui-choice press">
            <input
              type="radio"
              class="ring"
              id={i() === 0 ? field?.id : undefined}
              name={props.name ?? name}
              value={option.value}
              checked={option.value === props.value}
              required={props.required}
              disabled={props.disabled || option.disabled}
              aria-invalid={field?.invalid() ? "true" : undefined}
              onChange={() => props.onChange?.(option.value)}
            />
            <span>{option.label}</span>
          </label>
        )}
      </For>
    </div>
  )
}

const units = ["byte", "kilobyte", "megabyte", "gigabyte"]

/// How big a file is, as the user's language writes it: "248 kB".
function size(bytes) {
  const unit = Math.min(units.length - 1, Math.floor(Math.log10(Math.max(bytes, 1)) / 3))
  return new Intl.NumberFormat(navigator.language, { style: "unit", unit: units[unit], maximumFractionDigits: unit > 1 ? 1 : 0 }).format(bytes / 1000 ** unit)
}

/// An image drawn no bigger than `side` pixels either way. One this browser can't draw (an iPhone's HEIC, anywhere
/// else) throws.
async function drawn(file, side) {
  const image = await createImageBitmap(file)
  const scale = Math.min(1, side / Math.max(image.width, image.height))
  const canvas = Object.assign(document.createElement("canvas"), { width: Math.round(image.width * scale), height: Math.round(image.height * scale) })
  canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height)
  image.close()
  return canvas
}

/// `const sent = await fitted(file, 2000)`: a photo as a JPEG no bigger than `side` pixels either way, before it's
/// uploaded: a phone's photo is many times what reading one needs. A JPEG that's that small already, anything
/// that isn't a picture, and one this device can't draw are as they were. `File`'s `fit` does this to what's
/// chosen.
export async function fitted(file, side) {
  if (!file.type.startsWith("image/")) return file
  try {
    if (file.type === "image/jpeg") {
      const image = await createImageBitmap(file)
      const small = Math.max(image.width, image.height) <= side
      image.close()
      if (small) return file
    }
    const canvas = await drawn(file, side)
    const photo = await new Promise((made) => canvas.toBlob(made, "image/jpeg", 0.8))
    return new File([photo], file.name.replace(/\.[^.]*$/, "") + ".jpg", { type: photo.type, lastModified: file.lastModified })
  } catch {
    return file
  }
}

/// `<File name="receipt" accept="image/*" multiple onChange={(files) => ...} />`: files the user picks, in a
/// `Field`: a photo taken now, an image, anything else. Its button opens the device's own chooser, which offers
/// what the device has (under a finger, the camera and the photo library); `accept` and `capture` are an
/// `<input type="file">`'s. What's chosen shows above the button, each with a button that removes it. With
/// `multiple` choosing again adds; without it one file is all there is, and it's removed to choose another.
/// `onChange` is told what's chosen, as an array of Files, and a form is sent them under `name`. A file the app
/// has kept already goes in `files` as what's known of it, `{ name, type, size, src }` with `src` its picture's
/// address, and shows as one chosen. `fit={2000}` makes each photo chosen a JPEG of at most that many pixels
/// either way first: a phone's photo is many times what reading a receipt needs.
export function File(props) {
  const field = useField()
  let control
  const [files, setFiles] = createSignal(props.files ?? [])
  // Each image's small picture, once it's drawn. It's a `data:` address: an MCP App's page may show no `blob:` one.
  const [previews, setPreviews] = createSignal(new Map())
  const preview = (file) => {
    if (!(file instanceof Blob) || !file.type.startsWith("image/") || previews().has(file)) return
    drawn(file, 96).then((canvas) => setPreviews((now) => new Map(now).set(file, canvas.toDataURL("image/jpeg", 0.8))), () => {})
  }
  const set = (chosen) => {
    setFiles(chosen)
    chosen.forEach(preview)
    // What the form is sent is what's chosen now, which the device's control holds.
    const held = new DataTransfer()
    for (const file of chosen) if (file instanceof Blob) held.items.add(file)
    control.files = held.files
    props.onChange?.(chosen)
  }
  const chosen = async () => {
    let picked = [...control.files]
    if (props.fit) picked = await Promise.all(picked.map((file) => fitted(file, props.fit)))
    set(props.multiple ? [...files().filter((file) => !picked.includes(file)), ...picked] : picked.slice(0, 1))
  }
  const remove = (file) => {
    set(files().filter((kept) => kept !== file))
    // Its button went with it: the keyboard carries on from the picker.
    control.focus()
  }
  const picture = createMemo(() => (file) => file.src ?? previews().get(file))
  return (
    <div class="ui-file">
      <Show when={files().length}>
        <ul>
          <For each={files()}>
            {(file) => (
              <li>
                <Show when={picture()(file)} fallback={<span class="thumb"><Icon name="file" /></span>}>
                  {(src) => <img src={src()} alt="" />}
                </Show>
                <span class="words">
                  <span>{file.name}</span>
                  <small>{size(file.size)}</small>
                </span>
                <button class="press ring" type="button" aria-label={`${said.remove} ${file.name}`} disabled={props.disabled} onClick={() => remove(file)}>
                  <Icon name="x" />
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <div class="ui-picker" hidden={!props.multiple && files().length > 0}>
        <div class="ui-box" aria-hidden="true" data-disabled={props.disabled ? "" : undefined} data-invalid={field?.invalid() ? "" : undefined}>
          <Icon name="paperclip" />
          <span>{props.placeholder ?? said.file}</span>
        </div>
        <input
          ref={(element) => (control = element)}
          {...described(field)}
          class="native"
          type="file"
          title=""
          name={props.name}
          accept={props.accept}
          capture={props.capture}
          multiple={props.multiple}
          required={props.required && !files().length}
          disabled={props.disabled}
          aria-label={props["aria-label"]}
          onChange={chosen}
        />
      </div>
    </div>
  )
}
