// The launcher's own page while this program is behind its network's servers, which refuse it until it's updated:
// Update Required, as an app that needs an update says, with Update, which takes the latest build and starts again
// on it (`one/update`), and Try Again, which asks its network afresh (`one/update/retry`), as a server rolled back
// may talk with it again. Once one does, the Home Screen takes this page's place.
import { createComponent as h, createSignal, Show } from "/ui/client/solid.js"
import { Button, Problem, render, Screen, Status, leave } from "/ui/client/native/ui.js"
import { ask, cancelOnEscape } from "/client/ask.js"

cancelOnEscape(leave)

/// Whether it's updating, and what the last try said, if it failed.
const [updating, setUpdating] = createSignal(false)
const [failed, setFailed] = createSignal()

async function update() {
  setFailed()
  setUpdating(true)
  try {
    await ask("one/update")
    await ask("one/update/retry")
  } catch (error) {
    setFailed(error.message)
  } finally {
    setUpdating(false)
  }
}

const retry = () => ask("one/update/retry").then(() => setFailed(), (error) => setFailed(error.message))

render(() =>
  h(Screen, {
    get children() {
      return h(Show, {
        get when() { return !updating() },
        fallback: h(Status, { heading: "Updating…" }),
        get children() {
          return h(Problem, {
            warn: true,
            get heading() { return failed() ? "Couldn't Update" : "Update Required" },
            get text() { return failed() ?? "This version of the One App no longer works with your network." },
            get action() {
              return [h(Button, { onPress: update, children: "Update" }), h(Button, { variant: "outline", onPress: retry, children: "Try Again" })]
            },
          })
        },
      })
    },
  }),
)
