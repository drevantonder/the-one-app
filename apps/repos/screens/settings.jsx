// Repos' Settings: GitHub, connected. An admin makes the network's GitHub App once; each user signs in to their own
// GitHub accounts; a group's admins connect an organization for the group. GitHub sends the browser back here with
// { back: the address it came back to } to finish (to github_app, this screen too, after making the App). Opened with
// { add: true } (a shell's Add Account), it starts signing in at once.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Card, CardContent, Empty, Icon, List, ListRow, Problem, Spinner, Pending, useCall } from "@one/ui"
import { Frame, Section, said } from "../lib/use.jsx"

const P = one.params

function Offer({ offer }) {
  return (
    <Section title={offer.title}>
      <Card className="max-w-2xl gap-3">
        <CardContent className="flex flex-col items-start gap-3">
          <Button asChild><a href={offer.url}>Continue to GitHub</a></Button>
        </CardContent>
      </Card>
    </Section>
  )
}

function Settings() {
  const github = useCall("github", {})
  const accounts = useCall("accounts", {})
  const [trouble, setTrouble] = useState("")
  const [offer, setOffer] = useState(null)
  const [finishing, setFinishing] = useState(false)
  const [out, setOut] = useState("")
  const d = github.data
  const redraw = () => (github.reload(), accounts.reload())
  const fail = (err) => setTrouble(said(err).title)
  const started = useRef(false)

  useEffect(() => {
    one.title("Settings")
  }, [])

  const createApp = async () => {
    try {
      setOffer({ url: (await one.call("create_github_app")).url, title: "Create GitHub App" })
    } catch (err) {
      fail(err)
    }
  }
  const start = async (account) => {
    try {
      setOffer({ url: (await one.call("sign_in", account ? { account } : {})).url, title: "Connect GitHub" })
    } catch (err) {
      fail(err)
    }
  }
  const connectOrg = async (button) => {
    const pick = d.groups.length === 1 ? 0 : await one.menu(d.groups.map((g) => ({ title: g.name })), button)
    if (pick === null) return
    try {
      setOffer({ url: (await one.call("connect_org", { group: d.groups[pick].handle })).url, title: `Connect an Organization for ${d.groups[pick].name}` })
    } catch (err) {
      fail(err)
    }
  }
  const finish = async (back) => {
    setFinishing(true)
    try {
      const q = new URL(back).searchParams
      if (q.get("error")) throw new Error(`GitHub didn’t finish (${q.get("error_description") || q.get("error")}). Try again.`)
      if (one.screen === "github_app") {
        await one.call("finish_github_app", { code: back })
      } else if (q.get("installation_id")) {
        await one.call("finish_connect_org", { back })
      } else if (q.get("code")) {
        const r = await one.call("finish_sign_in", { code: back })
        if (r.signed_in === false) throw new Error(r.why)
      }
      setTrouble("")
    } catch (err) {
      fail(err)
    }
    setFinishing(false)
    redraw()
  }
  useEffect(() => {
    if (started.current) return
    started.current = true
    if (P.back) finish(P.back)
    else if (P.add) start()
  }, [])

  const signOut = async (account) => {
    setOut(account)
    try {
      await one.call("sign_out", { account })
      redraw()
    } catch (err) {
      fail(err)
    }
    setOut("")
  }
  const disconnect = async (org) => {
    const ok = await one.alert({ title: `Disconnect ${org}?`, text: "Its repos no longer show for its group here. The App stays installed on GitHub until you uninstall it there.", action: "Disconnect", danger: true })
    if (!ok) return
    try {
      await one.call("disconnect_org", { org, confirm: true })
      redraw()
    } catch (err) {
      fail(err)
    }
  }

  if (!d || !accounts.data) return <Frame><Pending call={d ? accounts : github} rows={3} /></Frame>

  let body
  if (!d.app) {
    body = d.admin
      ? <Empty action={{ label: "Create GitHub App", run: createApp }}>Connect GitHub, and everyone’s GitHub repos show here beside your own.</Empty>
      : <Empty>An admin connects GitHub once, for everyone.</Empty>
  } else {
    const needs = accounts.data.filter((a) => !a.ok)
    body = (
      <>
        {needs.map((a) => (
          <Problem key={a.account} warn title={`${a.account} needs signing in`} text={said(a.why).title} action={{ label: "Sign In", run: () => start(a.account) }} />
        ))}
        <Section title="GitHub Accounts">
          <List>
            {accounts.data.map((a, i) => (
              <ListRow
                key={a.account}
                title={a.account}
                sub={a.ok ? (i ? undefined : "Used first") : "Sign in again"}
                trailing={<Button variant="ghost" size="sm" disabled={out === a.account} onClick={() => signOut(a.account)}>{out === a.account && <Spinner />}Sign Out</Button>}
              />
            ))}
            <ListRow leading={<Icon name="plus" />} title={accounts.data.length ? "Add Account" : "Connect GitHub"} onClick={() => start()} />
          </List>
        </Section>
        {(d.orgs.length > 0 || d.groups.length > 0) && (
          <Section title="Organizations">
            <List>
              {d.orgs.map((o) => (
                <ListRow
                  key={o.org}
                  title={o.org}
                  sub={`For ${o.group || "a group"}`}
                  trailing={o.yours ? <Button variant="ghost" size="sm" onClick={() => disconnect(o.org)}>Disconnect</Button> : undefined}
                />
              ))}
              {d.groups.length > 0 && <ListRow leading={<Icon name="plus" />} title="Connect an Organization" onClick={(e) => connectOrg(e.currentTarget)} />}
            </List>
            <p className="px-1 text-xs text-muted-foreground">An organization’s repos show for everyone in its group.</p>
          </Section>
        )}
        {d.admin && (
          <p className="text-xs text-muted-foreground">
            GitHub App:{" "}
            <button type="button" className="underline underline-offset-4 hover:text-foreground" onClick={() => one.link(d.app.url)}>{d.app.slug}</button>
          </p>
        )}
      </>
    )
  }
  if (!d.app) {
    return (
      <Frame>
        {trouble && <Problem title={trouble} />}
        {body}
        {offer && <Offer offer={offer} />}
        {finishing && <p className="flex items-center gap-2 text-sm text-muted-foreground"><Spinner />Finishing</p>}
      </Frame>
    )
  }
  return (
    <Frame>
      <div className="flex max-w-2xl flex-col gap-6">
        {trouble && <Problem title={trouble} />}
        {body}
        {offer && <Offer offer={offer} />}
        {finishing && <p className="flex items-center gap-2 text-sm text-muted-foreground"><Spinner />Finishing</p>}
      </div>
    </Frame>
  )
}

createRoot(document.getElementById("root")).render(<Settings />)
