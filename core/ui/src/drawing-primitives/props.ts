// What a primitive makes of the props it's given, on the DOM and a native host alike.
import { children, merge, omit, type Element } from "solid-js"
import type { PickerItemProps } from "./primitives.tsx"

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
