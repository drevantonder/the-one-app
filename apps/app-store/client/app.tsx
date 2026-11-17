// The App Store's own page: its screen (store.tsx), whose Back leaves it.
import { render } from "@solidjs/web"
import { leave } from "ui"
import Store from "./store.tsx"

render(() => <Store onBack={leave} />, document.body)
