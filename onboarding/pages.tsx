// What auth's pages show as they open, each @one/ui's Welcome: drawn to HTML when @one/ui is built (ui/kit/build.js
// writes them to its dist/auth.json), with {{name}} and the rest for pages.js to fill in. pages.js serves them, with the
// scripts that make them work, and render.js draws them all to HTML files to look at.
import { Problem } from "@/components/problem"
import { List, ListRow } from "@/components/layout"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { Welcome } from "./welcome"

const logo = "{{logo}}"
const ANOTHER = "Add a second one, so losing one doesn't lock you out."

export const pages = {
  "sign-in": <Welcome logo={logo} title="{{name}}" action={{ label: "Sign In" }} />,
  "sign-in-provider": <Welcome logo={logo} title="{{name}}" action={{ label: "Sign In with {{provider}}" }} other={{ label: "Use a Passkey" }} />,
  "sign-in-throwaway": <Welcome logo={logo} title="{{name}}" action={{ label: "Sign In as {{user}}" }} />,
  "invite": <Welcome logo={logo} title="Join {{name}}" action={{ label: "Join" }} />,
  "passkeys": (
    <Welcome logo={logo} title="Passkeys" text="Each one signs you in to {{name}}." callout={ANOTHER} action={{ label: "Add a Passkey" }}>
      <Spinner className="size-5" />
      <List data-slot="passkeys" className="empty:hidden" />
    </Welcome>
  ),
  "link": <Welcome logo={logo} title="Link a Sign-In" action={{ label: "Add a Passkey" }} />,
  "linked": <Welcome logo={logo} title="Linked" text="Go back to where you started." />,
}

// What the pages' scripts draw in them: a problem, a button's spinner, a passkey's row.
export const parts = {
  problem: <Problem title="-" text="-" />,
  spinner: <Spinner className="text-current" />,
  passkey: <ListRow title="-" sub="-" trailing={<Button variant="ghost" size="sm">Remove</Button>} />,
  provider: <Button variant="secondary" size="lg" className="mt-2 min-w-56">-</Button>,
}
