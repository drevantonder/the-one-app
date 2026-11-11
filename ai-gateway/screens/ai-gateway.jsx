// Settings › AI Gateway: the default models, whose key pays for each company's, and where to see what calls spent.
// One page of Lists (DESIGN.md's Settings shape). Everyone's keys are an admin's to add, in Everyone's Vault; anyone
// adds their own, in their own Vault, which pays for their calls over Everyone's.
//
// A Settings screen written in React: built by @one/ui's build into dist/ai-gateway.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Icon, Input, LargeTitle, List, ListRow, ListSkeleton, LoadFailure, Problem, ResponsiveDialog, Section, Spinner } from "@one/ui"

const api = one.api
const said = (err) => one.plain(err).title

// Each company's tile: Workers AI in Cloudflare's orange, the rest in their own colours.
const COLORS = { "workers-ai": "#f38020", google: "#4285f4", anthropic: "#d97757", openai: "#10a37f", openrouter: "#6467f2" }
const tile = (c) => ({ name: c.id, title: c.title, icon: c.id === "workers-ai" ? "cloud" : "sparkle", color: COLORS[c.id] || "#8e8e93" })

// A model id as people read it: its own name, with where it runs when that's not the company's own.
function modelName(id) {
  const [company, ...rest] = String(id || "").split("/")
  const model = rest.join("/").replace(/^@cf\/[^/]+\//, "")
  const name = model.split(/[-_]/).map((w) => (/^\d/.test(w) ? w : w[0]?.toUpperCase() + w.slice(1))).join(" ")
  return company === "workers-ai" ? `${name} on Workers AI` : name
}
const WHOSE = { yours: "Yours", everyone: "Everyone's", built_in: "Built in" }

function AiGateway() {
  const [models, setModels] = useState(null)
  const [admin, setAdmin] = useState(false)
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")
  const [setting, setSetting] = useState(null)

  async function load() {
    try {
      const [got, state] = await Promise.all([api("POST", "/api/ai-gateway/models", {}), api("GET", "/api/state")])
      setModels(got)
      setAdmin(state.user?.role === "admin")
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.on("changed", load)
    load()
  }, [])

  // An admin adds Everyone's key, anyone else their own: a request in that Vault, answered in the Vault's own sheet,
  // so the key is typed there and never passes through this screen.
  async function addKey(c) {
    const scope = admin ? "global" : "user"
    setError("")
    const fields = c.id === "workers-ai" ? { account_id: { secret: false }, api_key: { secret: true } } : { api_key: { secret: true } }
    try {
      const asked = await api("POST", "/api/vault/request", { scope, app: "node", need: c.key_kind, title: `${c.title} Key`, fields, hosts: [c.host], why: admin ? `Pays for everyone's calls to ${c.title}'s models.` : `Pays for your calls to ${c.title}'s models.` })
      one.open("request", { id: asked.id, scope: asked.scope })
    } catch (e) {
      setError(said(e))
    }
  }

  if (!models) {
    return (
      <div className="flex max-w-2xl flex-col gap-6">
        <LargeTitle>AI Gateway</LargeTitle>
        {problem ? <LoadFailure problem={problem} retry={load} /> : <ListSkeleton rows={4} />}
      </div>
    )
  }
  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <LargeTitle>AI Gateway</LargeTitle>
      <p className="-mt-4 px-1 text-muted-foreground">How your apps use AI models.</p>
      {error && <Problem title={error} />}

      <Section title="Default Models">
        <List>
          {["chat", "decide"].map((kind) => {
            const d = models.defaults[kind]
            return <ListRow key={kind} title={kind === "chat" ? "Chat" : "Decide"} sub={`${modelName(d.model)} · ${WHOSE[d.whose]}`} trailing={<Icon name="caret-right" className="size-4 text-muted-foreground" />} onClick={() => setSetting(kind)} />
          })}
        </List>
      </Section>

      <Section title="Keys">
        <List>
          {models.companies.map((c) => (
            <ListRow
              key={c.id}
              leading={<AppIcon app={tile(c)} size={30} />}
              title={c.title}
              sub={{ network: c.id === "workers-ai" ? "Everyone's · your Cloudflare account" : "Everyone's", user: "Yours" }[c.paid] || "No key"}
              trailing={(c.paid === null || (!admin && c.paid === "network")) && <Button size="sm" variant="outline" onClick={() => addKey(c)}>{admin ? "Add Key" : "Add Mine"}</Button>}
            />
          ))}
        </List>
      </Section>

      <Section title="Spending">
        <List>
          <ListRow title="What each call used" sub="In Insights, by app, model and person." trailing={<Icon name="caret-right" className="size-4 text-muted-foreground" />} onClick={() => one.open("insights", {})} />
        </List>
      </Section>
      {setting && <SetDefault kind={setting} models={models} admin={admin} close={() => setSetting(null)} done={load} />}
    </div>
  )
}

// Sets a kind's default model: the user's own, or with an admin's choice, Everyone's.
function SetDefault({ kind, models, admin, close, done }) {
  const d = models.defaults[kind]
  const [model, setModel] = useState(d.whose === "built_in" ? "" : d.model)
  const [everyone, setEveryone] = useState(admin && d.whose !== "yours")
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState("")

  async function save() {
    setBusy(true)
    setProblem("")
    try {
      await api("POST", "/api/ai-gateway/set", { [kind]: model.trim(), everyone })
      done()
      close()
    } catch (e) {
      setProblem(said(e))
    }
    setBusy(false)
  }

  return (
    <ResponsiveDialog open onOpenChange={(o) => !o && close()} title={kind === "chat" ? "Default Chat Model" : "Default Decide Model"} action={<Button size="sm" disabled={busy} onClick={save}>{busy && <Spinner />}Save</Button>}>
      <form className="flex flex-col gap-4 p-4" onSubmit={(e) => (e.preventDefault(), save())}>
        <Input value={model} onChange={(e) => setModel(e.target.value)} placeholder={kind === "chat" ? "google/gemini-3.5-flash-lite" : "workers-ai/@cf/cloudflare/clef-flash"} autoFocus />
        <p className="px-1 text-sm text-muted-foreground">Apps that don’t name a model get this one. Leave it empty for {everyone ? "the built-in default" : "Everyone’s"}.</p>
        {admin && (
          <List>
            <ListRow title="For everyone" trailing={everyone && <Icon name="check" className="size-4" />} onClick={() => setEveryone(true)} />
            <ListRow title="Just for me" trailing={!everyone && <Icon name="check" className="size-4" />} onClick={() => setEveryone(false)} />
          </List>
        )}
        {problem && <Problem title={problem} />}
        <button type="submit" hidden />
      </form>
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<AiGateway />)
