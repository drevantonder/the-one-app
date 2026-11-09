// Developer mode, on this device alone: every screen's bar has a </> that opens Developer (src/screen/settings/devtools.jsx).
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/developer.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { List, Skeleton, Switch } from "@one/ui"

function Developer() {
  // Not known until the device says.
  const [on, setOn] = useState(null)

  useEffect(() => {
    one.largeTitle("Developer")
    one.developer().then(setOn)
  }, [])

  // Shown at once; a device that couldn't change it puts it back.
  async function change(now) {
    one.tap()
    setOn(now)
    setOn(await one.developer(now).catch(() => !now))
  }

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <h1 className="large-title text-3xl font-bold tracking-tight">Developer</h1>
      <List>
        <label className="flex cursor-pointer items-center gap-3 px-3 py-3 text-sm">
          <span className="min-w-0 flex-1 font-medium">Developer Mode</span>
          {on === null ? <Skeleton className="h-[1.15rem] w-8 rounded-full" /> : <Switch checked={on} onCheckedChange={change} />}
        </label>
      </List>
      <p className="px-1 text-sm text-muted-foreground">
        Puts a &lt;/&gt; at the top of every screen on this device, to see where its time goes.
      </p>
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Developer />)
