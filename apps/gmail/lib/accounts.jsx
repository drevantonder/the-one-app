// What the settings screen of every app that keeps accounts shares (Gmail, Docs, Drive, Sheets, Google Calendar,
// Messenger and WhatsApp): the accounts signed in, each a row with what's wrong with it and Sign Out, and what a screen
// says while it waits or fails. The same file is in each of those apps' lib/, since an app is its folder.
import { useCallback, useEffect, useRef, useState } from "react"
import { Button, Icon, List, ListRow, Problem, Skeleton, Spinner } from "@one/ui"

/// A failure's first sentence, without "Try again." (what the button says).
export const sentence = (err) => one.plain(err).title.replace(/(\. Try again)?\.$/, "")

/// The accounts signed in, as `accounts` gives them ([{ account, ok, why, ... }]), kept fresh: asked for now and again
/// whenever the app changes; a later failure keeps what's shown. Signing one out takes it off at once (`signOut`), and
/// puts it back, saying why on its row, only if that fails. `shown` is what's left.
export function useAccounts() {
  const [accounts, setAccounts] = useState(null)
  const [problem, setProblem] = useState(null)
  // Signed out here and maybe not yet on the device, so a redraw meanwhile doesn't bring them back.
  const [leaving, setLeaving] = useState([])
  const [failed, setFailed] = useState({})
  const asked = useRef(0)
  const have = useRef(false)

  const load = useCallback(() => {
    const n = ++asked.current
    return one.call("accounts").then(
      (got) => {
        if (n !== asked.current) return
        have.current = true
        setAccounts(got)
        setProblem(null)
      },
      (err) => !have.current && setProblem(one.plain(err)),
    )
  }, [])
  useEffect(() => {
    one.on("changed", load)
    load()
  }, [load])

  const signOut = async (a, input = {}) => {
    const name = a.account
    setLeaving((l) => [...l, name])
    setFailed(({ [name]: _, ...rest }) => rest)
    let ok = true
    try {
      await one.call("sign_out", { account: name, ...input })
      setAccounts((all) => all.filter((x) => x.account !== name))
    } catch (err) {
      ok = false
      setFailed((f) => ({ ...f, [name]: sentence(err) }))
    }
    setLeaving((l) => l.filter((x) => x !== name))
    if (ok) load()
  }

  const shown = accounts && accounts.filter((a) => !leaving.includes(a.account))
  return { accounts: shown, problem, retry: load, failed, signOut, reload: load }
}

/// The account rows under their heading, each with its Sign Out. `line(account, index)` is the grey line under its name:
/// `{ text, warn }` or nothing; what a failed Sign Out said replaces it. `confirm(account)` asks first where signing out
/// loses something, and answers whether to go on.
export function AccountList({ title, accounts, failed, line, signOut, confirm, children }) {
  const out = async (a) => (!confirm || (await confirm(a))) && signOut(a)
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      <List>
        {accounts.map((a, i) => {
          const said = failed[a.account] ? { text: failed[a.account], warn: true } : line(a, i)
          return (
            <ListRow
              key={a.account}
              title={a.account}
              sub={said && (said.warn ? <span className="inline-flex items-center gap-1 text-warning"><Icon name="warning-circle-fill" />{said.text}</span> : said.text)}
              trailing={<Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" onClick={() => out(a)}>Sign Out</Button>}
            />
          )
        })}
      </List>
      {children}
    </section>
  )
}

/// What a failed first load says, with Try Again.
export function Trouble({ problem, retry }) {
  return <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: retry }} />
}

/// A button that's working: disabled, with a spinner, until its `run` is done.
export function Working({ run, children, ...props }) {
  const [busy, setBusy] = useState(false)
  return (
    <Button
      {...props}
      disabled={busy || props.disabled}
      onClick={async () => {
        setBusy(true)
        try {
          await run()
        } finally {
          setBusy(false)
        }
      }}
    >
      {busy && <Spinner />}
      {children}
    </Button>
  )
}

/// Grey shapes where rows will be, once it's taken longer than a blink, so a quick load shows nothing in between.
export function Waiting({ rows = 2 }) {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  if (!late) return null
  return (
    <div className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, i) => <Skeleton key={i} className="h-14 w-full max-w-2xl" />)}
    </div>
  )
}

/// One grey line with a spinner, for what's under way.
export function Loading({ children }) {
  return (
    <p className="flex items-center gap-2 text-sm text-muted-foreground">
      <Spinner />
      {children}
    </p>
  )
}

/// A grey line under what it's about.
export function Footer({ children }) {
  return <p className="text-sm text-muted-foreground">{children}</p>
}

/// A line of what to do, as a row of a List: it wraps, which a ListRow's title doesn't.
export function Step({ children }) {
  return <div className="px-3 py-2.5 text-sm">{children}</div>
}
