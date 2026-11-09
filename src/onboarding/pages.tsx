// What auth's pages show as they open, each @one/ui's Welcome: drawn to HTML when @one/ui is built (src/screen/ui/build.js
// writes them to its dist/auth.json), with {{name}} and the rest for pages.js to fill in. pages.js serves them, with the
// scripts that make them work, and render.js draws them all to HTML files to look at.
import { Problem } from "@/components/problem"
import { List, ListRow } from "@/components/layout"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { Welcome } from "./welcome"

const logo = "{{logo}}"
const signIn = "Sign in to {{name}}"
const WORKS = "Passkeys saved in iCloud Keychain, Google Password Manager or a security key work, on a current system."
const SECOND = "Add a second passkey, on another device or a security key, so losing this one doesn't lock you out."
const ANOTHER = "Add a second passkey, on another device or a security key, so losing one doesn't lock you out."

export const pages = {
  "sign-in": <Welcome logo={logo} title={signIn} text="With your passkey." action={{ label: "Sign In with a Passkey" }}
    note="No passkey here yet? Use one from another device, or ask whoever runs {{name}} for an invite link." />,
  "sign-in-google": <Welcome logo={logo} title={signIn} text="With your Google account at {{google}}." action={{ label: "Sign In with Google" }}
    other={{ label: "Sign In with a Passkey" }} note="Admins sign in with a passkey." />,
  "sign-in-throwaway": <Welcome logo={logo} title={signIn} text="For testing: no passkey needed." action={{ label: "Sign In as {{user}}" }} />,
  "invite": <Welcome logo={logo} title="Join {{name}}" text="Make a passkey to join. It's how you sign in." action={{ label: "Make a Passkey" }} note={WORKS} />,
  "invite-google": <Welcome logo={logo} title="Join {{name}}" text="With your Google account." action={{ label: "Sign In with Google" }} />,
  "invite-throwaway": <Welcome logo={logo} title="Join {{name}}" text="For testing: no passkey needed." action={{ label: "Join" }} />,
  "joined": <Welcome logo={logo} title="You've joined {{yours}}" callout={SECOND} action={{ label: "Add Another Passkey", href: "/passkeys" }}
    other={{ label: "Open {{name}}", href: "/" }} />,
  "joined-throwaway": <Welcome logo={logo} title="You've joined {{yours}}" action={{ label: "Open {{name}}", href: "/" }} />,
  "passkeys": (
    <Welcome logo={logo} title="Passkeys" text="Each one signs you in to {{name}}." callout={ANOTHER} action={{ label: "Add a Passkey" }} note={WORKS}>
      <Spinner className="size-5" />
      <List data-slot="passkeys" className="empty:hidden" />
    </Welcome>
  ),
  "code": <Welcome logo={logo} title="Sign in with a code" text="Type the code you were shown, then sign in with your passkey. Only use a code you got yourself, on your own device."
    field={{ label: "Code", placeholder: "BCDF-GHJK" }} action={{ label: "Continue" }} />,
  "code-google": <Welcome logo={logo} title="Sign in with a code" text="Type the code you were shown, then sign in with Google or your passkey. Only use a code you got yourself, on your own device."
    field={{ label: "Code", placeholder: "BCDF-GHJK" }} action={{ label: "Continue" }} />,
  "code-signed-in": <Welcome logo={logo} title="You're signed in" text="Go back to where you got the code: it carries on by itself." />,
  "code-not-signed-in": <Welcome logo={logo} title="That didn't sign in" text="Start signing in again where you got the code." />,
}

// What the pages' scripts draw in them: a problem, a button's spinner, a passkey's row.
export const parts = {
  problem: <Problem title="-" text="-" />,
  spinner: <Spinner className="text-current" />,
  passkey: <ListRow title="-" sub="-" trailing={<Button variant="ghost" size="sm">Remove</Button>} />,
}
