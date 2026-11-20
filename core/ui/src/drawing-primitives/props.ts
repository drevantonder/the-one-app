// What a primitive makes of the props it's given, on the DOM and a native host alike.
import { children, merge, omit, type Element } from "solid-js"
import type { PickerItemProps, PressableProps } from "./primitives.tsx"
import { flat, styles, type Styled, type Styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

/** The roles of what's pressed, which the DOM draws as a button, so the keyboard reaches it too (the renderer's
 * `BUTTON_ROLES`). */
export const pressed = new Set(["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link", "combobox"])

// A box's is a ring round it; words', a link's, drawn on what's behind them, as a run of text has no box.
const { ring, highlight } = styles({ ring: { boxShadow: { focused: tokens.shadow.focus } }, highlight: { backgroundColor: { focused: tokens.colour.selection } } })

/** Whether styles say how what they style looks with the keyboard on it: a property's `focused` state, in the last of
 * them to say that property. */
const showsFocus = (style: Styles) => {
  const last: Record<string, Styled> = {}
  for (const one of flat(style)) for (const property of Object.keys(one)) last[property] = one
  return Object.entries(last).some(([property, one]) => one.$focused?.includes(property))
}

/** A pressable's style, or a text's (`words`), with the focus ring first where Tab reaches it, as a browser's `:focus-visible` outline and
 * the OS's focus ring show where the keyboard is: its own styles change it by saying how it looks `focused`, and
 * one that doesn't, or says it and then something after it says that property again without it, still has it. What
 * the keyboard is only put on (`tabIndex` -1: a dialog as it opens) has none. */
export const focusable = (props: Pick<PressableProps, "tabIndex" | "onPress" | "disabled" | "role" | "style">, words = false): Styles =>
  props.tabIndex !== -1 && (props.tabIndex === 0 || (!!props.onPress && !props.disabled && pressed.has(props.role!))) && !showsFocus(props.style) ? [words ? highlight : ring, props.style] : props.style

/** `props` with each of `names` read from it as `names` says, in place of the props it reads (`from`): React Native's
 * names as the host names them (`tabIndex` as `tabindex`). */
export function renamed<T extends object>(props: T, from: readonly string[], names: Record<string, (props: T) => unknown>) {
  const made = {}
  for (const [name, read] of Object.entries(names)) Object.defineProperty(made, name, { get: () => read(props), enumerable: true })
  return merge(omit(props as Record<string, unknown>, ...from), made)
}

/** A picker's options, as its items (`Picker.Item`) give them, as they change. */
export function options(items: () => Element) {
  const resolved = children(items)
  return () => (resolved.toArray() as unknown as PickerItemProps[]).map(({ label, value, enabled }) => ({ label, value, disabled: enabled === false ? true : undefined }))
}

/** Whether what's typed in a text input is taken as it's typed, not capitalised or corrected, where it doesn't say
 * (`autoCapitalize`, `autoCorrect`), as iOS's and Android's own search, address, username, code and password fields
 * are: a search, an email, a URL, a password, a username or a one-time code. */
export const literal = (props: { inputMode?: string; secureTextEntry?: boolean; autoComplete?: string }) =>
  !!props.secureTextEntry || ["search", "email", "url"].includes(props.inputMode!) || /\b(username|one-time-code)\b/.test(props.autoComplete ?? "")
