import * as React from "react"
import { WarningCircleIcon } from "@phosphor-icons/react"
import { Button } from "@/components/ui/button"
import { List, ListRow } from "@/components/layout"
import { failureSentence } from "@/components/loading"

/// What an app that keeps accounts answers to its `accounts` call, one each: its name, whether it works and, if it
/// doesn't, why. An app may add what it likes (a device, say) for its own rows.
type Account = { account: string; ok?: boolean; why?: string; [more: string]: unknown }

type Bridge = {
  call: (tool: string, input?: object) => Promise<any>
  on: (event: string, run: () => void) => void
  plain: (err: unknown) => { title: string; next?: string }
}
const bridge = () => (globalThis as unknown as { one: Bridge }).one

/// The accounts an app has signed in (its `accounts` call), kept fresh: asked for now and again whenever the app
/// changes; a later failure keeps what's shown. `signOut(account, input?)` (the app's `sign_out` call) takes one off at
/// once, and puts it back, saying why in `failed`, only if that fails. `accounts` is null until the first answer;
/// `problem` is why that failed, with `retry`.
function useAccounts() {
  const [accounts, setAccounts] = React.useState<Account[] | null>(null)
  const [problem, setProblem] = React.useState<{ title: string; next?: string } | null>(null)
  // Signed out here and maybe not yet on the device, so a redraw meanwhile doesn't bring them back.
  const [leaving, setLeaving] = React.useState<string[]>([])
  const [failed, setFailed] = React.useState<Record<string, string>>({})
  const asked = React.useRef(0)
  const have = React.useRef(false)

  const load = React.useCallback(() => {
    const n = ++asked.current
    return bridge().call("accounts").then(
      (got: Account[]) => {
        if (n !== asked.current) return
        have.current = true
        setAccounts(got)
        setProblem(null)
      },
      (err: unknown) => {
        if (!have.current) setProblem(bridge().plain(err))
      },
    )
  }, [])
  React.useEffect(() => {
    bridge().on("changed", load)
    load()
  }, [load])

  const signOut = async (a: Account, input: object = {}) => {
    const name = a.account
    setLeaving((l) => [...l, name])
    setFailed(({ [name]: _, ...rest }) => rest)
    let ok = true
    try {
      await bridge().call("sign_out", { account: name, ...input })
      setAccounts((all) => all && all.filter((x) => x.account !== name))
    } catch (err) {
      ok = false
      setFailed((f) => ({ ...f, [name]: failureSentence(err) }))
    }
    setLeaving((l) => l.filter((x) => x !== name))
    if (ok) load()
  }

  const shown = accounts && accounts.filter((a) => !leaving.includes(a.account))
  return { accounts: shown, problem, retry: load, failed, signOut, reload: load }
}

/// The accounts as rows under their heading, each with its Sign Out. `line(account, index)` is the grey line under its
/// name: `{ text, warn }` or nothing; what a failed Sign Out said replaces it. `confirm(account)` asks first where
/// signing out loses something, and answers whether to go on. What's inside goes under the rows (an Add Account button).
function AccountList({ title, accounts, failed = {}, line, signOut, confirm, children }: {
  title: string
  accounts: Account[]
  failed?: Record<string, string>
  line?: (account: Account, index: number) => { text: string; warn?: boolean } | null | undefined
  signOut: (account: Account) => unknown
  confirm?: (account: Account) => Promise<boolean> | boolean
  children?: React.ReactNode
}) {
  const out = async (a: Account) => (!confirm || (await confirm(a))) && signOut(a)
  return (
    <section data-slot="account-list" className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      <List>
        {accounts.map((a, i) => {
          const said = failed[a.account] ? { text: failed[a.account], warn: true } : line?.(a, i)
          return (
            <ListRow
              key={a.account}
              title={a.account}
              sub={said && (said.warn ? (
                <span className="inline-flex items-center gap-1 text-warning">
                  <WarningCircleIcon weight="fill" className="size-4" />
                  {said.text}
                </span>
              ) : said.text)}
              trailing={<Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" onClick={() => out(a)}>Sign Out</Button>}
            />
          )
        })}
      </List>
      {children}
    </section>
  )
}

export { AccountList, useAccounts }
export type { Account }
