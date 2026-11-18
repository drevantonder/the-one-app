// Colour Mode: light, dark, or as the device is, for every app on this device.
import { createSignal, Show } from "solid-js"
import { client, Field, RadioGroup, render, Screen } from "ui"

const [mode, setMode] = createSignal<string>()
client().then(({ theme }) => setMode(theme ?? "device"))
const choose = (mode: string) => client({ theme: mode === "device" ? null : (mode as "light" | "dark") }).then(() => setMode(mode))

render(() => (
  <Screen heading="Colour Mode">
    <Show when={mode()}>
      <Field hint="For every app on this device.">
        <RadioGroup
          name="mode"
          value={mode()}
          onChange={choose}
          options={[
            { value: "device", label: "As this device is" },
            { value: "light", label: "Light" },
            { value: "dark", label: "Dark" },
          ]}
        />
      </Field>
    </Show>
  </Screen>
))
