// Settings: each WhatsApp account is a phone linked to one of your devices, as WhatsApp Desktop is linked.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AccountList, Hint, Input, List, ListNote, ListRow, ListSkeleton, LoadFailure, LoadingButton, Problem, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, useAccounts } from "@one/ui"

const NOTE = "Recent chats take a few minutes to arrive."

function Pairing({ phone, device, had, done }) {
  const [codes, setCodes] = useState({})
  const [note, setNote] = useState(NOTE)
  useEffect(() => {
    let over = false
    ;(async () => {
      for (let i = 0; i < 90 && !over; i++) {
        const p = await one.call("_pairing", device ? { device } : {}).catch((err) => ({ error: err.message }))
        if (over) return
        if (p.error) {
          setNote(p.error)
          setCodes({})
          return void setTimeout(done, 3000)
        }
        if (!p.qr) {
          const now = await one.call("accounts").catch(() => [])
          if (now.length > had) return done()
          setNote("The codes ran out before the phone linked.")
          setCodes({})
          return void setTimeout(done, 3000)
        }
        setCodes({ qr: p.qr, code: p.code || "" })
        await new Promise((r) => setTimeout(r, 2000))
      }
      if (!over) done()
    })()
    return () => void (over = true)
  }, [])
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">Link a Device</h2>
      <List>
        <ListNote>On the phone, open WhatsApp › Settings › Linked devices › Link a device.</ListNote>
        <ListNote>{`Scan this code${phone ? ", or tap Link with phone number instead and type the code under it" : ""}.`}</ListNote>
      </List>
      {codes.qr && <img alt="" src={codes.qr} className="mx-auto my-2 block size-[220px] rounded-xl bg-white [image-rendering:pixelated]" />}
      {codes.code && <p className="text-center font-mono text-2xl font-semibold tracking-widest">{codes.code}</p>}
      <Hint>{note}</Hint>
    </section>
  )
}

function Settings() {
  const { accounts, problem, retry, failed, signOut, reload } = useAccounts()
  // The devices that run WhatsApp, or null until known: the accounts and whether any device runs it come together, so
  // the form shows once, or not at all.
  const [running, setRunning] = useState(null)
  const [phone, setPhone] = useState("")
  const [on, setOn] = useState("")
  const [step, setStep] = useState(null)
  // Whether that's not known (the call failed): then the form shows anyway, as when it's known there are some.
  const [unknown, setUnknown] = useState(false)
  const field = useRef(null)

  useEffect(() => {
    one.call("_running").then(
      (r) => setRunning(r.devices),
      () => (setUnknown(true), setRunning([])),
    )
  }, [])
  const device = running && running.length > 1 ? on || running[0] : ""
  const ready = !!accounts && !!running
  useEffect(() => {
    if (ready && one.params.add) field.current?.focus()
  }, [ready])

  async function start() {
    setStep(null)
    try {
      await one.call("sign_in", { ...(phone.trim() ? { phone: phone.trim() } : {}), ...(device ? { device } : {}) })
    } catch (err) {
      return setStep({ why: one.plain(err).title, retry: start })
    }
    setStep({ pairing: { phone: phone.trim(), device, had: accounts.length } })
  }
  const paired = () => (setStep(null), reload())

  if (problem && !accounts) return <LoadFailure problem={problem} retry={retry} />
  if (!ready) return <ListSkeleton />
  if (!running.length && !unknown)
    return (
      <Problem
        warn
        title="WhatsApp isn’t on any of your devices yet."
        action={{ label: "Open App Store", run: () => one.open("app", { app: "whatsapp" }, "apps") }}
      />
    )

  const needs = accounts.filter((a) => !a.ok)
  return (
    <div className="flex flex-col gap-6">
      {!accounts.length && <Hint>Link WhatsApp, and your agents can read your chats and, when you ask, send.</Hint>}
      {needs.map((a) => (
        <Problem
          key={a.account}
          warn
          title={a.account + " needs linking again"}
          text={one.plain(a.why || "Needs linking again").title.replace(/^Needs linking again\.$/, "Link it from the phone again.")}
          action={{ label: "Link Again", run: () => start() }}
        />
      ))}
      {accounts.length > 0 && (
        <AccountList
          title="Accounts"
          accounts={accounts}
          failed={failed}
          signOut={(a) => signOut(a, { device: a.device })}
          confirm={(a) =>
            one.alert({
              title: `Sign out of ${a.account}?`,
              text: "This device is unlinked from it, and the messages kept for it are deleted. Linking again needs the phone.",
              action: "Sign Out",
              danger: true,
            })
          }
          line={(a, i) => (a.ok ? { text: `Linked on ${a.device}${i ? "" : " · used first"}` } : { text: "Link again", warn: true })}
        />
      )}
      {step?.pairing ? (
        <Pairing {...step.pairing} done={paired} />
      ) : (
        <section className="flex flex-col gap-2">
          <List>
            {running.length > 1 && (
              <ListRow
                title="Link on"
                trailing={
                  <Select value={device} onValueChange={setOn}>
                    <SelectTrigger size="sm" aria-label="Link on"><SelectValue /></SelectTrigger>
                    <SelectContent align="end" position="popper">
                      {running.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                    </SelectContent>
                  </Select>
                }
              />
            )}
            <div className="px-3 py-2.5">
              <Input ref={field} type="tel" placeholder="Phone number (optional)" aria-label="Phone number" value={phone} onChange={(e) => setPhone(e.target.value)} />
            </div>
          </List>
          {step?.why && <Problem title={step.why} action={{ label: "Try Again", run: step.retry }} />}
          <LoadingButton className="self-start" run={start}>Add Account</LoadingButton>
        </section>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Settings />)
