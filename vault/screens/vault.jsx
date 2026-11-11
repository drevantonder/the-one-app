// The Vault: the items on each page this user keeps (theirs, their groups' as an admin, everyone's as an admin),
// searchable, each with what it's for and the apps that use it. The node never gives a secret field's value, so this
// screen never has one: replacing one sends the new value once.
//
// A Settings screen written in React: @one/ui's build (ui/kit, `bun run build`) makes it into dist/vault.html,
// which the client builds in (ui/src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import {
  AppIcon, Button, Empty, Grid, GridRow, Icon, Input, Label, List, ListAdd, ListRow, ListSkeleton, LoadFailure, LoadingButton, Problem, ResponsiveDialog, SearchInput, Split, Switch,
  Tabs, TabsList, TabsTrigger, LargeTitle, Section,
} from "@one/ui"
import { appNamed } from "../../ui/settings/agents.js"

const api = one.api
const call = async (app, tool, input) => (await api("POST", "/api/call", { app, tool, input })).ok
const listItems = (s) => api("GET", `/api/vault?scope=${encodeURIComponent(s)}`)
const setItem = (s, id, change) => api("POST", "/api/vault", { ...change, scope: s, ...(id ? { id } : {}) })
const has = (a, tool) => (Array.isArray(a?.tools) ? a.tools.some((o) => (o.name ?? o) === tool) : !!a?.tools?.[tool])
const shown = (item) => item.fields.filter((f) => !f.name.startsWith("_"))
// Its one short line: what it is, else its first plain field; none when what it is is its title.
const line = (item) => (item.type ? (item.type === item.title ? "" : item.type) : shown(item).find((f) => !f.secret && f.value)?.value || "")
// A field's name as a user reads it: `client_id` is "Client ID".
const fieldLabel = (name) => {
  const words = name.split(/[_\-.]/).filter(Boolean).map((w) => (["id", "url", "api", "pin", "otp"].includes(w.toLowerCase()) ? w.toUpperCase() : w.toLowerCase()))
  return words.length ? [words[0][0].toUpperCase() + words[0].slice(1), ...words.slice(1)].join(" ") : name
}
const sites = (text) => text.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean)
const said = (err) => one.plain(err).title
const KEY = { name: "Item", icon: "key", color: "#8e8e93" }

// The pages this user keeps items on: theirs, the groups they're an admin of, and everyone's as an admin.
function pagesFrom(state) {
  const admin = (scope) => scope === "user" || (scope === "global" ? state.user?.role === "admin" : state.network?.groups.find((g) => `group:${g.id}` === scope)?.role === "admin")
  return (state.pages || []).filter((p) => admin(p.scope))
}

// What the screen knows of the apps: each by name, and what they say about items.
function know(apps) {
  const app = (name) => appNamed(apps, name)
  const label = (name) => app(name)?.title || name
  // "Gmail and Drive".
  const names = (list) => new Intl.ListFormat(undefined, { type: "conjunction" }).format(list.map(label))
  return {
    app,
    label,
    names,
    // An item's icon: the logo of the site it's for, else the icon of the app that made or uses it, else a key.
    icon: (item) => (item.image ? { name: item.title, image: item.image, image_kind: item.image_kind } : app(item.app || item.apps[0]) || { ...KEY, name: item.title }),
    // Whether it's a plain key: no site's logo and no app's icon to tell it from the others.
    plain: (item) => !item.image && !app(item.app || item.apps[0]),
    // The items the apps added for this user make, as their `vault` says.
    makers: () => apps.filter((a) => a.added?.length).flatMap((a) => Object.values(a.vault || {}).filter((n) => n.make && n.type).map((n) => ({ app: a.name, tool: n.make, type: n.type }))).sort((x, y) => x.type.localeCompare(y.type)),
    // What the apps switched on for an item do with items of its type.
    actions: (item) => apps.filter((a) => item.apps.includes(a.name)).flatMap((a) => Object.values(a.vault || {}).filter((n) => n.type && n.type === item.type).flatMap((n) => Object.entries(n.actions || {}).map(([tool, title]) => ({ app: a.name, tool, title })))).sort((x, y) => x.title.localeCompare(y.title)),
    // Its row's grey line: what it is, and the apps using it but the one that made it and those what it is names;
    // with no type, the apps using it, else its first plain field.
    rowLine: (item) => {
      const others = item.apps.filter((a) => a !== item.app && !(item.type || "").toLowerCase().includes(label(a).toLowerCase()))
      const used = others.length ? `Used by ${names(others)}` : ""
      return item.type ? [line(item), used].filter(Boolean).join(" · ") : used || line(item)
    },
    matches: (item, query) => {
      if (!query) return true
      const words = [item.title, item.type, item.domain, ...item.hosts, ...item.apps.map(label), ...item.fields.filter((f) => !f.secret).map((f) => f.value)]
      return words.some((w) => String(w ?? "").toLowerCase().includes(query))
    },
    // Deletes an item, signing an account out first: each app using it, or whose sign-in made it, that lists it among
    // its accounts (by its title or a plain field) signs that account out, so what the app keeps elsewhere goes too and
    // listing accounts doesn't make the item again.
    remove: async (item) => {
      if (item.account) {
        const known = new Set([item.title, ...item.fields.filter((f) => !f.secret).map((f) => f.value)])
        // Every app at once.
        await Promise.all([...new Set([...item.apps, item.app].filter(Boolean))].map(async (name) => {
          const a = app(name)
          const accounts = Array.isArray(a?.tools) && a.tools.find((o) => o.accounts)?.name
          if (!accounts || !has(a, "sign_out")) return
          const account = (await call(name, accounts, {})).find((x) => known.has(x.account))
          if (account) await call(name, "sign_out", { account: account.account })
        }))
        // Signing out may have let go of it already.
        if (!(await listItems(item.scope)).some((i) => i.id === item.id)) return
      }
      await setItem(item.scope, item.id, { deleted: true })
    },
  }
}

// A link: through the bridge's link handling where there is one (a frame), else a new tab or the browser.
function openLink(url) {
  const a = Object.assign(document.createElement("a"), { href: url, target: "_blank", rel: "noopener" })
  document.body.append(a)
  a.click()
  a.remove()
}

// An app's action on an item opens the app's screen it gives, or else its link.
const go = (to) => (to.screen ? one.open(to.screen.screen || "main", to.screen.params || {}, to.app) : to.url && openLink(to.url))

function Vault() {
  // Opened for an item (`{ scope, id }`, from another screen), on its page with it open.
  const [scope, setScope] = useState(one.params?.scope || "user")
  const [opening, setOpening] = useState(one.params?.id)
  const [pages, setPages] = useState([])
  const [apps, setApps] = useState([])
  const [items, setItems] = useState(null)
  const [problem, setProblem] = useState(null)
  // Another page's items are coming: what's there dims until they do.
  const [switching, setSwitching] = useState(false)
  const [query, setQuery] = useState("")
  const [openId, setOpenId] = useState(null)
  // What an app's action left open, by item.
  const [following, setFollowing] = useState({})
  const [making, setMaking] = useState(null)
  const [dialogOpen, setDialogOpen] = useState(false)
  const now = useRef(scope)
  now.current = scope
  const k = know(apps)

  async function load(s = now.current) {
    try {
      const [state, all, list] = await Promise.all([api("GET", "/api/state"), api("GET", "/api/apps"), listItems(s)])
      const p = pagesFrom(state)
      if (!p.some((x) => x.scope === s)) return void (setScope((now.current = "user")), load("user"))
      if (s !== now.current) return
      setPages(p)
      setApps(all)
      setItems(list)
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  // Another page's items: chosen at once, its list dimmed until only its items come.
  async function show(s) {
    if (s === scope) return
    setScope((now.current = s))
    setSwitching(true)
    try {
      const list = await listItems(s)
      if (s === now.current) setItems(list)
    } catch (e) {
      if (s === now.current) setProblem(one.plain(e))
    } finally {
      if (s === now.current) setSwitching(false)
    }
  }
  // The items as they are now, after a change here.
  async function refresh() {
    const list = await listItems(now.current)
    setItems(list)
  }

  useEffect(() => {
    load()
    // The node's own entries (an item changed) change no app.
    one.on("changed", (e) => e.type === "changed" && !(e.apps || []).length && load())
  }, [])
  useEffect(() => {
    if (items && opening) {
      if (items.some((i) => i.id === opening)) setOpenId(opening)
      setOpening(null)
    }
  }, [items, opening])

  // New: an item a user types, or with the apps that make items of a type, one of those.
  function startNew() {
    const made = k.makers()
    setMaking(made.length ? { made } : { item: true })
    setDialogOpen(true)
  }

  // Gone elsewhere (deleted on another device): its page goes too.
  const selected = items?.find((i) => i.id === openId)
  const found = (items?.filter((i) => k.matches(i, query.trim().toLowerCase())) || []).sort((a, b) => a.title.localeCompare(b.title))
  let body
  if (!items) {
    body = problem ? <LoadFailure problem={problem} retry={() => load()} />
      : <ListSkeleton rows={4} />
  } else if (found.length) {
    body = (
      <Grid min="18rem" max={5} gap="0.25rem" className={switching ? "opacity-50 transition-opacity" : ""}>
        {found.map((item) => (
          <GridRow
            key={item.id}
            leading={k.plain(item) ? <Icon name="key" className="size-5 text-muted-foreground" /> : <AppIcon app={k.icon(item)} size={28} />}
            title={item.title}
            sub={k.rowLine(item) || undefined}
            selected={item.id === openId}
            onClick={() => setOpenId(item.id)}
          />
        ))}
      </Grid>
    )
  } else {
    body = <Empty className={switching ? "opacity-50" : ""}>{query.trim() ? `Nothing matches “${query.trim()}”.` : "Keys and sign-ins your apps and agents use without seeing them."}</Empty>
  }

  return (
    <>
      <Split
        title={selected?.title}
        onClose={() => setOpenId(null)}
        list={
          <div className="flex flex-col gap-4">
            <LargeTitle action={<Button onClick={startNew}>New Item</Button>}>Vault</LargeTitle>
            {pages.length > 1 && <Pages pages={pages} value={scope} onChange={show} />}
            {!!items?.length && <SearchInput className="max-w-md" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search" enterKeyHint="search" autoComplete="off" autoCapitalize="off" spellCheck={false} />}
            {body}
          </div>
        }
        detail={selected && (
          <Item
            key={selected.id}
            item={selected}
            k={k}
            following={following[selected.id]}
            follow={(f) => setFollowing((all) => ({ ...all, [selected.id]: f }))}
            refresh={refresh}
            deleted={() => (setOpenId(null), load())}
          />
        )}
      />
      {making?.made && <Choose k={k} made={making.made} open={dialogOpen} setOpen={setDialogOpen} pick={(maker) => setMaking(maker ? { maker } : { item: true })} />}
      {making?.maker && <MakeItem maker={making.maker} open={dialogOpen} setOpen={setDialogOpen} done={() => (setDialogOpen(false), load())} />}
      {making?.item && <NewItem pages={pages} scope={scope} open={dialogOpen} setOpen={setDialogOpen} done={(page) => (setDialogOpen(false), page !== now.current && setScope((now.current = page)), load(page))} />}
    </>
  )
}

function Pages({ pages, value, onChange }) {
  return (
    <Tabs value={value} onValueChange={onChange}>
      <TabsList className="max-w-full justify-start overflow-x-auto">
        {pages.map((p) => <TabsTrigger key={p.scope} value={p.scope}>{p.title}</TabsTrigger>)}
      </TabsList>
    </Tabs>
  )
}

// A heading over a part of an item.
const quiet = { autoCapitalize: "none", autoCorrect: "off", spellCheck: false }
// An input that's a row of a List: the list's lines are its edges.
const inRow = "h-11 rounded-none border-x-0 border-t-0 bg-transparent px-3 shadow-none focus-visible:ring-0 dark:bg-transparent"

// An item's page: its fields (a plain one to copy, a secret one only to replace), where they're sent, the apps that
// want it, each with a switch, and deleting it (signing out, for an account a sign-in made).
function Item({ item, k, following: f, follow, refresh, deleted }) {
  const [error, setError] = useState("")
  const [renaming, setRenaming] = useState(false)
  const [busy, setBusy] = useState(null)
  const [adding, setAdding] = useState(false)
  const acts = k.actions(item)
  // The apps that want it or use it, those using it first.
  const wanting = [...new Set([...item.apps, ...(item.wanted_by || [])])]
  const hostsShown = !item.hosts.length || !item.hosts.every(k.app)

  // Changes the item, then shows it as it is now. Whether it worked.
  async function save(change) {
    setError("")
    try {
      await setItem(item.scope, item.id, change)
      await refresh()
      return true
    } catch (e) {
      setError(said(e))
      return false
    }
  }

  // An app's action on the item: one that leaves something open says how to finish it.
  async function act(a, n) {
    if (busy !== null) return
    setBusy(n)
    setError("")
    try {
      const reply = (await call(a.app, a.tool, { item: item.id })) || {}
      const to = { app: a.app, screen: reply.screen, url: reply.url }
      if (reply.then?.tool) follow({ ...to, tool: reply.then.tool, title: reply.then.title || "Done" })
      await refresh()
      go(to)
    } catch (e) {
      setError(said(e))
    } finally {
      setBusy(null)
    }
  }
  async function finish() {
    if (busy !== null) return
    setBusy("finish")
    setError("")
    try {
      await call(f.app, f.tool, { item: item.id })
      follow(undefined)
      await refresh()
    } catch (e) {
      setError(said(e))
    } finally {
      setBusy(null)
    }
  }

  async function remove() {
    const ok = await one.alert({
      title: item.account ? `Sign out of ${item.title}?` : `Delete ${item.title}?`,
      text: item.apps.length ? `${k.names(item.apps)} ${item.apps.length === 1 ? "stops" : "stop"} using it.` : "It can’t be undone.",
      action: item.account ? "Sign Out" : "Delete",
      danger: true,
    })
    if (!ok) return
    one.tap("medium")
    setBusy("delete")
    setError("")
    try {
      await k.remove(item)
      deleted()
    } catch (e) {
      setError(said(e))
      setBusy(null)
    }
  }

  function rename(title) {
    setRenaming(false)
    title = title.trim()
    if (title && title !== item.title) save({ title })
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center gap-4 pr-8">
        <AppIcon app={k.icon(item)} size={56} />
        <div className="min-w-0 flex-1">
          {renaming ? (
            <Input
              autoFocus
              defaultValue={item.title}
              aria-label="Title"
              className="text-lg font-semibold"
              onFocus={(e) => e.target.select()}
              onBlur={(e) => rename(e.target.value)}
              onKeyDown={(e) => (e.key === "Enter" ? e.currentTarget.blur() : e.key === "Escape" && (e.currentTarget.value = item.title, e.currentTarget.blur()))}
            />
          ) : (
            <h2 className="text-xl font-semibold break-words select-text">{item.title}</h2>
          )}
          {!renaming && <Button variant="ghost" size="xs" className="-mx-2 text-muted-foreground" onClick={() => setRenaming(true)}>Rename</Button>}
        </div>
      </div>

      {!!acts.length && (
        <div className="flex flex-wrap gap-2">
          {f ? (
            <>
              {(f.screen || f.url) && <Button variant="outline" onClick={() => go(f)}>Open Again</Button>}
              <Button variant="secondary" disabled={busy !== null} onClick={finish}>{busy === "finish" ? `${f.title}…` : f.title}</Button>
            </>
          ) : acts.map((a, n) => (
            <Button key={`${a.app}.${a.tool}`} variant="secondary" disabled={busy !== null} onClick={() => act(a, n)}>{busy === n ? `${a.title}…` : a.title}</Button>
          ))}
        </div>
      )}

      <List>
        {shown(item).map((field) => <Field key={field.name} field={field} save={save} />)}
        {adding ? <NewField save={save} done={() => setAdding(false)} /> : <ListAdd onClick={() => setAdding(true)}>Add Field</ListAdd>}
      </List>

      {hostsShown && (
        <Section title="Sent only to">
          <Hosts key={item.hosts.join()} hosts={item.hosts} save={save} />
        </Section>
      )}

      {!!wanting.length && (
        <Section title="Used by">
          <List>
            {wanting.map((a) => <Using key={a} app={k.app(a) || { name: a }} title={k.label(a)} on={item.apps.includes(a)} set={(on) => save({ apps: [...item.apps.filter((x) => x !== a), ...(on ? [a] : [])] })} />)}
          </List>
        </Section>
      )}

      {error && <Problem title={error} />}

      <Button variant="destructive" className="self-start" disabled={busy === "delete"} onClick={remove}>
        {item.account ? "Sign Out" : "Delete Item"}
      </Button>
    </div>
  )
}

// A field: a plain one is changed by tapping it, a secret one only replaced, typed unseen. Editing, it's a row of its
// own, the whole width to type in; taking it out is asked first, as a secret can't be got back.
function Field({ field, save }) {
  const [editing, setEditing] = useState(false)
  const [copied, setCopied] = useState(false)
  const label = fieldLabel(field.name)
  if (editing) {
    const put = async (input) => {
      if (!input.value) return
      const value = input.value
      input.value = ""
      if (await save({ fields: { [field.name]: { value } } })) setEditing(false)
      else if (!field.secret) input.value = value
    }
    const remove = async () => {
      const ok = await one.alert({ title: `Remove ${label}?`, text: field.secret ? "It can’t be undone." : undefined, action: "Remove", danger: true })
      if (ok) save({ fields: { [field.name]: null } })
    }
    return (
      <form className="flex flex-col gap-2 p-3" onSubmit={(e) => (e.preventDefault(), put(e.currentTarget.elements.value))}>
        <span className="text-sm font-medium">{label}</span>
        <Input
          name="value"
          autoFocus
          type={field.secret ? "password" : "text"}
          autoComplete="off"
          {...quiet}
          aria-label={label}
          placeholder={field.secret ? "New value" : undefined}
          defaultValue={field.secret ? undefined : field.value || ""}
          onKeyDown={(e) => e.key === "Escape" && setEditing(false)}
        />
        <div className="flex items-center gap-2">
          <Button type="submit" size="sm">Save</Button>
          <Button type="button" size="sm" variant="ghost" onClick={() => setEditing(false)}>Cancel</Button>
          <Button type="button" size="sm" variant="ghost" className="ml-auto text-destructive" onClick={remove}>Remove</Button>
        </div>
      </form>
    )
  }
  const copy = async (e) => {
    e.stopPropagation()
    await one.copy(field.value || "")
    one.tap("success")
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }
  return (
    <div
      className={`flex min-h-12 items-center gap-3 px-3 py-1.5 text-sm ${field.secret ? "" : "cursor-pointer hover:bg-accent/50"}`}
      onClick={field.secret ? undefined : () => setEditing(true)}
    >
      <span className="shrink-0 font-medium">{label}</span>
      <span className="min-w-0 flex-1 truncate text-right text-muted-foreground select-text">{field.secret ? "••••••••" : field.value}</span>
      {field.secret ? (
        <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>Replace</Button>
      ) : (
        <Button variant="ghost" size="icon-sm" aria-label={copied ? "Copied" : `Copy ${label}`} onClick={copy}>
          <Icon name={copied ? "check" : "copy"} />
        </Button>
      )}
    </div>
  )
}

// A field added to an item: its name, its value, and whether it's secret (at first it is).
function NewField({ save, done }) {
  const [secret, setSecret] = useState(true)
  const name = useRef(null)
  const value = useRef(null)
  const put = async () => name.current.value.trim() && value.current.value && (await save({ fields: { [name.current.value.trim()]: { value: value.current.value, secret } } })) && done()
  return (
    <div className="flex flex-col gap-2 p-3">
      <Input ref={name} autoFocus placeholder="Field name" aria-label="Field name" {...quiet} />
      <Input ref={value} type={secret ? "password" : "text"} placeholder="Value" aria-label="Value" autoComplete="off" onKeyDown={(e) => e.key === "Enter" && put()} />
      <Label className="justify-between py-1 font-normal">Secret<Switch checked={secret} onCheckedChange={setSecret} /></Label>
      <div className="flex items-center gap-2">
        <LoadingButton size="sm" run={put}>Save</LoadingButton>
        <Button size="sm" variant="ghost" onClick={done}>Cancel</Button>
      </div>
    </div>
  )
}

// The sites an item's fields are sent to, changed once typing's done.
function Hosts({ hosts, save }) {
  return (
    <Input
      className="max-w-2xl"
      defaultValue={hosts.join(", ")}
      placeholder="api.example.com"
      aria-label="Sent only to"
      inputMode="url"
      {...quiet}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      onBlur={async (e) => {
        const input = e.currentTarget
        const list = sites(input.value)
        if (list.join() !== hosts.join() && !(await save({ hosts: list }))) input.value = hosts.join(", ")
      }}
    />
  )
}

// An app that wants an item or uses it: a switch shows its new place at once; one that couldn't change goes back.
function Using({ app, title, on, set }) {
  const [want, setWant] = useState(null)
  return (
    <label className="flex cursor-pointer items-center gap-3 px-3 py-2 text-sm">
      <AppIcon app={app} size={28} />
      <span className="min-w-0 flex-1 truncate font-medium">{title}</span>
      <Switch
        checked={want ?? on}
        aria-busy={want !== null || undefined}
        onCheckedChange={async (now) => {
          setWant(now)
          await set(now)
          setWant(null)
        }}
      />
    </label>
  )
}

// The New dialog's choice: an item a user types, or one an app makes.
function Choose({ k, made, open, setOpen, pick }) {
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="New Item" action={<Button variant="ghost" size="sm" onClick={() => setOpen(false)}>Cancel</Button>}>
      <div className="overflow-y-auto p-4">
        <List>
          <ListRow leading={<AppIcon app={KEY} size={30} />} title="Key or Password" onClick={() => pick(null)} />
          {made.map((m) => <ListRow key={`${m.app}.${m.type}`} leading={<AppIcon app={k.app(m.app) || { name: m.app }} size={30} />} title={m.type} onClick={() => pick(m)} />)}
        </List>
      </div>
    </ResponsiveDialog>
  )
}

// A new item a user types: its page, title, fields (each secret or not) and the sites they're sent to, which a secret
// must have.
function NewItem({ pages, scope, open, setOpen, done }) {
  const [page, setPage] = useState(scope)
  const [drafts, setDrafts] = useState([{ n: 0, secret: true }])
  const [error, setError] = useState("")
  const [saving, setSaving] = useState(false)
  const form = useRef(null)
  async function save() {
    const el = form.current
    const title = el.elements.title.value.trim()
    const hosts = sites(el.elements.hosts.value)
    const fields = {}
    for (const d of drafts) {
      const name = el.elements[`name${d.n}`].value.trim(), value = el.elements[`value${d.n}`].value
      if (name && value) fields[name] = { value, secret: d.secret }
    }
    if (!title) return setError("Give it a title.")
    if (!Object.keys(fields).length) return setError("Give a field a name and a value.")
    if (!hosts.length && Object.values(fields).some((f) => f.secret)) return setError("Say which site a secret is sent to.")
    setSaving(true)
    setError("")
    try {
      await setItem(page, null, { title, fields, hosts, apps: [] })
      el.reset()
      done(page)
    } catch (e) {
      setSaving(false)
      setError(said(e))
    }
  }
  const draft = (d, set) => (
    <List key={d.n}>
      <Input name={`name${d.n}`} autoFocus={d.n > 0} placeholder="Field name" aria-label="Field name" {...quiet} className={inRow} />
      <Input name={`value${d.n}`} type={d.secret ? "password" : "text"} placeholder="Value" aria-label="Value" autoComplete="off" className={inRow} />
      <Label className="justify-between px-3 py-3 font-normal">Secret<Switch checked={d.secret} onCheckedChange={set} /></Label>
    </List>
  )
  return (
    <ResponsiveDialog
      open={open}
      onOpenChange={setOpen}
      title="New Item"
      action={<Button size="sm" disabled={saving} onClick={save}>Save</Button>}
    >
      <form ref={form} className="flex flex-col gap-4 overflow-y-auto p-4" onSubmit={(e) => (e.preventDefault(), save())}>
        <Input name="title" autoFocus placeholder="Title" aria-label="Title" />
        {pages.length > 1 && <Pages pages={pages} value={page} onChange={setPage} />}
        {drafts.map((d) => draft(d, (secret) => setDrafts((all) => all.map((x) => (x.n === d.n ? { ...x, secret } : x)))))}
        <Button type="button" variant="ghost" className="self-start" onClick={() => setDrafts((all) => [...all, { n: all.length, secret: false }])}>
          <Icon name="plus" />Add Field
        </Button>
        <Section title="Sent only to">
          <Input name="hosts" placeholder="api.example.com" aria-label="Sent only to" inputMode="url" {...quiet} />
        </Section>
        {error && <Problem title={error} />}
        <button type="submit" hidden />
      </form>
    </ResponsiveDialog>
  )
}

// A new item an app makes, of a type it uses: the user names it.
function MakeItem({ maker, open, setOpen, done }) {
  const [error, setError] = useState("")
  const [adding, setAdding] = useState(false)
  const input = useRef(null)
  async function make() {
    const title = input.current.value.trim()
    if (!title || adding) return
    setAdding(true)
    setError("")
    try {
      await call(maker.app, maker.tool, { title })
      done()
    } catch (e) {
      setAdding(false)
      setError(said(e))
    }
  }
  return (
    <ResponsiveDialog
      open={open}
      onOpenChange={setOpen}
      title={`New ${maker.type}`}
      action={<Button size="sm" disabled={adding} onClick={make}>Add</Button>}
    >
      <div className="flex flex-col gap-4 p-4">
        <Input ref={input} autoFocus placeholder="Name" aria-label="Name" onKeyDown={(e) => e.key === "Enter" && make()} />
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<Vault />)
