// Everyone in the network, each with their handle, theirs first, which opens a dialog to change it. For admins, each
// other one's role a menu on their row, with Remove there too, and adding someone new, with an invite link (or, where
// users sign in with Google, how they join). The last admin's role can't change.
//
// A Settings screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into dist/users.html,
// which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Avatar, Button, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger, Grid, GridRow, Hint, Icon, Input, LargeTitle, List, ListRow, ListSkeleton, Problem, ResponsiveDialog, Section, useDialog } from "@one/ui"

const api = one.api
const yours = one.config.yours
const said = (err) => one.plain(err).title
const roleName = (r) => (r === "admin" ? "Admin" : "Member")
const quiet = { autoCapitalize: "none", autoCorrect: "off", spellCheck: false }

function Users() {
  const [state, setState] = useState(null)
  const [problem, setProblem] = useState(null)
  // Roles changed but not yet taken in, shown at once: null for someone being removed.
  const [pending, setPending] = useState({})
  const [error, setError] = useState("")
  const [add, startAdd, setAddOpen] = useDialog()
  const [handle, startHandle, setHandleOpen] = useDialog()

  async function load() {
    try {
      setState(await api("GET", "/v1/state"))
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
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
      await (role === null ? api("POST", "/v1/devices/remove", { user: u.id }) : api("POST", "/v1/role", { user: u.id, role }))
    } catch (e) {
      setError(said(e))
    }
    setPending(({ [u.id]: _, ...rest }) => rest)
    load()
  }

  async function withdraw(i) {
    setError("")
    try {
      await api("POST", "/v1/devices/invite/withdraw", { user: i.user })
    } catch (e) {
      setError(said(e))
    }
    load()
  }

  const me = state?.user?.id
  const admin = state?.user?.role === "admin"
  // Them first, then everyone by name.
  const users = [...(state?.network?.users || [])].sort((x, y) => (y.id === me) - (x.id === me) || x.name.localeCompare(y.name))
  const admins = users.filter((u) => u.role === "admin").length
  const domain = one.config.join_domain
  const handleNow = users.find((u) => u.id === me)?.handle || ""

  let body
  if (!state) {
    body = problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />
      : <ListSkeleton rows={3} />
  } else {
    body = (
      <>
        <Grid min="22rem" max={4} gap="0.25rem">
          {users.filter((u) => pending[u.id] !== null).map((u) => {
            const role = pending[u.id] || u.role
            const fixed = !admin || u.id === me || (role === "admin" && admins === 1)
            const trailing = fixed
              ? <span className="text-sm text-muted-foreground">{roleName(role)}</span>
              : <Role user={u} role={role} change={(r) => r !== role && change(u, r)} />
            return (
              <GridRow
                key={u.id}
                leading={<Avatar id={u.id} name={u.name} size={36} />}
                title={`${u.name}${u.id === me ? " (you)" : ""}`}
                sub={[u.handle && `@${u.handle}`, u.recovery_holder && "Holds the recovery key"].filter(Boolean).join(" · ") || undefined}
                trailing={trailing}
                onClick={u.id === me && u.handle ? () => startHandle() : undefined}
              />
            )
          })}
        </Grid>
        {!!state.network?.email_invites?.length && (
          <Section title="Invited">
            <List>
              {state.network.email_invites.map((i) => (
                <ListRow key={i.user} title={i.name} sub={i.email} trailing={<Button variant="ghost" size="sm" onClick={() => withdraw(i)}>Cancel Invite</Button>} />
              ))}
            </List>
          </Section>
        )}
        {error && <Problem title={error} />}
        {admin && domain && <p className="max-w-2xl text-sm text-muted-foreground">Anyone else at {domain} can sign in and ask.</p>}
      </>
    )
  }

  return (
    <>
      <div className="flex flex-col gap-4">
        <LargeTitle action={state && admin && <Button onClick={() => startAdd()}>Add Someone</Button>}>Users</LargeTitle>
        {body}
      </div>
      {add.n > 0 && <AddSomeone key={`add${add.n}`} open={add.open} setOpen={setAddOpen} providers={!!state?.network?.settings?.sign_in_clients} />}
      {handle.n > 0 && <Handle key={`handle${handle.n}`} was={handleNow} open={handle.open} setOpen={setHandleOpen} saved={load} />}
    </>
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

// Someone new. Where the network has providers: their name and email, and they sign in with it and go straight in.
// Elsewhere: their name, then a link that lets them join, to send them.
function AddSomeone({ open, setOpen, providers }) {
  const [name, setName] = useState("")
  const [email, setEmail] = useState("")
  const [made, setMade] = useState(null)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  async function make() {
    const who = name.trim()
    if (!who || busy) return
    setBusy(true)
    setError("")
    try {
      const { link, until } = await api("POST", "/v1/devices/invite", { name: who, ...(email.trim() ? { email: email.trim() } : {}) })
      one.tap("success")
      setMade({ who, link, email: email.trim(), days: Math.max(1, Math.round((until - Date.now()) / 86_400_000)) })
    } catch (e) {
      setError(said(e))
    }
    setBusy(false)
  }
  return (
    <ResponsiveDialog
      open={open}
      onOpenChange={setOpen}
      title={made ? (made.link ? `Send ${made.who} This Link` : `${made.who} Can Sign In`) : "Add Someone"}
      action={made
        ? <Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Done</Button>
        : <Button size="sm" disabled={!name.trim() || (providers && !email.trim()) || busy} onClick={make}>{providers ? "Invite" : "Make Link"}</Button>}
    >
      {made ? (made.link ? <Link link={made.link} days={made.days} /> : <p className="p-4 text-sm text-muted-foreground">With {made.email}, for {made.days} day{made.days > 1 ? "s" : ""}.</p>) : (
        <form className="flex flex-col gap-4 p-4" onSubmit={(e) => (e.preventDefault(), make())}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Name" autoComplete="off" enterKeyHint="done" />
          {providers && <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="Email" aria-label="Email" autoComplete="off" enterKeyHint="done" />}
          {error && <Problem title={error} />}
        </form>
      )}
    </ResponsiveDialog>
  )
}

// The link made, to send them: copying it is what's left to do.
function Link({ link, days }) {
  const [copied, setCopied] = useState(false)
  async function copy() {
    await one.copy(link)
    one.tap("success")
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }
  return (
    <div className="flex flex-col gap-4 p-4">
      <div className="min-w-0">
        <p className="truncate font-mono text-sm select-all">{link}</p>
        <Hint>It works once, for {days} day{days > 1 ? "s" : ""}.</Hint>
      </div>
      <Button onClick={copy}><Icon name={copied ? "check" : "copy"} />{copied ? "Copied" : "Copy Link"}</Button>
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
      await api("POST", "/v1/groups/handle", { handle: value })
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
        <Hint>Lowercase letters, numbers and hyphens. Your Repos addresses change with it.</Hint>
        {error && <Problem title={error} />}
      </form>
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<Users />)
