// The client's own page in an app's window that didn't open: its problem box, as DESIGN.md has one, with Try Again,
// which opens the app again. It's drawn with the UI library, as the recovery key window is (recovery.js), and asks its
// host only what it's for (`one/unopened`, in native.rs).
import { createComponent as h } from "/ui/client/solid.js"
import { Button, Problem, render, Screen } from "/ui/client/native/ui.js"

const waiting = new Map()
let asked = 0
const heard = globalThis.ui.message
globalThis.ui.message = (message) => {
  const answer = typeof message.id === "string" ? waiting.get(message.id) : undefined
  if (!answer) return heard(message)
  waiting.delete(message.id)
  answer(message)
}
const ask = (method, params = {}) =>
  new Promise((done, failed) => {
    const id = `unopened-${++asked}`
    waiting.set(id, ({ result, error }) => (error ? failed(new Error(error.message)) : done(result)))
    globalThis.host.post({ jsonrpc: "2.0", id, method, params })
  })

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
