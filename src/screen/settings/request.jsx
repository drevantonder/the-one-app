// A request for an item an app or an agent may use: who asks, in the title; what they'd get and where it goes, in one
// grey line; then the user's items that have the fields it wants, to pick one, or new values, which go to the node and
// nowhere else. Allow is the only way to say yes, and the sheet's Cancel declines (screens.js). The asker never sees a
// secret. It closes itself with whether it was allowed, as it is when it's answered somewhere else (another device, an
// agent's CLI).
//
// A client's own screen written in React (see vault.jsx): built by @one/ui's build into dist/request.html.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, CopyButton, Input, Label, ListSkeleton, Problem, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Spinner } from "@one/ui"
import { AGENTS, appNamed, asking } from "./agents.js"

const api = one.api
const { scope, id } = one.params || {}
const find = () => api("GET", `/v1/vault/requests?scope=${encodeURIComponent(scope || "")}&id=${encodeURIComponent(id || "")}`).then((l) => l[0])

/// A field's name as a user reads it: "api_key" → "API key".
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

const NEW = "new"

// What a request starts on. An app's declared need has fields only its own kind of item has, so the first that fits; an
// agent's can be any `token`, so only the item of the name it asked for. Otherwise a new one.
const startsOn = (r) => (r.need ? r.matches?.[0] : r.matches?.find((m) => m.title.toLowerCase() === r.title.toLowerCase()))?.id ?? NEW

function Request() {
  const [request, setRequest] = useState(null)
  const [apps, setApps] = useState([])
  const [problem, setProblem] = useState(null)
  const [picked, setPicked] = useState(NEW)
  const [values, setValues] = useState({})
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const now = useRef({ busy, request })
  now.current = { busy, request }

  async function load() {
    try {
      const [r, a] = await Promise.all([find(), api("GET", "/v1/apps").catch(() => [])])
      if (!r) throw new Error("Can’t find this request: it may be for someone else.")
      setRequest(r)
      setApps(a)
      setPicked(startsOn(r))
      setProblem(null)
      one.title(`${asking(r, a)} needs a key`)
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

  async function allow(e) {
    e.preventDefault()
    if (busy || !ready) return
    setBusy(true)
    setError("")
    const body = { scope: request.scope, id: request.id }
    if (picked !== NEW) body.item = picked
    else body.fields = values
    try {
      await api("POST", "/v1/vault/answer", body)
      setValues({})
      one.tap("success")
      one.close(true)
    } catch (err) {
      setError(one.plain(err).title)
      setBusy(false)
    }
  }

  if (!request) {
    return problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: () => (one.tap(), setProblem(null), load()) }} /> : <ListSkeleton rows={3} />
  }

  const matches = request.matches || []
  const wanted = request.fields || []
  const ready = !busy && (picked !== NEW || wanted.every((f) => values[f.name]))
  const set = (name, value) => setValues((v) => ({ ...v, [name]: value }))

  if (request.state !== "waiting") {
    return <p className="text-sm text-muted-foreground">{request.state === "set" ? "Allowed." : request.state === "declined" ? "Declined." : `This request expired. Ask ${request.agent || request.by} to ask again.`}</p>
  }

  return (
    <form className="flex max-w-lg flex-col gap-4" onSubmit={allow}>
      <p className="text-sm text-muted-foreground">{request.why && <><Why why={request.why} />{" "}</>}{granted(request, apps, matches.find((m) => m.id === picked))}</p>
      {matches.length > 0 && (
        <Select value={picked} onValueChange={setPicked}>
          <SelectTrigger className="w-full" aria-label="Which item"><SelectValue /></SelectTrigger>
          <SelectContent>
            {matches.map((m) => <SelectItem key={m.id} value={m.id}>{m.title}</SelectItem>)}
            <SelectItem value={NEW}>New Item</SelectItem>
          </SelectContent>
        </Select>
      )}
      {picked === NEW && wanted.map((f, i) => <Field key={f.name} first={i === 0} label={fieldLabel(f.name)} secret={f.secret} value={values[f.name] || ""} onChange={(v) => set(f.name, v)} />)}
      {error && <Problem title={error} />}
      <Button type="submit" size="lg" className="w-full" disabled={!ready}>{busy ? <><Spinner />Allowing…</> : "Allow"}</Button>
    </form>
  )
}

/// What Allow gives, to what, in a sentence: the item, for its page if that isn't the user's own, and for a new one the
/// only sites it can go to.
function granted(r, apps, item) {
  const to = r.app === AGENTS.name ? "Your agents" : appNamed(apps, r.app)?.title || r.app
  const page = r.scope.startsWith("user:") ? "" : ` for ${r.scope_name}`
  const sites = !item && r.hosts?.length ? `, sent only to ${r.hosts.join(", ")}` : ""
  return `${to} can use ${item?.title || r.title}${page}${sites}.`
}

/// A value to type, its name over it.
function Field({ label, value, onChange, secret, first }) {
  const name = `field-${label}`
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={name}>{label}</Label>
      <Input
        id={name}
        type={secret ? "password" : "text"}
        autoComplete={secret ? "new-password" : "off"}
        spellCheck={false}
        autoFocus={first}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Request />)
