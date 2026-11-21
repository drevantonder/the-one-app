// What's typed in or chosen: a field around it, a line or lines of text, a search, a checkbox, a switch and radios,
// written once on the drawing primitives (#primitives), the same on every host.
import { createSignal, For, omit, onSettled, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Path, Pressable, Svg, Text, TextInput, View } from "#primitives"
import { keydown, touch } from "#bridge"
import type { Keyed, TextInputProps } from "../types/ui.d.ts"
import { Icon } from "./button.tsx"
import { askMac, pressed } from "./actions.tsx"
import { ListStyle } from "./list.tsx"
import { Room } from "./screen.tsx"
import { messages } from "../words.ts"
import { createField, createRadioGroup, createToggle, Field as Context, named, useField, type FieldProps, type RadioGroupProps, type ToggleProps } from "../headless-components/field.ts"
import { styles, type Styles } from "../styles.ts"
import { pixels, tokens } from "../tokens.ts"

/** How wide a field's room is at least for its label to go beside it, in CSS pixels: the label's and a field's. */
const BESIDE = 30 * 16

const look = styles({
  field: { gap: tokens.grid[2], minWidth: 0, font: tokens.type.medium, color: tokens.colour.label },
  label: { alignSelf: "flex-start", font: tokens.type.medium, fontWeight: tokens.weight.medium, userSelect: "none" },
  // On a computer, as Raycast's and macOS's forms: the label beside what it names, grey and right-aligned, and the
  // hint under what it names; the whole stops at a readable width, as a macOS form's fields are a fixed width, not
  // the window's.
  beside: { flexDirection: "row", alignItems: "flex-start", gap: tokens.grid[3], maxWidth: tokens.length.readable },
  besideLabel: { flexShrink: 0, width: tokens.length.formLabel, paddingTop: tokens.grid[2], textAlign: "end", fontWeight: tokens.weight.regular, color: tokens.colour.secondaryLabel },
  control: { flexGrow: 1, flexShrink: 1, minWidth: 0, gap: tokens.grid[1.5] },
  // In a column as wide as a screen's, the label and what it names split the column at its middle, as macOS's System
  // Settings' do, so a few radios or a switch are in the middle of the pane and not at the label's end.
  halfLabel: { width: "auto", flexGrow: 1, flexBasis: 0 },
  halfControl: { flexBasis: 0 },
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
  // A sidebar's search is a pill, as macOS's and Raycast's Settings' are.
  pill: { borderRadius: tokens.radius.full },
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
    boxShadow: { default: tokens.shadow.tick, focused: tokens.shadow.focus },
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

/// `<Field label="Name" hint="As your card has it" error={problem()}><Input value={name()} onChangeText={setName} /></Field>`: a
/// `label` above whatever's typed in or chosen, which names it, a grey `hint` below, and its problem, `error`, red in
/// the hint's place, which the app sets: the field's edge goes red too. Every control below goes in one.
export function Field(props: FieldProps & { children?: JSX.Element }) {
  // Its label beside what it names on a computer, where there's room for both: in an alert it's above it.
  const room = useContext(Room)
  const beside = () => !touch() && !!props.label && room() >= BESIDE
  const halves = () => beside() && room() >= pixels(tokens.length.readable)
  return (
    <Context value={createField(props)}>
      <View style={[look.field, beside() && look.beside]}>
        <Show when={props.label}>
          <Text style={[look.label, beside() && look.besideLabel, halves() && look.halfLabel, props.disabled && look.dim]}>{props.label}</Text>
        </Show>
        <View style={[look.control, halves() && look.halfControl]}>
          {props.children}
          <Show when={props.error || props.hint}>
            <Text style={[look.message, !!props.error && look.wrong]}>{props.error || props.hint}</Text>
          </Show>
        </View>
      </View>
    </Context>
  )
}

/** An `Input`'s props: a text input's, in a field's box, with styles of its own over it: a big typed number's. */
export interface InputProps extends Omit<TextInputProps, "style" | "multiline"> {
  style?: Styles
}

/// `<Input value={name()} onChangeText={setName} onSubmitEditing={save} />`: one line of text, in a `Field`, a
/// `TextInput`'s props in a field's box. `inputMode` (`email`, `url`, `tel`, `numeric`), `secureTextEntry` and
/// `autoComplete` (`username`, `current-password`, `one-time-code`) give the device's right keyboard and what it offers
/// to fill in; `autoCapitalize`, `autoCorrect` and `spellCheck` say what it does to a name or a code. Return is
/// `onSubmitEditing`: what the field's sheet does, as a form's Return sends it; `enterKeyHint` names a phone's key for
/// it (`go`, `send`, `search`; `done` unless it says).
export function Input(props: InputProps) {
  return <Boxed {...props} />
}

/// `<Textarea value={note()} onChangeText={setNote} />`: several lines of text, in a
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
  return <TextInput {...omit(props, "style", "own", "aria-label", "aria-invalid", "disabled")} {...named(field, props)} style={[look.box, props.multiline && look.lines, props.own, wrong() && look.boxWrong, off() && look.dim, props.style]} />
}

/// `<Search placeholder="Search notes" aria-label="Search notes" value={words()} onChangeText={setWords} />`:
/// what's typed to find something in a list: a field with a magnifier and, once there's text, a button that clears
/// it. Escape clears it too. ⌘F, or Ctrl+F off a Mac, gives it the keyboard, as a Mac's and a browser's Find does. The
/// list it narrows is the app's.
export function Search(props: Omit<InputProps, "inputMode" | "style">) {
  const sidebar = useContext(ListStyle) === "sidebar"
  const [own, setOwn] = createSignal("")
  // Given the keyboard as `autoFocus` is set again: off, then on at the next frame.
  const [found, setFound] = createSignal<boolean>()
  onSettled(() => {
    askMac()
    return keydown((event) => {
      if (!pressed(["mod", "f"], event)) return
      event.preventDefault()
      setFound(false)
      requestAnimationFrame(() => setFound(true))
    })
  })
  const text = () => props.value ?? own()
  const typed = (value: string) => {
    setOwn(value)
    props.onChangeText?.(value)
  }
  const keyed = (event: Keyed) => {
    props.onKeyDown?.(event)
    // Escape empties a field with text in it, and only then does what it does elsewhere (closes the sheet).
    if (event.key !== "Escape" || !text()) return
    event.preventDefault()
    typed("")
  }
  return (
    <View style={look.search}>
      <Boxed
        {...omit(props, "onChangeText", "onKeyDown", "value", "autoFocus")}
        autoFocus={found() ?? props.autoFocus}
        inputMode="search"
        enterKeyHint={props.enterKeyHint ?? "search"}
        value={text()}
        onChangeText={typed}
        onKeyDown={keyed}
        own={[look.searchText, sidebar && look.pill]}
      />
      <View style={look.magnifier}>
        <Icon name="magnifying-glass" />
      </View>
      <Show when={text() && !props.disabled}>
        <Pressable role="button" aria-label={messages.clear} onPress={() => typed("")} style={look.clear}>
          <Icon name="x" />
        </Pressable>
      </Show>
    </View>
  )
}

/** A `Checkbox`'s or a `Switch`'s props. */
export type ChoiceProps = ToggleProps

/// One thing that's so or not, pressed to change it: its mark, `mark(on, wrong)`, before or after its words. A switch
/// with a label on a computer, where there's room, is a form's row as a `Field`'s: its label beside, and the switch at
/// the controls' start with its hint under it, as macOS's two-column preferences draw a control.
function Choice(props: ChoiceProps & { role: "checkbox" | "switch"; mark: (on: boolean, wrong: boolean) => JSX.Element }) {
  const toggle = createToggle(props, props.role)
  const room = useContext(Room)
  const beside = () => props.role === "switch" && !touch() && !!props.label && room() >= BESIDE
  const halves = () => beside() && room() >= pixels(tokens.length.readable)
  const words = () => (
    <Show when={props.label}>
      <View style={look.words}>
        <Text style={look.said}>{props.label}</Text>
        <Show when={props.hint}>
          <Text style={look.hint}>{props.hint}</Text>
        </Show>
      </View>
    </Show>
  )
  const control = () => (
    <>
      <Pressable {...toggle.control()} style={[look.choice, props.disabled && look.off]}>
        <Show when={props.role === "switch" && !beside()}>{words()}</Show>
        {props.mark(toggle.checked(), !!props.error)}
        <Show when={props.role === "checkbox"}>{words()}</Show>
      </Pressable>
      <Show when={beside() && props.hint}>
        <Text style={look.message}>{props.hint}</Text>
      </Show>
      <Show when={props.error}>
        <Text style={[look.message, look.wrong]}>{props.error}</Text>
      </Show>
    </>
  )
  return (
    <View style={[look.field, beside() && look.beside]}>
      <Show when={beside()} fallback={control()}>
        <Text style={[look.label, look.besideLabel, halves() && look.halfLabel, props.disabled && look.dim]}>{props.label}</Text>
        <View style={[look.control, halves() && look.halfControl]}>{control()}</View>
      </Show>
    </View>
  )
}

/** Phosphor's check, as the checkbox's mark draws it, bold. */
const check = "M232.49,80.49l-128,128a12,12,0,0,1-17,0l-56-56a12,12,0,1,1,17-17L96,183,215.51,63.51a12,12,0,0,1,17,17Z"

/// `<Checkbox label="I have the receipt" value={has()} onValueChange={setHas} />`:
/// one thing that's so or not, ticked, with what it says beside it and a grey `hint` under that. It's its own field.
export function Checkbox(props: ChoiceProps) {
  return (
    <Choice
      {...props}
      role="checkbox"
      mark={(on, wrong) => (
        <View style={[look.tick, on && look.ticked, wrong && !on && look.tickWrong]}>
          <Svg viewBox="0 0 256 256" aria-hidden="true" style={[look.mark, on && look.shown]}><Path d={check} /></Svg>
        </View>
      )}
    />
  )
}

/// `<Switch label="Reimbursable" value={on()} onValueChange={set} />`: something
/// that's on or off, and takes effect at once, at the end of what it says, or in a computer's form where its label is beside it, at the
/// controls' start. A checkbox in every other way.
export function Switch(props: ChoiceProps) {
  return (
    <Choice
      {...props}
      role="switch"
      mark={(on) => (
        <View style={[look.track, on && look.on]}>
          <View style={[look.thumb, on && look.thumbOn]} />
        </View>
      )}
    />
  )
}

/// `<RadioGroup value={paid()} onValueChange={setPaid} options={[{ value: "card", label: "My card" }, ...]} />`: one of a
/// few, all in view, in a `Field` whose label names them. For two to five things; a select holds more. The arrow keys
/// move between them. `onValueChange` is told the value chosen.
export function RadioGroup(props: RadioGroupProps) {
  const field = useField()
  const radios = createRadioGroup(props, field)
  return (
    <View {...radios.group()} style={look.radios}>
      <For each={props.options}>
        {(option) => {
          const on = () => option.value === radios.chosen()
          return (
            <Pressable {...radios.radio(option)} style={[look.choice, radios.off(option) && look.off]}>
              <View style={[look.tick, look.round, on() && look.ticked, field?.invalid() && !on() && look.tickWrong]}>
                <View style={[look.dot, on() && look.shown]} />
              </View>
              <View style={look.words}>
                <Text style={look.said}>{option.label}</Text>
              </View>
            </Pressable>
          )
        }}
      </For>
    </View>
  )
}
