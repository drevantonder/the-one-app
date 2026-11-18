// Sign In & Security, on a page of its own, for links that open it. Settings draws the same component, in the same grouped cards.
import { ListStyle, render } from "ui"
import SignInAndSecurity from "./passkeys.tsx"

render(() => (
  <ListStyle value="insetGrouped">
    <SignInAndSecurity />
  </ListStyle>
))
