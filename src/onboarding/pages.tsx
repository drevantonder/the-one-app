// What auth's pages show as they open, each @one/ui's Welcome: drawn to HTML when @one/ui is built (src/screen/ui/build.js
// writes them to its dist/auth.json), with {{name}} and the rest for pages.js to fill in. pages.js serves them, with the
// scripts that make them work, and render.js draws them all to HTML files to look at.
import { Problem } from "@/components/problem"
import { List, ListRow } from "@/components/layout"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { Welcome } from "./welcome"

const logo = "{{logo}}"
const WORKS = "Passkeys saved in iCloud Keychain, Google Password Manager or a security key work, on a current system."
const ANOTHER = "Add a second passkey, on another device or a security key, so losing one doesn't lock you out."

export const pages = {
  "sign-in": <Welcome logo={logo} title="{{name}}" action={{ label: "Sign In" }} />,
  "sign-in-google": <Welcome logo={logo} title="{{name}}" action={{ label: "Sign In with Google" }} other={{ label: "Use a Passkey" }} />,
  "sign-in-throwaway": <Welcome logo={logo} title="{{name}}" action={{ label: "Sign In as {{user}}" }} />,
  "invite": <Welcome logo={logo} title="Join {{name}}" action={{ label: "Join" }} />,
  "invite-google": <Welcome logo={logo} title="Join {{name}}" action={{ label: "Sign In with Google" }} />,
  "joined": <Welcome logo={logo} title="You've joined {{yours}}" action={{ label: "Open {{name}}", href: "/" }} />,
  "passkeys": (
    <Welcome logo={logo} title="Passkeys" text="Each one signs you in to {{name}}." callout={ANOTHER} action={{ label: "Add a Passkey" }} note={WORKS}>
      <Spinner className="size-5" />
      <List data-slot="passkeys" className="empty:hidden" />
    </Welcome>
  ),
  "code": <Welcome logo={logo} title="Enter your code" field={{ label: "Code", placeholder: "BCDF-GHJK" }} action={{ label: "Continue" }} />,
  "code-signed-in": <Welcome logo={logo} title="You're signed in" text="Go back to where you got the code." />,
  "code-not-signed-in": <Welcome logo={logo} title="That didn't sign in" text="Start again where you got the code." />,
}

// What the pages' scripts draw in them: a problem, a button's spinner, a passkey's row.
export const parts = {
  problem: <Problem title="-" text="-" />,
  spinner: <Spinner className="text-current" />,
  passkey: <ListRow title="-" sub="-" trailing={<Button variant="ghost" size="sm">Remove</Button>} />,
}
