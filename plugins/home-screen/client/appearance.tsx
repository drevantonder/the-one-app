// Appearance: light, dark, or as the device is, for every app on this device.
import { createSignal, Show } from "solid-js"
import { render } from "@solidjs/web"
import { client, Field, leave, RadioGroup, Screen } from "ui"

const [theme, setTheme] = createSignal<string>()
client().then(({ theme }) => setTheme(theme ?? "device"))
const choose = (theme: string) => client({ theme: theme === "device" ? null : (theme as "light" | "dark") }).then(() => setTheme(theme))

render(
  () => (
    <Screen heading="Appearance" back onBack={leave}>
      <Show when={theme()}>
        <Field label="Theme" hint="For every app on this device.">
          <RadioGroup
            name="theme"
            value={theme()}
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
  ),
  document.body,
)
