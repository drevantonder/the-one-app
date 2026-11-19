// What's typed in or chosen, with its state and no look: a field that names what's in it and says what's wrong with
// it, a checkbox or a switch, and radios. From Vercel's UI SDK (Apache-2.0) at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7: a field's label names its control, its description is its hint or, once
// there is one, its error, and the control is invalid and disabled with it
// (packages/core/src/components/field/field-attrs.ts); a checkbox or a switch is ticked as it's pressed, by a pointer
// or Space (components/checkbox/checkbox.ts, components/switch/switch.ts); radios are one tab stop, the checked one,
// and every arrow moves to the next and checks it, wrapping (components/radio/radio.ts, on
// foundation/focus/roving-focus.ts). On our host a control is named by `aria-label` and described by
// `aria-description`, in place of the ids `aria-labelledby` and `aria-describedby` name.
import { createContext, merge, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { createControllableSignal } from "../behavior-primitives/controllable.ts"
import { createRovingFocus } from "../behavior-primitives/roving-focus.ts"

/** What the controls inside a `Field` read of it. */
export interface FieldContext {
  /** Its label's words, which name what's in it. */
  label: () => string | undefined
  /** Its hint, or its problem. */
  message: () => string | undefined
  invalid: () => boolean
  disabled: () => boolean
}

/** A field's props. */
export interface FieldProps {
  label?: JSX.Element
  hint?: JSX.Element
  error?: string
  disabled?: boolean
}

export const Field = createContext<FieldContext | null>(null)

/// What the controls inside a `Field` read of it: the words that name them (`label()`), its hint or problem
/// (`message()`), whether what's in it is wrong (`invalid()`), and whether it's off (`disabled()`).
export const useField = (): FieldContext | null => useContext(Field)

const words = (said: JSX.Element) => (typeof said === "string" ? said : undefined)

/** `<Field value={createField(props)}>`: what a field tells what's in it. */
export const createField = (props: FieldProps): FieldContext => ({
  label: () => words(props.label),
  message: () => props.error || words(props.hint),
  invalid: () => !!props.error,
  disabled: () => !!props.disabled,
})

/** What every control in a field says of it: the field's label as its name, its hint or problem as its description,
 * whether it's wrong, and whether it's off. */
export const named = (field: FieldContext | null, props: { "aria-label"?: string; "aria-invalid"?: "true" | "false"; disabled?: boolean }) => ({
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

/** A checkbox's or a switch's props. */
export interface ToggleProps {
  label?: JSX.Element
  hint?: JSX.Element
  error?: string
  value?: boolean
  disabled?: boolean
  "aria-label"?: string
  onValueChange?: (value: boolean) => void
}

/** `const toggle = createToggle(props, "switch")`: whether it's ticked, and the props of what's pressed to tick it. */
export function createToggle(props: ToggleProps, role: "checkbox" | "switch") {
  const [checked, setChecked] = createControllableSignal<boolean>({ value: () => props.value, defaultValue: () => false, onChange: (value) => props.onValueChange?.(value) })
  return {
    checked: () => !!checked(),
    control: () => ({
      role,
      get "aria-checked"() {
        return checked() ? ("true" as const) : ("false" as const)
      },
      get "aria-disabled"() {
        return props.disabled ? ("true" as const) : undefined
      },
      get "aria-invalid"() {
        return props.error ? ("true" as const) : undefined
      },
      get "aria-label"() {
        return props["aria-label"] ?? words(props.label)
      },
      get "aria-description"() {
        return props.error ?? words(props.hint)
      },
      onPress: () => !props.disabled && setChecked(!checked()),
    }),
  }
}

/** Radios' props. */
export interface RadioGroupProps {
  value?: string
  options: { value: string; label: JSX.Element; disabled?: boolean }[]
  disabled?: boolean
  onValueChange?: (value: string) => void
}

/** `const radios = createRadioGroup(props, field)`: which is checked, and the props of the group and each radio. */
export function createRadioGroup(props: RadioGroupProps, field: FieldContext | null) {
  const [chosen, choose] = createControllableSignal<string>({ value: () => props.value, onChange: (value) => props.onValueChange?.(value) })
  const off = (option: RadioGroupProps["options"][number]) => !!props.disabled || !!option.disabled || !!field?.disabled()
  const roving = createRovingFocus({ items: () => props.options.filter((option) => !off(option)).map((option) => ({ key: option.value, preferred: option.value === chosen() })), orientation: "both", loop: true, remember: false })
  return {
    chosen,
    off,
    group: () => ({
      role: "radiogroup" as const,
      get "aria-label"() {
        return field?.label()
      },
      get "aria-description"() {
        return field?.message()
      },
      get "aria-invalid"() {
        return field?.invalid() ? ("true" as const) : undefined
      },
      onKeyDown: (event: Parameters<typeof roving.keydown>[0]) => {
        const to = roving.keydown(event)
        if (to !== undefined) choose(to)
      },
    }),
    radio: (option: RadioGroupProps["options"][number]) =>
      merge(roving.item(option.value), {
        role: "radio" as const,
        get "aria-checked"() {
          return option.value === chosen() ? ("true" as const) : ("false" as const)
        },
        get "aria-disabled"() {
          return off(option) ? ("true" as const) : undefined
        },
        onPress: () => !off(option) && choose(option.value),
      }),
  }
}
