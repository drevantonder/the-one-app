// A request for an item an app may use: who asks and why, then the person's items that have the fields it wants, to
// pick one, or a new one, whose values go to the node and nowhere else. The asker never sees a secret. It closes
// itself with whether it was allowed, as it is when it's answered somewhere else (another device, an agent's CLI).
//
// A client's own screen written in React (see vault.jsx): built by @one/ui's build into dist/request.html.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, CopyButton, Icon, Input, List, ListRow, ListSkeleton, Problem, Spinner } from "@one/ui"

const api = one.api
const { scope, id } = one.params || {}
const find = () => api("GET", `/v1/vault/requests?scope=${encodeURIComponent(scope || "")}&id=${encodeURIComponent(id || "")}`).then((l) => l[0])

/// A field's name as a person reads it: "api_key" → "API key".
function fieldLabel(name) {
  const words = String(name).replace(/[_-]+/g, " ").trim().split(/\s+/)
  return words.map((w, i) => (/^(api|url|id|ssh|pem|aws|s3|ip)$/i.test(w) ? w.toUpperCase() : i ? w.toLowerCase() : w[0].toUpperCase() + w.slice(1).toLowerCase())).join(" ")
}

/// The asker's reason, a web address in it with Copy beside it, as one to paste somewhere else.
const Why = ({ why }) =>
  String(why).split(/(https?:\/\/[^\s]+)/).map((part, i) => (i % 2 ? (
    <span key={i}>
      <span className="break-all text-foreground">{part}</span>
      <CopyButton text={part} label="" className="-my-1 align-middle" />
    </span>
  ) : part))

const Footer = ({ children }) => <p className="-mt-3 px-3 text-xs text-muted-foreground">{children}</p>

function Request() {
  const [request, setRequest] = useState(null)
  const [apps, setApps] = useState([])
  const [problem, setProblem] = useState(null)
  const [picked, setPicked] = useState(null)
  const [values, setValues] = useState({})
  const [error, setError] = useState("")
  // "allow" or "decline", from the tap until the answer is in.
  const [busy, setBusy] = useState(false)
  const now = useRef({ busy, request })
  now.current = { busy, request }

  async function load() {
    try {
      const [r, a] = await Promise.all([find(), api("GET", "/v1/apps").catch(() => [])])
      if (!r) throw new Error("Can’t find this request: it may be for someone else.")
      setRequest(r)
      setApps(a)
      setPicked(r.matches?.[0]?.id ?? null)
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }

  useEffect(() => {
    one.title("Vault")
    load()
    // Answered somewhere else: the sheet goes.
    const t = setInterval(async () => {
      const { busy, request } = now.current
      if (busy || request?.state !== "waiting") return
      const fresh = await find().catch(() => null)
      if (fresh && fresh.state !== "waiting") one.close(fresh.state === "set")
    }, 2100)
    return () => clearInterval(t)
  }, [])

  async function answer(declined) {
    if (busy) return
    setBusy(declined ? "decline" : "allow")
    setError("")
    const body = { scope: request.scope, id: request.id }
    if (declined) body.declined = true
    else if (picked) body.item = picked
    else {
      const { _title, ...fields } = values
      if (_title?.trim()) body.title = _title.trim()
      body.fields = fields
    }
    try {
      await api("POST", "/v1/vault/answer", body)
      setValues({})
      one.tap(declined ? "selection" : "success")
      one.close(!declined)
    } catch (err) {
      setError(one.plain(err).title)
      setBusy(false)
    }
  }

  if (!request) {
    return problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: () => (one.tap(), setProblem(null), load()) }} /> : <ListSkeleton rows={3} />
  }

  const a = apps.find((x) => x.name === request.app)
  const wanted = request.fields || []
  const what = request.title || ""
  const asker = request.agent || request.by
  const ready = !busy && (picked !== null || wanted.every((f) => values[f.name]))
  const set = (name, value) => setValues((v) => ({ ...v, [name]: value }))

  return (
    <div className="flex max-w-2xl flex-col gap-5">
      <List>
        <div className="flex items-start gap-3 px-3 py-3 text-sm">
          <AppIcon app={a || { name: request.app }} size={44} />
          <div className="min-w-0 flex-1">
            <div className="break-words font-semibold">{a ? `${a.title || a.name} wants ${what}` : what}</div>
            {request.why && <div className="break-words text-muted-foreground"><Why why={request.why} /></div>}
          </div>
        </div>
      </List>
      <Footer>{request.agent ? `${asker} · ${request.scope_name}` : request.scope_name}</Footer>
      {request.state === "waiting" ? (
        <>
          <List>
            {(request.matches || []).map((m) => (
              <Pick key={m.id} on={picked === m.id} run={() => setPicked(m.id)} leading={<AppIcon app={{ name: m.title, title: m.title }} />} title={m.title} sub={m.type} />
            ))}
            <Pick on={picked === null} run={() => setPicked(null)} leading={<span className="grid size-9 shrink-0 place-items-center"><Icon name="plus" className="size-5" /></span>} title="New Item" />
          </List>
          {picked === null && (
            <>
              <List>
                <Field label="Title" value={values._title ?? what} onChange={(v) => set("_title", v)} />
                {wanted.map((f) => <Field key={f.name} label={fieldLabel(f.name)} secret={f.secret} value={values[f.name] || ""} onChange={(v) => set(f.name, v)} />)}
              </List>
              {!!request.hosts?.length && <Footer>Sent only to {request.hosts.join(", ")}</Footer>}
            </>
          )}
          <List>
            <button type="button" className="flex w-full items-center justify-center gap-2 px-3 py-2.5 text-sm font-medium text-destructive outline-none hover:bg-accent/50 focus-visible:bg-accent/50 disabled:opacity-50" disabled={!!busy} onClick={() => answer(true)}>
              {busy === "decline" ? <><Spinner />Declining…</> : "Decline"}
            </button>
          </List>
          {error && <Problem title={error} />}
          <div className="flex justify-end">
            <Button size="lg" className="min-w-30" disabled={!ready} onClick={() => answer(false)}>
              {busy === "allow" ? <><Spinner />Allowing…</> : "Allow"}
            </Button>
          </div>
        </>
      ) : (
        <List>
          <ListRow title={<span className="font-normal text-muted-foreground">{request.state === "set" ? "Allowed." : request.state === "declined" ? "Declined." : `This request expired. Ask ${asker} to ask again.`}</span>} wrap />
        </List>
      )}
    </div>
  )
}

/// A row that picks what it's for, with a check on the one picked, and no chevron: it opens nothing.
function Pick({ on, run, leading, title, sub }) {
  return (
    <button type="button" aria-pressed={on} onClick={run} className="flex w-full min-w-0 cursor-pointer items-center gap-3 px-3 py-2.5 text-left text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50">
      {leading}
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{title}</span>
        {sub && <span className="block truncate text-muted-foreground">{sub}</span>}
      </span>
      {on && <Icon name="check" className="size-4 shrink-0" />}
    </button>
  )
}

/// A field to type a value in, its name beside it.
function Field({ label, value, onChange, secret }) {
  return (
    <label className="flex items-center gap-3 px-3 py-1.5 text-sm">
      <span className="w-28 shrink-0 text-muted-foreground">{label}</span>
      <Input
        className="border-0 bg-transparent px-0 text-sm shadow-none focus-visible:ring-0 md:text-sm dark:bg-transparent"
        type={secret ? "password" : "text"}
        autoComplete={secret ? "new-password" : "off"}
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  )
}

createRoot(document.getElementById("root")).render(<Request />)
