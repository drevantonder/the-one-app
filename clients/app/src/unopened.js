// The client's own page in an app's window that didn't open: its problem box, as DESIGN.md has one, with Try Again,
// which opens the app again. It's drawn with the UI library, as the recovery key window is (recovery.js), and asks its
// host only what it's for (`one/unopened`, in native.rs).
import { createComponent as h } from "/ui/client/solid.js"
import { Button, Problem, render, Screen } from "/ui/client/native/ui.js"
import { ask } from "/client/ask.js"

const { name, why } = await ask("one/unopened")
const said = why.charAt(0).toUpperCase() + why.slice(1)

render(() =>
  h(Screen, {
    get children() {
      return h(Problem, {
        heading: `${name} didn't open`,
        text: said,
        get action() { return h(Button, { variant: "outline", onPress: () => ask("one/unopened/retry"), children: "Try Again" }) },
      })
    },
  }),
)
