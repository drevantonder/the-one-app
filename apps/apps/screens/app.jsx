// An app's page as a screen of its own: where the App Store's list has no room to show it beside, and where another
// app's link or a notification opens it.
import { createRoot } from "react-dom/client"
import { Failed, Waiting, useList } from "../lib/store.jsx"
import { AppPage } from "../lib/app-page.jsx"

function App() {
  const list = useList()
  return (
    <div className="p-4">
      {list.data
        ? <AppPage bar name={one.params.app} list={list.data} reloadList={list.reload} onOpen={(app) => one.open("app", { app })} onGone={() => one.close()} />
        : list.error ? <Failed error={list.error} retry={list.reload} /> : <Waiting rows={3} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<App />)
