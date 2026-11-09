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
  if (!app || !path) return <Empty>This page isn’t here.</Empty>
  if (!file.data) return <Pending call={file} rows={1} />
  return <CodeView text={file.data.text} path={file.data.path} />
}

createRoot(document.getElementById("root")).render(<File />)
