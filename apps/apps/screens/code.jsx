// An app's code: the folders and files of its source, folders first; at the top, the module it runs as.
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { Empty, Icon, List, ListRow, Pending, fileSize, useCall } from "@one/ui"

const { app, dir = "" } = one.params

function folder(files, dir) {
  const here = new Map()
  for (const f of files.filter((f) => f.path.startsWith("src/" + dir))) {
    const [first, ...rest] = f.path.slice(4 + dir.length).split("/")
    here.set(rest.length ? first + "/" : first, rest.length ? { dir: dir + first + "/" } : f)
  }
  const natural = new Intl.Collator(undefined, { numeric: true }).compare
  const names = [...here.keys()].sort((a, b) => b.endsWith("/") - a.endsWith("/") || natural(a, b))
  const built = files.find((f) => f.path === "app.js")
  if (!dir && built) names.push("app.js, as it runs"), here.set("app.js, as it runs", built)
  return names.map((n) => [n, here.get(n)])
}

function Code() {
  const code = useCall("code", { app })
  useEffect(() => {
    one.title(dir.replace(/\/$/, "").split("/").pop() || "Code")
  }, [])
  if (!app) return <Empty>This page isn't here. The link may be old or mistyped.</Empty>
  if (!code.data) return <div className="p-4"><Pending call={code} rows={4} /></div>
  return (
    <div className="p-4">
      <List>
        {folder(code.data.files, dir).map(([name, f]) => (
          <ListRow
            key={name}
            leading={<Icon name={f.dir ? "folder" : "file"} className="size-5 text-muted-foreground" />}
            title={name.replace(/\/$/, "")}
            trailing={!f.dir && <span className="text-muted-foreground">{fileSize(f.size)}</span>}
            onClick={() => (f.dir ? one.open("code", { app, dir: f.dir }) : one.open("file", { app, path: f.path }))}
          />
        ))}
      </List>
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Code />)
