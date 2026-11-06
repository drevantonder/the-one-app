// Settings: Messenger's accounts are browser profiles (Vault items of type Browser profile): adding one picks a profile,
// or makes one, and opens messenger.com in it to sign in.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AccountList, Empty, Hint, Input, List, ListNote, ListRow, ListSkeleton, LoadFailure, LoadingButton, Problem, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, useAccounts } from "@one/ui"

const BLURB = "Link Messenger to a browser profile signed in to messenger.com, and your agents can read your chats and, when you ask, send."
// The choice that makes a profile of its own, not one already there.
const NEW = "\u0000"

// Asked for as the screen opens, so Add Account has them at once.
const profiles = () => one.call("_profiles").then((r) => (r || []).map((p) => p.profile), () => [])

/// Picks a profile not linked yet, or names a new one, then opens messenger.com in it.
function Pick({ free, onSigningIn }) {
  const [choice, setChoice] = useState(free[0] ?? NEW)
  const [name, setName] = useState("")
  const [why, setWhy] = useState(null)
  const field = useRef(null)
  const go = async () => {
    const profile = choice === NEW ? name.trim() : choice
    if (!profile) return field.current?.focus()
    setWhy(null)
    try {
      onSigningIn(await one.call("sign_in", { profile }))
    } catch (err) {
      setWhy(one.plain(err).title)
    }
  }
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">Browser profile</h2>
      <List>
        <ListRow
          title="Profile"
          trailing={
            <Select value={choice} onValueChange={setChoice}>
              <SelectTrigger size="sm" aria-label="Browser profile"><SelectValue /></SelectTrigger>
              <SelectContent align="end" position="popper">
                {free.map((p) => <SelectItem key={p} value={p}>{p}</SelectItem>)}
                <SelectItem value={NEW}>New profile…</SelectItem>
              </SelectContent>
            </Select>
          }
        />
        {choice === NEW && (
          <div className="px-3 py-2.5">
            <Input ref={field} placeholder="Its name, like Personal or Work" aria-label="Its name" autoCapitalize="words" autoCorrect="off" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
        )}
      </List>
      <Hint>Each has its own sign-ins, like Personal or Work.</Hint>
      {why && <Problem title={why} action={{ label: "Try Again", run: go }} />}
      <LoadingButton className="self-start" run={go}>Sign In to Messenger</LoadingButton>
    </section>
  )
}

/// Sign in in the device's browser, then press Done to keep the profile.
function SigningIn({ link, onDone }) {
  const [why, setWhy] = useState(null)
  const done = async () => {
    setWhy(null)
    try {
      await one.call("_save", { profile: link.profile })
    } catch (err) {
      return setWhy(one.plain(err).title)
    }
    onDone()
  }
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">Sign In</h2>
      <List>
        <ListRow title="Open the device" onClick={() => one.link(link.url)} />
        <ListNote>{`Its browser is on messenger.com, in the ${link.profile} profile.`}</ListNote>
        <ListNote>Sign in there, and tick Keep me signed in.</ListNote>
      </List>
      <Hint>Come back and press Done.</Hint>
      {why && <Problem title={why} action={{ label: "Try Again", run: done }} />}
      <LoadingButton className="self-start" run={done}>Done</LoadingButton>
    </section>
  )
}

function Settings() {
  const { accounts, problem, retry, failed, signOut, reload } = useAccounts()
  // What's under way: choosing a profile (`picking`), or signing in to one (`link`).
  const [step, setStep] = useState(null)
  const known = useRef(null)
  known.current ||= profiles()

  const pick = async () => {
    const all = await known.current
    known.current = profiles()
    setStep({ picking: [...new Set(all)] })
  }
  const stop = () => (setStep(null), reload())

  useEffect(() => {
    if (one.params.add) known.current.then(() => pick())
  }, [])

  if (problem && !accounts) return <LoadFailure problem={problem} retry={retry} />
  if (!accounts) return <ListSkeleton />

  const picking = !!step
  const needs = accounts.filter((a) => !a.ok)
  return (
    <div className="flex flex-col gap-6">
      {!accounts.length && !picking && <Empty className="mx-auto max-w-md" action={{ label: "Add Account", run: pick }}>{BLURB}</Empty>}
      {needs.map((a) => <Problem key={a.account} warn title={a.account + " needs signing in"} text="Sign it out, then add it again." />)}
      {accounts.length > 0 && (
        <AccountList
          title="Browser profiles"
          accounts={accounts}
          failed={failed}
          signOut={signOut}
          line={(a, i) => (a.ok ? (i ? null : { text: "Used first" }) : { text: "Sign in again", warn: true })}
        >
          {!picking && <LoadingButton variant="secondary" className="self-start" run={pick}>Add Account</LoadingButton>}
        </AccountList>
      )}
      {step?.picking && (
        <Pick
          free={step.picking.filter((p) => !accounts.some((a) => a.account === p))}
          onSigningIn={(link) => setStep({ link })}
        />
      )}
      {step?.link && <SigningIn link={step.link} onDone={stop} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Settings />)
