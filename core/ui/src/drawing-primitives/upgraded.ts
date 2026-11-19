// What a page built before the drawing primitives took React Native's names gives them (`<Box onClick tabindex>`,
// `<EditableText onInput action>`), as those names (`onPress`, `tabIndex`, `onChangeText`, `enterKeyHint`), on the DOM
// and a native host alike, until every page deployed has moved to them. Then this file goes.
import type { Props, TextInputAction } from "./renderer.ts"
import type { Located } from "../types/ui.d.ts"
import { renamed } from "./props.ts"
import type { PressableProps, TextInputProps } from "./primitives.tsx"

/** A text's props before. */
export interface OldText {
  onClick?: (event: Located) => void
  tabindex?: 0 | -1
}

/** A box's props before: a view's or a pressable's, as the DOM named them. */
export type BoxProps = Omit<PressableProps, "onPress" | "tabIndex" | "autoFocus"> & {
  tabindex?: 0 | -1
  autofocus?: boolean
  onClick?: (event: Located) => void
}

/** A box's props, as a pressable's. */
export const box = (props: BoxProps) => renamed(props, ["onClick", "tabindex", "autofocus"], { onPress: (it) => it.onClick, tabIndex: (it) => it.tabindex, autoFocus: (it) => it.autofocus }) as PressableProps

/** An editable text's props before, as the DOM named them: a text input's, by other names. */
export interface EditableTextProps {
  type?: Props["type"]
  inputmode?: Props["inputmode"]
  action?: TextInputAction
  onAction?: (event: { action: TextInputAction }) => void
  onInput?: (event: { currentTarget: { value: string } }) => void
  readonly?: boolean
  autofocus?: boolean
  autocapitalize?: Props["autocapitalize"]
  autocorrect?: boolean
  spellcheck?: boolean
  autocomplete?: string
}

/** What an action key said, by what it does. */
const hints: Record<TextInputAction, string> = { newline: "enter", done: "done", go: "go", next: "next", previous: "previous", search: "search", send: "send" }

/** A text input's props, from its props before where it's given those, and as they are where it isn't. */
export function textInput<T extends object>(props: T): T {
  const it = props as T & EditableTextProps & TextInputProps
  const old = ["type", "inputmode", "action", "onAction", "onInput", "readonly", "autofocus", "autocapitalize", "autocorrect", "spellcheck", "autocomplete"].filter((name) => name in it)
  if (!old.length) return props
  return renamed(it, old, {
    inputMode: () => it.inputMode ?? it.inputmode ?? (it.type && it.type !== "password" && it.type !== "text" ? (it.type === "number" ? "decimal" : it.type) : undefined),
    secureTextEntry: () => it.secureTextEntry ?? (it.type === "password" || undefined),
    enterKeyHint: () => it.enterKeyHint ?? (it.action && hints[it.action]),
    onSubmitEditing: () => it.onSubmitEditing ?? (it.onAction && (() => it.onAction!({ action: it.action ?? (it.multiline ? "newline" : "done") }))),
    onChangeText: () => it.onChangeText ?? (it.onInput && ((text: string) => it.onInput!({ currentTarget: { value: text } }))),
    readOnly: () => it.readOnly ?? it.readonly,
    autoFocus: () => it.autoFocus ?? it.autofocus,
    autoCapitalize: () => it.autoCapitalize ?? it.autocapitalize,
    autoCorrect: () => it.autoCorrect ?? it.autocorrect,
    spellCheck: () => it.spellCheck ?? it.spellcheck,
    autoComplete: () => it.autoComplete ?? it.autocomplete,
  }) as T
}
