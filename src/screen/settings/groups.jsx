// The groups this person is in and is invited to, and making one; and one group, open beside the list where there's
// room, else in a sheet: its handle, members and invites, each member's role a menu on their row for its admins. The
// last admin's role can't change. A group's handle is changed in a dialog, by its admins.
//
// A Settings screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into dist/groups.html,
// which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import {
  Avatar, AppIcon, Button, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuRadioGroup, DropdownMenuRadioItem,
  DropdownMenuSeparator, DropdownMenuTrigger, Empty, Icon, Input, List, ListRow, ListSkeleton, Problem, ResponsiveDialog,
  Split, Tabs, TabsList, TabsTrigger, useDialog,
} from "@one/ui"

const api = one.api
const said = (err) => one.plain(err).title
const roleName = (r) => (r === "admin" ? "Admin" : "Member")
const quiet = { autoCapitalize: "none", autoCorrect: "off", spellCheck: false }
// Groups' own icon, as Settings draws it.
const GROUPS = { name: "groups", icon: "users-three", color: "#34c759" }

// "Ann, Bo and Cy".
const names = (state, list) => new Intl.ListFormat(undefined, { type: "conjunction" }).format(list.map((m) => userName(state, m.user)))
const userName = (state, id) => state.network?.users.find((u) => u.id === id)?.name || "Someone"

function Groups() {
  const [state, setState] = useState(null)
  const [problem, setProblem] = useState(null)
  // Opened for a group (`{ group }`, from another screen), with it open.
  const [openId, setOpenId] = useState(one.params?.group || null)
  // Roles changed but not yet taken in, shown at once, by group and person: null for someone being removed.
  const [pending, setPending] = useState({})
  // Invites sent but not yet taken in.
  const [sent, setSent] = useState([])
  const [joining, setJoining] = useState(null)
  const [error, setError] = useState("")
  const [create, startCreate, setCreateOpen] = useDialog()
  const [invite, startInvite, setInviteOpen] = useDialog()
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
    one.largeTitle("Groups")
    load()
    one.on("changed", load)
  }, [])

  const me = state?.user?.id
  const groups = state?.network?.groups || []
  const invited = groups.filter((g) => g.invited.some((m) => m.user === me))
  const mine = groups.filter((g) => g.role)
  const selected = openId && groups.find((g) => g.id === openId && (g.role || g.invited.some((m) => m.user === me)))

  // Changes a member's role, or with none removes them: shown at once, and undone with why if it fails.
  async function change(group, user, role) {
    const key = `${group}:${user}`
    setError("")
    setPending((p) => ({ ...p, [key]: role }))
    try {
      await api("POST", "/v1/member", { group, user, role })
      // Leaving, there's nothing left to show here.
      if (user === me && role === null) setOpenId(null)
    } catch (e) {
      setError(said(e))
    }
    await load()
    setPending(({ [key]: _, ...rest }) => rest)
  }

  // Joining says so in its button until the group shows among theirs.
  async function accept(id) {
    if (joining) return
    setJoining(id)
    setError("")
    try {
      await api("POST", "/v1/accept", { group: id })
      one.tap("success")
    } catch (e) {
      setError(said(e))
    }
    await load()
    setJoining(null)
  }

  // Shown among the invited at once; taken out again, with why, if it fails.
  async function send(group, user, role) {
    const item = { group, user, role }
    setError("")
    setSent((all) => [...all, item])
    one.tap("success")
    try {
      await api("POST", "/v1/invite", { group, user, role })
    } catch (e) {
      setError(said(e))
    }
    await load()
    setSent((all) => all.filter((x) => x !== item))
  }

  const empty = state && !mine.length && !invited.length
  let body
  if (!state) {
    body = problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />
      : <ListSkeleton rows={3} />
  } else if (empty) {
    body = <Empty action={{ label: "New Group", run: () => startCreate() }}>A group shares its apps and data, on a page of its own.</Empty>
  } else {
    body = (
      <>
        {openId && !selected && <Problem title="You're not in this group." />}
        {!!invited.length && (
          <Section title="Invited">
            <List>
              {invited.map((g) => (
                <ListRow
                  key={g.id}
                  leading={<AppIcon app={GROUPS} size={36} />}
                  title={g.name}
                  sub={names(state, g.members)}
                  trailing={<Button variant="secondary" size="sm" disabled={!!joining} onClick={() => accept(g.id)}>{joining === g.id ? "Joining…" : "Join"}</Button>}
                />
              ))}
            </List>
          </Section>
        )}
        {!!mine.length && (
          <Section title={invited.length ? "Yours" : undefined}>
            <List>
              {mine.map((g) => (
                <ListRow
                  key={g.id}
                  leading={<AppIcon app={GROUPS} size={36} />}
                  title={g.name}
                  sub={names(state, g.members)}
                  trailing={g.role === "admin" ? <span className="text-sm text-muted-foreground">Admin</span> : undefined}
                  selected={g.id === openId}
                  onClick={() => setOpenId(g.id)}
                />
              ))}
            </List>
          </Section>
        )}
        {error && !selected && <Problem title={error} />}
      </>
    )
  }

  return (
    <>
      <Split
        title={selected?.name}
        onClose={() => setOpenId(null)}
        list={
          <div className="flex max-w-2xl flex-col gap-4">
            <div className="flex items-center justify-between gap-3">
              <h1 className="large-title text-3xl font-bold tracking-tight">Groups</h1>
              {state && !empty && <Button onClick={() => startCreate()}><Icon name="plus" />New Group</Button>}
            </div>
            {body}
          </div>
        }
        detail={selected && (
          <Group
            key={selected.id}
            g={selected}
            state={state}
            me={me}
            pending={pending}
            sent={sent.filter((s) => s.group === selected.id)}
            error={error}
            change={(user, role) => change(selected.id, user, role)}
            invite={() => startInvite(selected.id)}
            handle={() => startHandle(selected.id)}
          />
        )}
      />
      {create.n > 0 && <NewGroup key={`create${create.n}`} open={create.open} setOpen={setCreateOpen} made={async (id) => (await load(), setOpenId(id))} />}
      {invite.n > 0 && <Invite key={`invite${invite.n}`} g={groups.find((g) => g.id === invite.for)} state={state} sent={sent} open={invite.open} setOpen={setInviteOpen} send={(user, role) => send(invite.for, user, role)} />}
      {handle.n > 0 && <Handle key={`handle${handle.n}`} g={groups.find((g) => g.id === handle.for)} open={handle.open} setOpen={setHandleOpen} saved={load} />}
    </>
  )
}

// A heading over a group of rows.
const Section = ({ title, children }) => (
  <section className="flex flex-col gap-2">
    {title && <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>}
    {children}
  </section>
)

// One group: its handle, its members, its invites, and inviting or leaving.
function Group({ g, state, me, pending, sent, error, change, invite, handle }) {
  const admin = g.role === "admin"
  const admins = g.members.filter((m) => m.role === "admin").map((m) => m.user)
  const lastAdmin = admins.length === 1 && admins[0] === me
  const members = g.members.filter((m) => pending[`${g.id}:${m.user}`] !== null).sort((x, y) => (y.user === me) - (x.user === me) || userName(state, x.user).localeCompare(userName(state, y.user)))
  const waiting = [...g.invited, ...sent.filter((s) => !g.invited.some((m) => m.user === s.user))]
  const stuck = lastAdmin && g.members.length > 1
  const note = admin ? (admins.length === 1 ? "Make someone else an admin before you step down." : "") : "Only its admins change who's in it."
  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center gap-4 pr-8">
        <AppIcon app={GROUPS} size={56} />
        <h2 className="min-w-0 text-xl font-semibold break-words select-text">{g.name}</h2>
      </div>

      {g.handle && (
        <List>
          <ListRow title="Handle" trailing={<span className="text-sm text-muted-foreground">@{g.handle}</span>} onClick={admin ? handle : undefined} />
        </List>
      )}

      <Section title="Members">
        <List>
          {members.map((m) => {
            const role = pending[`${g.id}:${m.user}`] || m.role
            const name = userName(state, m.user)
            const trailing = admin && !(role === "admin" && admins.length === 1)
              ? <Role name={name} role={role} remove={m.user === me ? `Leave ${g.name}` : `Remove from ${g.name}`} change={(r) => r !== role && change(m.user, r)} />
              : <span className="text-sm text-muted-foreground">{roleName(role)}</span>
            return <ListRow key={m.user} leading={<Avatar id={m.user} name={name} size={36} />} title={`${name}${m.user === me ? " (you)" : ""}`} trailing={trailing} />
          })}
        </List>
        {error ? <Problem title={error} /> : note && <p className="text-sm text-muted-foreground">{note}</p>}
      </Section>

      {!!waiting.length && (
        <Section title="Invited">
          <List>
            {waiting.map((m) => <ListRow key={m.user} leading={<Avatar id={m.user} name={userName(state, m.user)} size={36} />} title={userName(state, m.user)} trailing={<span className="text-sm text-muted-foreground">Waiting</span>} />)}
          </List>
        </Section>
      )}

      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          {admin && <Button variant="secondary" onClick={invite}><Icon name="plus" />Invite Someone</Button>}
          <Button variant="destructive" disabled={stuck} onClick={() => change(me, null)}>Leave {g.name}</Button>
        </div>
        {stuck && <p className="text-sm text-muted-foreground">Make someone else an admin before you leave.</p>}
      </div>
    </div>
  )
}

// Someone's role in the group, a menu: the roles, and removing them (leaving, for themselves) after them.
function Role({ name, role, remove, change }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" aria-label={`${name}'s role`}>{roleName(role)}<Icon name="caret-up-down" /></Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup value={role} onValueChange={change}>
          <DropdownMenuRadioItem value="admin">Admin</DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="member">Member</DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={() => change(null)}>{remove}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

// A new group, with them its first admin; then its page.
function NewGroup({ open, setOpen, made }) {
  const [name, setName] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  async function save() {
    if (!name.trim() || busy) return
    setBusy(true)
    setError("")
    try {
      const group = await api("POST", "/v1/group", { name: name.trim() })
      setOpen(false)
      one.tap("success")
      made(group.group)
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="New Group" action={<Button size="sm" disabled={!name.trim() || busy} onClick={save}>Create</Button>}>
      <form className="flex flex-col gap-4 p-4" onSubmit={(e) => (e.preventDefault(), save())}>
        <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Name" autoComplete="off" enterKeyHint="done" />
        <p className="text-sm text-muted-foreground">You'll be its first admin.</p>
        {error && <Problem title={error} />}
      </form>
    </ResponsiveDialog>
  )
}

// Inviting someone already in the network: find them by name, tap one (or Return for the first) to invite.
function Invite({ g, state, sent, open, setOpen, send }) {
  const [role, setRole] = useState("member")
  const [query, setQuery] = useState("")
  const taken = (id) => g.members.some((m) => m.user === id) || g.invited.some((m) => m.user === id) || sent.some((s) => s.group === g.id && s.user === id)
  const others = [...state.network.users].sort((x, y) => x.name.localeCompare(y.name)).filter((u) => !taken(u.id))
  const matches = others.filter((u) => u.name.toLowerCase().includes(query.trim().toLowerCase()))
  const pick = (user) => (setOpen(false), send(user, role))
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title={`Invite to ${g.name}`} action={<Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Cancel</Button>}>
      <div className="flex flex-col gap-4 overflow-y-auto p-4">
        <Tabs value={role} onValueChange={setRole}>
          <TabsList className="w-full">
            <TabsTrigger value="member">Member</TabsTrigger>
            <TabsTrigger value="admin">Admin</TabsTrigger>
          </TabsList>
        </Tabs>
        <label className="relative block">
          <Icon name="magnifying-glass" className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted-foreground" />
          <Input
            autoFocus
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && matches[0] && pick(matches[0].id)}
            placeholder="Find people"
            aria-label="Find people"
            enterKeyHint="search"
            autoComplete="off"
            {...quiet}
            className="pl-9 [&::-webkit-search-cancel-button]:hidden"
          />
        </label>
        {matches.length ? (
          <List className="max-w-none">
            {matches.map((u) => (
              <ListRow
                key={u.id}
                leading={<Avatar id={u.id} name={u.name} size={36} />}
                title={u.name}
                sub={u.role === "admin" ? `Admin of ${one.config.yours}` : undefined}
                trailing={<Icon name="plus-circle" className="size-5 text-muted-foreground" />}
                className="[&>svg:last-child]:hidden"
                onClick={() => pick(u.id)}
              />
            ))}
          </List>
        ) : others.length ? (
          <p className="text-center text-sm text-muted-foreground">No one called “{query.trim()}”.</p>
        ) : null}
        <p className="text-sm text-muted-foreground">{others.length ? "They join once they accept." : "Everyone is in this group already."}</p>
      </div>
    </ResponsiveDialog>
  )
}

// A new handle for a group: lowercase as it's typed, saved once it's one.
function Handle({ g, open, setOpen, saved }) {
  const [value, setValue] = useState(g.handle)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const ok = /^[a-z0-9]+(-[a-z0-9]+)*$/.test(value) && value.length <= 39 && value !== g.handle
  async function save() {
    if (!ok || busy) return
    setBusy(true)
    setError("")
    try {
      await api("POST", "/v1/handle", { handle: value, group: g.id })
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
        <p className="text-sm text-muted-foreground">What apps call {g.name} by, like Repos' addresses, which move with it. Lowercase letters, digits and hyphens.</p>
        {error && <Problem title={error} />}
      </form>
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<Groups />)
