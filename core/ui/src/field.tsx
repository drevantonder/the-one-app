// What's typed in or chosen: a field around it, a line or lines of text, a search, a checkbox, a switch and radios,
// written once in strict, on the primitives (#primitives), the same on the DOM and on a native host.
import { createContext, createSignal, For, omit, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, EditableText, Path, Text } from "#primitives"
import { touch } from "#bridge"
import type { EditableTextProps, Keyed } from "./strict/primitives.tsx"
import { Icon } from "./button.tsx"
import { messages } from "./words.ts"
import { styles, type Styles } from "./styles.ts"
import { tokens } from "./tokens.ts"

const look = styles({
  field: { gap: tokens.grid[2], minWidth: 0, font: tokens.type.medium, color: tokens.colour.label },
  label: { alignSelf: "flex-start", font: tokens.type.medium, fontWeight: tokens.weight.medium, userSelect: "none" },
  // On a computer, as Raycast's and macOS's forms: the label beside what it names, grey and right-aligned, and the
  // hint under what it names.
  beside: { flexDirection: "row", alignItems: "flex-start", gap: tokens.grid[3] },
  besideLabel: { flexShrink: 0, width: tokens.length.formLabel, paddingTop: tokens.grid[2], textAlign: "end", fontWeight: tokens.weight.regular, color: tokens.colour.secondaryLabel },
  control: { flexGrow: 1, flexShrink: 1, minWidth: 0, gap: tokens.grid[1.5] },
  dim: { opacity: 0.5 },
  message: { font: tokens.type.small, color: tokens.colour.secondaryLabel },
  wrong: { color: tokens.colour.red },
  box: {
    width: "100%",
    minWidth: 0,
    height: tokens.length.control,
    paddingInline: tokens.grid[3],
    borderRadius: tokens.radius.medium,
    font: tokens.type.field,
    color: tokens.colour.label,
    backgroundColor: tokens.colour.field,
    boxShadow: { default: tokens.shadow.field, focused: tokens.shadow.focus },
    transitionProperty: ["boxShadow"],
    transition: tokens.motion.quick,
  },
  boxWrong: { boxShadow: { default: tokens.shadow.wrongEdge, focused: tokens.shadow.wrongFocus } },
  lines: { height: "auto", minHeight: tokens.grid[16], paddingBlock: tokens.grid[2] },
  search: { position: "relative", maxWidth: tokens.length.readable },
  searchText: { paddingStart: tokens.length.control, paddingEnd: tokens.length.control },
  magnifier: { position: "absolute", top: 0, bottom: 0, start: tokens.grid[3], justifyContent: "center", color: tokens.colour.secondaryLabel, pointerEvents: "none" },
  clear: {
    position: "absolute",
    top: 0,
    bottom: 0,
    end: 0,
    width: tokens.length.control,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: tokens.radius.medium,
    color: { default: tokens.colour.secondaryLabel, hover: tokens.colour.label },
    cursor: "pointer",
  },
  choice: { flexDirection: "row", alignItems: "center", gap: tokens.grid[3], maxWidth: tokens.length.readable, minHeight: tokens.length.control, cursor: "pointer", userSelect: "none", opacity: { default: 1, pressed: 0.7 } },
  off: { cursor: "default", opacity: 0.5 },
  words: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  said: { font: tokens.type.medium, fontWeight: tokens.weight.medium },
  hint: { font: tokens.type.small, color: tokens.colour.secondaryLabel },
  tick: {
    width: tokens.grid[4],
    height: tokens.grid[4],
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: tokens.radius.small,
    backgroundColor: tokens.colour.field,
    boxShadow: { default: tokens.shadow.field, focused: tokens.shadow.focus },
    color: tokens.colour.onAccent,
  },
  // Ticked, as a computer's own checkbox is: its accent with a white tick.
  ticked: { backgroundColor: tokens.colour.blue, boxShadow: { default: "none", focused: tokens.shadow.focus } },
  tickWrong: { boxShadow: { default: tokens.shadow.wrongEdge, focused: tokens.shadow.wrongFocus } },
  mark: { width: tokens.grid[3], height: tokens.grid[3], opacity: 0, scale: 0.5, transitionProperty: ["opacity", "scale"], transition: tokens.motion.quick },
  shown: { opacity: 1, scale: 1 },
  round: { borderRadius: tokens.radius.full },
  dot: { width: tokens.grid[2], height: tokens.grid[2], borderRadius: tokens.radius.full, backgroundColor: tokens.colour.background, opacity: 0, scale: 0.5, transitionProperty: ["opacity", "scale"], transition: tokens.motion.quick },
  track: {
    width: tokens.grid[8],
    height: tokens.grid[5],
    flexShrink: 0,
    paddingInline: tokens.grid[0.5],
    justifyContent: "center",
    borderRadius: tokens.radius.full,
    backgroundColor: tokens.colour.quaternaryLabel,
    boxShadow: { default: "none", focused: tokens.shadow.focus },
    transitionProperty: ["backgroundColor"],
    transition: tokens.motion.settle,
  },
  // On, as iOS's and macOS's switches are: green, with a white thumb.
  on: { backgroundColor: tokens.colour.green },
  thumb: { width: tokens.grid[4], height: tokens.grid[4], borderRadius: tokens.radius.full, backgroundColor: tokens.colour.onAccent, boxShadow: tokens.shadow.edge, transitionProperty: ["translateX"], transition: tokens.motion.settle },
  thumbOn: { translateX: tokens.grid[3] },
  radios: { minWidth: 0 },
})

/** What the controls inside a `Field` read of it. */
export interface FieldContext {
  /** Its label's words, which name what's in it. */
  label: () => string | undefined
  /** Its hint, or its problem. */
  message: () => string | undefined
  invalid: () => boolean
  disabled: () => boolean
}

const Context = createContext<FieldContext | null>(null)

/// What the controls inside a `Field` read of it: the words that name them (`label()`), its hint or problem
/// (`message()`), whether what's in it is wrong (`invalid()`), and whether it's off (`disabled()`).
export const useField = (): FieldContext | null => useContext(Context)

/** A `Field`'s props. */
export interface FieldProps {
  label?: JSX.Element
  hint?: JSX.Element
  error?: string
  disabled?: boolean
  children?: JSX.Element
}

/// `<Field label="Name" hint="As your card has it" error={problem()}><Input value={name()} onInput={…} /></Field>`: a
/// `label` above whatever's typed in or chosen, which names it, a grey `hint` below, and its problem, `error`, red in
/// the hint's place, which the app sets: the field's edge goes red too. Every control below goes in one.
export function Field(props: FieldProps) {
  const words = (said: JSX.Element) => (typeof said === "string" ? said : undefined)
  const field: FieldContext = {
    label: () => words(props.label),
    message: () => props.error || words(props.hint),
    invalid: () => !!props.error,
    disabled: () => !!props.disabled,
  }
  return (
    <Context value={field}>
      <Box style={[look.field, !touch() && !!props.label && look.beside]}>
        <Show when={props.label}>
          <Text style={[look.label, !touch() && look.besideLabel, props.disabled && look.dim]}>{props.label}</Text>
        </Show>
        <Box style={look.control}>
          {props.children}
          <Show when={props.error || props.hint}>
            <Text style={[look.message, !!props.error && look.wrong]}>{props.error || props.hint}</Text>
          </Show>
        </Box>
      </Box>
    </Context>
  )
}

/** An `Input`'s props: an editable text's, in a field's box, with styles of its own over it: a big typed number's. */
export interface InputProps extends Omit<EditableTextProps, "style" | "multiline"> {
  style?: Styles
}

/// What every control in a field says of it: the field's label as its name, its hint or problem as its description,
/// and whether it's wrong.
const named = (field: FieldContext | null, props: { "aria-label"?: string; "aria-invalid"?: "true" | "false"; disabled?: boolean }) => ({
  get "aria-label"() {
    return props["aria-label"] ?? field?.label()
  },
  get "aria-description"() {
    return field?.message()
  },
  get "aria-invalid"() {
    return props["aria-invalid"] ?? (field?.invalid() ? ("true" as const) : undefined)
  },
  get disabled() {
    return props.disabled || field?.disabled() || undefined
  },
})

/// `<Input value={name()} onInput={(event) => setName(event.currentTarget.value)} onAction={save} />`: one line of
/// text, in a `Field`. `type` (`email`, `url`, `tel`, `password`) and `autocomplete` (`username`, `current-password`,
/// `one-time-code`) give the device's right keyboard and what it offers to fill in; `autocapitalize`, `autocorrect`
/// and `spellcheck` say what it does to a name or a code. Return is `onAction`: what the field's sheet does, as a
/// form's Return sends it; `action` names a phone's key for it (`go`, `send`, `search`; `done` unless it says).
export function Input(props: InputProps) {
  return <Boxed {...props} />
}

/// `<Textarea value={note()} onInput={(event) => setNote(event.currentTarget.value)} />`: several lines of text, in a
/// `Field`, growing with what's typed, where Return is a new line.
export function Textarea(props: Omit<InputProps, "style">) {
  return <Boxed {...props} multiline />
}

/// An editable text in a field's box, which every field above draws: named by its field, its edge red where what's in
/// it is wrong, dimmed where it's off, with a field's own look besides (`own`).
function Boxed(props: InputProps & { multiline?: boolean; own?: Styles }) {
  const field = useField()
  const wrong = () => (props["aria-invalid"] ?? (field?.invalid() ? "true" : undefined)) === "true"
  const off = () => !!props.disabled || !!field?.disabled()
  return <EditableText {...omit(props, "style", "own", "aria-label", "aria-invalid", "disabled")} {...named(field, props)} style={[look.box, props.multiline && look.lines, props.own, wrong() && look.boxWrong, off() && look.dim, props.style]} />
}

/// `<Search placeholder="Search notes" aria-label="Search notes" value={words()} onInput={(event) => setWords(event.currentTarget.value)} />`:
/// what's typed to find something in a list: a field with a magnifier and, once there's text, a button that clears
/// it. Escape clears it too. The list it narrows is the app's.
export function Search(props: Omit<InputProps, "type" | "style">) {
  const [own, setOwn] = createSignal("")
  const text = () => props.value ?? own()
  const typed = (value: string) => {
    setOwn(value)
    props.onInput?.({ currentTarget: { value } })
  }
  const keyed = (event: Keyed) => {
    props.onKeyDown?.(event)
    // Escape empties a field with text in it, and only then does what it does elsewhere (closes the sheet).
    if (event.key !== "Escape" || !text()) return
    event.preventDefault()
    typed("")
  }
  return (
    <Box style={look.search}>
      <Boxed
        {...omit(props, "onInput", "onKeyDown", "value")}
        type="search"
        action={props.action ?? "search"}
        value={text()}
        onInput={(event) => typed(event.currentTarget.value)}
        onKeyDown={keyed}
        own={look.searchText}
      />
      <Box style={look.magnifier}>
        <Icon name="magnifying-glass" />
      </Box>
      <Show when={text() && !props.disabled}>
        <Box role="button" aria-label={messages.clear} onClick={() => typed("")} style={look.clear}>
          <Icon name="x" />
        </Box>
      </Show>
    </Box>
  )
}

/** What a `Checkbox`'s or a `Switch`'s `onChange` is told: whether it's ticked now. */
export interface Ticked {
  currentTarget: { checked: boolean }
}

/** A `Checkbox`'s or a `Switch`'s props. */
export interface ChoiceProps {
  label?: JSX.Element
  hint?: JSX.Element
  error?: string
  checked?: boolean
  disabled?: boolean
  "aria-label"?: string
  onChange?: (event: Ticked) => void
}

/// One thing that's so or not, pressed to change it: its mark, `mark(on, wrong)`, before or after its words.
function Choice(props: ChoiceProps & { role: "checkbox" | "switch"; mark: (on: boolean, wrong: boolean) => JSX.Element }) {
  const [own, setOwn] = createSignal(false)
  const on = () => props.checked ?? own()
  const flip = () => {
    if (props.disabled) return
    setOwn(!on())
    props.onChange?.({ currentTarget: { checked: !on() } })
  }
  const words = () => (
    <Show when={props.label}>
      <Box style={look.words}>
        <Text style={look.said}>{props.label}</Text>
        <Show when={props.hint}>
          <Text style={look.hint}>{props.hint}</Text>
        </Show>
      </Box>
    </Show>
  )
  return (
    <Box style={look.field}>
      <Box
        role={props.role}
        aria-checked={on() ? "true" : "false"}
        aria-disabled={props.disabled ? "true" : undefined}
        aria-invalid={props.error ? "true" : undefined}
        aria-label={props["aria-label"] ?? (typeof props.label === "string" ? props.label : undefined)}
        aria-description={props.error ?? (typeof props.hint === "string" ? props.hint : undefined)}
        onClick={flip}
        style={[look.choice, props.disabled && look.off]}
      >
        <Show when={props.role === "switch"}>{words()}</Show>
        {props.mark(on(), !!props.error)}
        <Show when={props.role === "checkbox"}>{words()}</Show>
      </Box>
      <Show when={props.error}>
        <Text style={[look.message, look.wrong]}>{props.error}</Text>
      </Show>
    </Box>
  )
}

/** Phosphor's check, as the checkbox's mark draws it, bold. */
const check = "M232.49,80.49l-128,128a12,12,0,0,1-17,0l-56-56a12,12,0,1,1,17-17L96,183,215.51,63.51a12,12,0,0,1,17,17Z"

/// `<Checkbox label="I have the receipt" checked={has()} onChange={(event) => setHas(event.currentTarget.checked)} />`:
/// one thing that's so or not, ticked, with what it says beside it and a grey `hint` under that. It's its own field.
export function Checkbox(props: ChoiceProps) {
  return (
    <Choice
      {...props}
      role="checkbox"
      mark={(on, wrong) => (
        <Box style={[look.tick, on && look.ticked, wrong && !on && look.tickWrong]}>
          <Path viewBox="0 0 256 256" d={check} aria-hidden="true" style={[look.mark, on && look.shown]} />
        </Box>
      )}
    />
  )
}

/// `<Switch label="Reimbursable" checked={on()} onChange={(event) => set(event.currentTarget.checked)} />`: something
/// that's on or off, and takes effect at once, at the end of what it says. A checkbox in every other way.
export function Switch(props: ChoiceProps) {
  return (
    <Choice
      {...props}
      role="switch"
      mark={(on) => (
        <Box style={[look.track, on && look.on]}>
          <Box style={[look.thumb, on && look.thumbOn]} />
        </Box>
      )}
    />
  )
}

/** A `RadioGroup`'s props. */
export interface RadioGroupProps {
  value?: string
  options: { value: string; label: JSX.Element; disabled?: boolean }[]
  disabled?: boolean
  onChange?: (value: string) => void
}

/// `<RadioGroup value={paid()} onChange={setPaid} options={[{ value: "card", label: "My card" }, ...]} />`: one of a
/// few, all in view, in a `Field` whose label names them. For two to five things; a select holds more. The arrow keys
/// move between them. `onChange` is told the value chosen.
export function RadioGroup(props: RadioGroupProps) {
  const field = useField()
  const [own, setOwn] = createSignal<string>()
  const chosen = () => props.value ?? own()
  const off = (option: RadioGroupProps["options"][number]) => !!props.disabled || !!option.disabled || !!field?.disabled()
  const choose = (value: string) => {
    setOwn(value)
    props.onChange?.(value)
  }
  const keyed = (event: Keyed) => {
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[event.key]
    if (!step) return
    event.preventDefault()
    const open = props.options.filter((option) => !off(option))
    const at = open.findIndex((option) => option.value === chosen())
    const next = open[(at + step + open.length) % open.length]
    if (next) choose(next.value)
  }
  return (
    <Box role="radiogroup" aria-label={field?.label()} aria-description={field?.message()} aria-invalid={field?.invalid() ? "true" : undefined} onKeyDown={keyed} style={look.radios}>
      <For each={props.options}>
        {(option) => {
          const on = () => option.value === chosen()
          return (
            <Box
              role="radio"
              aria-checked={on() ? "true" : "false"}
              aria-disabled={off(option) ? "true" : undefined}
              tabindex={on() || (!chosen() && option === props.options[0]) ? 0 : -1}
              onClick={() => !off(option) && choose(option.value)}
              style={[look.choice, off(option) && look.off]}
            >
              <Box style={[look.tick, look.round, on() && look.ticked, field?.invalid() && !on() && look.tickWrong]}>
                <Box style={[look.dot, on() && look.shown]} />
              </Box>
              <Box style={look.words}>
                <Text style={look.said}>{option.label}</Text>
              </Box>
            </Box>
          )
        }}
      </For>
    </Box>
  )
}
