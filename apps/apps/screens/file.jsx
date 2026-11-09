// A file of an app's code, in the code viewer.
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { CodeView, Empty, Pending, useCall } from "@one/ui"

const { app, path } = one.params

function File() {
  const file = useCall("code", { app, path })
  useEffect(() => {
    one.title((path || "").split("/").pop() || "Code")
  }, [])
  if (!app || !path) return <Empty>This page isn't here. The link may be old or mistyped.</Empty>
  if (!file.data) return <div className="p-4"><Pending call={file} rows={1} /></div>
  return (
    <div className="p-4">
      <CodeView text={file.data.text} path={file.data.path} />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<File />)
