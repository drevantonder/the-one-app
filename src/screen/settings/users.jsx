// Everyone in the install, each with their handle, theirs first, which opens a dialog to change it. For admins, each
// other one's role a menu on their row, with Remove there too, and adding someone new, with an invite link (or, where
// people sign in with Google, how they join). The last admin's role can't change.
//
// A Settings screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into dist/users.html,
// which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import {
  Button, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuRadioGroup, DropdownMenuRadioItem,
  DropdownMenuSeparator, DropdownMenuTrigger, Icon, Input, List, ListRow, Problem, ResponsiveDialog, Skeleton,
} from "@one/ui"

const api = one.api
const yours = one.config.yours
const said = (err) => one.plain(err).title
const roleName = (r) => (r === "admin" ? "Admin" : "Member")
const COLORS = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"]
const quiet = { autoCapitalize: "none", autoCorrect: "off", spellCheck: false }

// A steady colour and initials for someone, from their id and name.
function Avatar({ id, name, size = 36 }) {
  let h = 0
  for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0
  const initials = String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase()
  return (
    <span
      aria-hidden="true"
      className="inline-grid shrink-0 place-items-center rounded-full font-semibold text-white"
      style={{ width: size, height: size, fontSize: size * 0.4, background: COLORS[h % COLORS.length] }}
    >
      {initials}
    </span>
  )
}

// Shows after `wait` ms: grey shapes only for a slow load, so a fast one shows nothing in between.
function useLater(wait) {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const timer = setTimeout(() => setLate(true), wait)
    return () => clearTimeout(timer)
  }, [wait])
  return late
}

// A dialog that opens afresh each time: its count (a new one each time, so what was typed is gone) and whether it's open
// (set false by its own closing, kept mounted until it has gone).
function useDialog() {
  const [d, set] = useState({ n: 0, open: false })
  return [d, () => set((x) => ({ n: x.n + 1, open: true })), (open) => set((x) => ({ ...x, open }))]
}

function Users() {
  const [state, setState] = useState(null)
  const [problem, setProblem] = useState(null)
  // Roles changed but not yet taken in, shown at once: null for someone being removed.
  const [pending, setPending] = useState({})
  const [error, setError] = useState("")
  const [add, startAdd, setAddOpen] = useDialog()
  const [handle, startHandle, setHandleOpen] = useDialog()
  const slow = useLater(100)

  async function load() {
    try {
      setState(await api("GET", "/v1/state"))
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.largeTitle("Users")
    load()
    one.on("changed", load)
  }, [])

  // Changes someone's role, or with none removes them (once they say so): shown at once, and undone with why if it
  // fails.
  async function change(u, role) {
    setError("")
    if (role === null) {
      const sure = await one.alert({ title: `Remove ${u.name} from ${yours}?`, text: "Their devices stop reaching it.", action: "Remove", danger: true })
      if (!sure) return
    }
    setPending((p) => ({ ...p, [u.id]: role }))
    try {
      await (role === null ? api("POST", "/v1/remove", { user: u.id }) : api("POST", "/v1/role", { user: u.id, role }))
    } catch (e) {
      setError(said(e))
    }
    setPending(({ [u.id]: _, ...rest }) => rest)
    load()
  }

  const me = state?.user?.id
  const admin = state?.user?.role === "admin"
  // Them first, then everyone by name.
  const users = [...(state?.install?.users || [])].sort((x, y) => (y.id === me) - (x.id === me) || x.name.localeCompare(y.name))
  const admins = users.filter((u) => u.role === "admin").length
  const address = state?.install?.settings?.address
  const domain = one.config.join_domain
  const handleNow = users.find((u) => u.id === me)?.handle || ""

  let body
  if (!state) {
    body = problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />
      : slow ? <List aria-busy="true" aria-label="Loading">{[0, 1, 2].map((n) => <Placeholder key={n} />)}</List> : null
  } else {
    body = (
      <>
        <List>
          {users.filter((u) => pending[u.id] !== null).map((u) => {
            const role = pending[u.id] || u.role
            const fixed = !admin || u.id === me || (role === "admin" && admins === 1)
            const trailing = fixed
              ? <span className="text-sm text-muted-foreground">{roleName(role)}</span>
              : <Role user={u} role={role} change={(r) => r !== role && change(u, r)} />
            return (
              <ListRow
                key={u.id}
                leading={<Avatar id={u.id} name={u.name} />}
                title={`${u.name}${u.id === me ? " (you)" : ""}`}
                sub={u.handle ? `@${u.handle}` : undefined}
                trailing={trailing}
                onClick={u.id === me && u.handle ? startHandle : undefined}
              />
            )
          })}
        </List>
        {error && <Problem title={error} />}
        {admin && domain && (
          <section className="flex max-w-2xl flex-col gap-2">
            <h2 className="text-sm font-medium text-muted-foreground">Add Someone</h2>
            <p className="text-sm text-muted-foreground">
              Anyone with a Google account at {domain} joins {yours} by signing in with Google{address && <> at <b>{address.replace(/^https?:\/\//, "")}</b></>}.
            </p>
          </section>
        )}
      </>
    )
  }

  return (
    <>
      <div className="flex max-w-2xl flex-col gap-4">
        <div className="flex items-center justify-between gap-3">
          <h1 className="large-title text-3xl font-bold tracking-tight">Users</h1>
          {state && admin && !domain && <Button onClick={startAdd}><Icon name="plus" />Add Someone</Button>}
        </div>
        {body}
      </div>
      {add.n > 0 && <AddSomeone key={`add${add.n}`} open={add.open} setOpen={setAddOpen} />}
      {handle.n > 0 && <Handle key={`handle${handle.n}`} was={handleNow} open={handle.open} setOpen={setHandleOpen} saved={load} />}
    </>
  )
}

// A row's grey shape, while the list is coming.
function Placeholder() {
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <Skeleton className="size-9 rounded-full" />
      <div className="flex-1 space-y-1.5">
        <Skeleton className="h-3.5 w-1/3" />
        <Skeleton className="h-3 w-1/4" />
      </div>
    </div>
  )
}

// Someone's role, a menu: the roles, and Remove after them.
function Role({ user, role, change }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" aria-label={`${user.name}'s role`}>{roleName(role)}<Icon name="caret-up-down" /></Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup value={role} onValueChange={change}>
          <DropdownMenuRadioItem value="admin">Admin</DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="member">Member</DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={() => change(null)}>Remove {user.name}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

// Someone new: their name, then a link that lets them join, to send them.
function AddSomeone({ open, setOpen }) {
  const [name, setName] = useState("")
  const [made, setMade] = useState(null)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  async function make() {
    const who = name.trim()
    if (!who || busy) return
    setBusy(true)
    setError("")
    try {
      const { link, until } = await api("POST", "/v1/invite-link", { name: who })
      one.tap("success")
      setMade({ who, link, days: Math.max(1, Math.round((until - Date.now()) / 86_400_000)) })
    } catch (e) {
      setError(said(e))
    }
    setBusy(false)
  }
  return (
    <ResponsiveDialog
      open={open}
      onOpenChange={setOpen}
      title="Add Someone"
      action={made
        ? <Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Done</Button>
        : <Button size="sm" disabled={!name.trim() || busy} onClick={make}>Make Link</Button>}
    >
      {made ? <Link who={made.who} link={made.link} days={made.days} /> : (
        <form className="flex flex-col gap-4 p-4" onSubmit={(e) => (e.preventDefault(), make())}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Name" autoComplete="off" enterKeyHint="done" />
          <p className="text-sm text-muted-foreground">You'll get a link to send them.</p>
          {error && <Problem title={error} />}
        </form>
      )}
    </ResponsiveDialog>
  )
}

// The link made, to send them, with a button to copy it.
function Link({ who, link, days }) {
  const [copied, setCopied] = useState(false)
  async function copy() {
    await one.copy(link)
    one.tap("success")
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }
  return (
    <div className="flex flex-col items-center gap-4 p-4 text-center">
      <p className="text-sm text-muted-foreground">Send {who} this link. It works once, for {days} days.</p>
      <div className="flex max-w-full items-center gap-2 rounded-full bg-muted py-1 pr-1 pl-3">
        <span className="min-w-0 truncate font-mono text-sm text-muted-foreground select-all">{link}</span>
        <Button variant="ghost" size="icon-sm" aria-label={copied ? "Copied" : "Copy link"} onClick={copy}><Icon name={copied ? "check" : "copy"} /></Button>
      </div>
    </div>
  )
}

// A new handle for them: lowercase as it's typed, saved once it's one.
function Handle({ was, open, setOpen, saved }) {
  const [value, setValue] = useState(was)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const ok = /^[a-z0-9]+(-[a-z0-9]+)*$/.test(value) && value.length <= 39 && value !== was
  async function save() {
    if (!ok || busy) return
    setBusy(true)
    setError("")
    try {
      await api("POST", "/v1/handle", { handle: value })
      setOpen(false)
      one.tap("success")
      saved()
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="Handle" action={<Button size="sm" disabled={!ok || busy} onClick={save}>Save</Button>}>
      <form className="flex flex-col gap-4 p-4" onSubmit={(e) => (e.preventDefault(), save())}>
        <div className="flex items-center gap-2">
          <span className="text-muted-foreground">@</span>
          <Input
            autoFocus
            value={value}
            onChange={(e) => setValue(e.target.value.toLowerCase().replace(/[\s_]/g, "-"))}
            aria-label="Handle"
            autoComplete="off"
            enterKeyHint="done"
            {...quiet}
          />
        </div>
        <p className="text-sm text-muted-foreground">What apps call you by, like Repos' addresses, which move with it. Lowercase letters, digits and hyphens.</p>
        {error && <Problem title={error} />}
      </form>
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<Users />)
