// Settings: the Google accounts signed in, each with whether it works, a way to allow it again and to sign it
// out, and signing in another at Google, which sends the browser back here with { back: the address it came back to }
// to finish. Opened with { add: true } (a shell's Add Account), it starts signing in at once.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AccountList, Empty, Hint, List, ListRow, ListSkeleton, LoadFailure, Loading, LoadingButton, Problem, failureSentence, useAccounts } from "@one/ui"

const TITLE = "Google Docs"
const BLURB = "Sign in to Google, and your agents can read your documents and, when you ask, write in them."

function Settings() {
  const { accounts, problem, retry, failed, signOut, reload } = useAccounts()
  // What's under way: a link to sign in at (`link`), finishing one (`finishing`), or why one failed (`why`, with its retry).
  const [step, setStep] = useState(null)
  const [signing, setSigning] = useState(false)
  // Not set up for anyone yet, and not this person's to set up: what to do, and no button that can't work.
  const [blocked, setBlocked] = useState(null)

  const stop = () => (setSigning(false), setStep(null))

  async function start(account) {
    let url
    try {
      url = (await one.call("sign_in", account ? { account } : {})).url
    } catch (e) {
      // No Google sign-in yet: an admin sets one up for everyone, then it goes on; anyone else is told whom to ask, and
      // can't sign in until then.
      if (/No Google sign-in/.test(e.message) && one.vault) {
        try {
          if (await one.vault("client")) return start(account)
        } catch (err) {
          e = err
        }
      }
      const ask = /Ask (.+?) to set up (.+?)(?::|$)/.exec(e.message)
      if (ask) return setBlocked({ title: ask[2] + " isn't set up", text: "Ask " + ask[1] + " to set it up. It's done once for everyone." })
      return setStep({ why: failureSentence(e), retry: () => start(account) })
    }
    setSigning(true)
    setStep({ link: url })
  }

  // Back from Google, at the address it sent the browser to.
  async function finish(back) {
    setSigning(true)
    setStep({ finishing: true })
    try {
      const r = await one.call("finish_sign_in", { code: back })
      if (r?.signed_in === false) throw new Error(r.why)
    } catch (e) {
      // Finished already: this screen opened again at the same address.
      if (/no sign-in is waiting/.test(e.message)) return stop(), reload()
      return setStep({ why: failureSentence(e), retry: () => start() })
    }
    stop()
    reload()
  }

  useEffect(() => {
    if (one.params.back) finish(one.params.back)
    else if (one.params.add) start()
  }, [])

  // Signed in where Google sent the browser back (another tab, or the app): done here too.
  const seen = useRef(null)
  const key = accounts && JSON.stringify(accounts)
  useEffect(() => {
    if (seen.current !== null && key !== seen.current && step?.link) stop()
    if (key !== null) seen.current = key
  }, [key])

  if (problem && !accounts) return <LoadFailure problem={problem} retry={retry} />
  if (!accounts) return <ListSkeleton />

  const needs = blocked ? [] : accounts.filter((a) => !a.ok)
  return (
    <div className="flex flex-col gap-6">
      {blocked && <Problem warn title={blocked.title} text={blocked.text} />}
      {!accounts.length && !blocked && !signing && <Empty className="mx-auto max-w-md" action={{ label: "Sign In to Google", run: () => start() }}>{BLURB}</Empty>}
      {/* What needs signing in again is a problem on its account, with the way to do it. */}
      {needs.map((a) => (
        <Problem
          key={a.account}
          warn
          title={a.account + " needs signing in"}
          text={one.plain(a.why || "Needs signing in again").title.replace(/^Needs signing in again\.$/, `Sign in to Google again to let ${TITLE} in.`)}
          action={{ label: "Sign In", run: () => start(a.account) }}
        />
      ))}
      {accounts.length > 0 && (
        <AccountList
          title="Accounts"
          accounts={accounts}
          failed={failed}
          signOut={signOut}
          line={(a, i) => (a.ok ? (i ? null : { text: "Used first" }) : { text: "Sign in again", warn: true })}
        >
          {!blocked && <LoadingButton variant="secondary" className="self-start" run={() => start()}>Add Account</LoadingButton>}
        </AccountList>
      )}
      {step?.link && (
        <section className="flex flex-col gap-2">
          <h2 className="text-sm font-medium text-muted-foreground">Sign In</h2>
          <List><ListRow title="Sign in to Google" sub={`and allow ${TITLE}`} onClick={() => one.link(step.link)} /></List>
          <Hint>Google brings you back here when you're done.</Hint>
        </section>
      )}
      {step?.finishing && <Loading>Signing In</Loading>}
      {step?.why && <Problem title={step.why} action={{ label: "Try Again", run: step.retry }} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Settings />)
