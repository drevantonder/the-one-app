import { children, omit, Show } from "solid-js"
import { useField } from "./field.tsx"
import { machine, type Part, type Root } from "./zag.tsx"
import type * as numberInput from "@zag-js/number-input"

const { Ark, use } = machine("number-input", () => import("./zag/number-input.ts"), ["div", "getRootProps"], {
  Label: ["label", "getLabelProps"],
  Control: ["div", "getControlProps"],
  Input: ["input", "getInputProps"],
  IncrementTrigger: ["button", "getIncrementTriggerProps"],
  DecrementTrigger: ["button", "getDecrementTriggerProps"],
  ValueText: ["span", "getValueTextProps"],
  Scrubber: ["div", "getScrubberProps"],
}, {
  defaults: () => {
    const field = useField()
    return () => ({ ids: { input: field?.id, label: field?.label }, invalid: field?.invalid(), locale: navigator.language })
  },
  // Until its machine is here, its box shows the number it starts with, and can't be typed in.
  still: (root) => ({
    getInputProps: () => ({ "data-number-input-input": "", readOnly: true, value: root.value ?? root.defaultValue ?? "" }),
    valueAsNumber: NaN,
  }),
})

/// `<Field label="Amount"><NumberInput.Root name="amount" min={0} formatOptions={{ style: "currency", currency: "AUD" }} /></Field>`:
/// a number, typed on the device's number keyboard, as the user's language writes numbers. It's Ark UI's number
/// input (ark-ui.com/docs/components/number-input), with its parts and props: `value` and `onValueChange`
/// (`({ value, valueAsNumber })`), `min`, `max` and `step`, which the arrow keys step it by, and `formatOptions`,
/// as `Intl.NumberFormat` takes them. Given no parts, it draws its box. In a form it's sent as `name`, the plain
/// number ("8.5").
export const NumberInput: {
  Root: Root<numberInput.Props & { name?: string }>
  Label: Part<{}, HTMLLabelElement>
  Control: Part
  Input: Part<{}, HTMLInputElement>
  IncrementTrigger: Part<{}, HTMLButtonElement>
  DecrementTrigger: Part<{}, HTMLButtonElement>
  ValueText: Part
  Scrubber: Part
} = {
  ...Ark,
  Root: (props) => {
    const inside = children(() => props.children)
    const field = useField()
    return (
      <Ark.Root {...omit(props, "children", "name")} class={["ui-number", props.class]}>
        {inside.toArray().length ? inside() : <Ark.Input class="ui-box ring" aria-describedby={field?.message} />}
        <Show when={props.name}>
          <Value name={props.name!} />
        </Show>
      </Ark.Root>
    )
  },
}

function Value(props: { name: string }) {
  const api = use()
  return <input type="hidden" name={props.name} value={Number.isNaN(api().valueAsNumber) ? "" : String(api().valueAsNumber)} />
}
