// Appearance, as Settings draws it: light, dark, or as the device is, for every app on this device.
import { createSignal, Show } from "solid-js"
import { client, Field, RadioGroup, Screen } from "ui"

const [mode, setMode] = createSignal<string>()
client().then(({ theme }) => setMode(theme ?? "device"))
const choose = (mode: string) => client({ theme: mode === "device" ? null : (mode as "light" | "dark") }).then(() => setMode(mode))

export default function Appearance() {
  return (
    <Screen heading="Appearance" back>
      <Show when={mode()}>
        <Field label="Colour Mode" hint="For every app on this device.">
          <RadioGroup
            value={mode()}
            onValueChange={choose}
            options={[
              { value: "device", label: "As this device is" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
          />
        </Field>
      </Show>
    </Screen>
  )
}
