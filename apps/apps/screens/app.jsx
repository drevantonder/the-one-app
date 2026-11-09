// An app's page as a screen of its own: where the App Store's list has no room to show it beside, and where another
// app's link or a notification opens it.
import { createRoot } from "react-dom/client"
import { Pending } from "@one/ui"
import { useApps } from "../lib/store.jsx"
import { AppPage } from "../lib/app-page.jsx"

function App() {
  const apps = useApps()
  return apps.data
    ? <AppPage bar name={one.params.app} apps={apps} onOpen={(app) => one.open("app", { app })} onGone={() => one.close()} />
    : <Pending call={apps} />
}

createRoot(document.getElementById("root")).render(<App />)
