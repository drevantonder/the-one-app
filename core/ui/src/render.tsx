// `render` as a page calls it: its first screen, in an error boundary, so a page that throws while drawing shows what
// failed and Try Again instead of a blank window. Once here for every page, the DOM's and a native host's: the
// primitives' own `render` (#primitives) draws it.
import { Errored } from "solid-js"
import type { JSX } from "@solidjs/web"
import { render as draw } from "#primitives"
import { Button } from "./styled-components/button.tsx"
import { Screen } from "./styled-components/screen.tsx"
import { Problem } from "./styled-components/states.tsx"
import { messages } from "./words.ts"

/// `render(() => <App />)`: draws an app's first screen, where its page is: the page itself on the web, the app's
/// window on a native host. → what takes it away again. If the page throws while drawing, a `<Problem>` takes its place,
/// with Try Again, which draws it again.
export const render = (code: () => JSX.Element): (() => void) =>
  draw(() => (
    <Errored
      fallback={(error, reset) => (
        <Screen>
          <Problem
            heading={messages.cantShowPage}
            text={error() as Error}
            action={
              <Button variant="outline" onPress={reset}>
                {messages.tryAgain}
              </Button>
            }
          />
        </Screen>
      )}
    >
      {code()}
    </Errored>
  ))
