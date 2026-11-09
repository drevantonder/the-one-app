import { Problem } from "@/components/problem"
import * as React from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { appear, rise, transition } from "./motion"

// What a client shows before its window, and auth's pages: signing in, waiting to be let in, starting. One screen at a
// time, each with the logo in the same place.

const pressed = "active:scale-[.97]"

/// A screen before the window: its logo, and what's under it, which rises in as the screen arrives (a new `key` is a new
/// screen; the same one drawn again doesn't rise).
function Stage({ logo, children }: { logo?: string; children: React.ReactNode }) {
  const stack = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => rise(stack.current!), [])
  return (
    <div data-slot="welcome">
      <div ref={stack} data-slot="welcome-stack" className="flex w-full max-w-sm flex-col items-center gap-3 text-center">
        {logo && <img data-slot="welcome-logo" src={logo} alt="" />}
        {children}
      </div>
    </div>
  )
}

/// The first screen, and every page of auth's (pages.tsx, drawn to HTML when this kit is built): its logo
/// and `title`; `text`, a grey line under it; `field`, what to type first (a native app's address, a sign-in code),
/// which the button runs with; `children`, what the page lists; `callout`, what to know before going on;
/// `action`, the one thing to do, if any, as a button that runs or a link, named `invite` instead when what's typed is an invite
/// link; `other`, a second way in (a Mac's Join with a Pairing Code), or back from it; `note`, a grey line under them;
/// `problem`, why the last try didn't work. Near the top, not centred: what comes in later (a problem, a client's name)
/// grows it down, never moving what's already there. A text, a callout, a note and a problem have their places even when empty, for a
/// page's script to fill.
function Welcome({ logo, title, text, field, callout, children, action, other, note, problem }: {
  logo?: string
  title: string
  text?: string
  field?: { label: string; placeholder?: string }
  callout?: string
  children?: React.ReactNode
  action?: { label: string; invite?: string; run?: (typed?: string) => unknown; href?: string }
  other?: { label: string; run?: () => unknown; href?: string }
  note?: string
  problem?: string
}) {
  const [typed, setTyped] = React.useState("")
  const [busy, setBusy] = React.useState(false)
  const run = async () => {
    setBusy(true)
    try { await action?.run?.(field ? typed : undefined) } finally { setBusy(false) }
  }
  const wide = `mt-2 min-w-56 ${pressed}`
  const told = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => { if (problem) appear(told.current!) }, [problem])
  return (
    <Stage logo={logo}>
        <h1 data-slot="welcome-title" className="mt-2 text-3xl font-semibold tracking-tight text-balance">{title}</h1>
        <p data-slot="welcome-text" className="text-sm text-balance text-muted-foreground empty:hidden">{text}</p>
        {field && (
          <form className="mt-2 w-full max-w-xs" onSubmit={(e) => (e.preventDefault(), typed.trim() && !busy && run())}>
            <Input data-slot="welcome-field" aria-label={field.label} placeholder={field.placeholder} value={typed} onChange={(e) => setTyped(e.target.value)}
              autoFocus autoCapitalize="none" autoCorrect="off" spellCheck={false} inputMode="url" />
          </form>
        )}
        {children}
        <p data-slot="welcome-callout" className="w-full text-left text-sm empty:hidden">{callout}</p>
        {action && (action.href ? (
          <Button size="lg" data-slot="welcome-action" className={wide} asChild><a href={action.href}>{action.label}</a></Button>
        ) : (
          <Button size="lg" data-slot="welcome-action" className={wide} disabled={busy || (!!field && !typed.trim())} onClick={run}>{action.invite && typed.includes("/invite#") ? action.invite : action.label}</Button>
        ))}
        {other && (other.href
          ? <Button variant="ghost" className={pressed} data-slot="welcome-other" asChild><a href={other.href}>{other.label}</a></Button>
          : <Button variant="ghost" className={pressed} data-slot="welcome-other" onClick={() => other.run?.()}>{other.label}</Button>)}
        <p data-slot="welcome-note" className="text-sm text-balance text-muted-foreground empty:hidden">{note}</p>
        <div ref={told} data-slot="welcome-problem" className="mt-3 w-full text-left empty:hidden">{problem && <Problem title={problem} />}</div>
    </Stage>
  )
}

/// A device waiting to be let in: its code, the code as a QR (`qr`, an SVG) that opens Approve with it, what to do with
/// them (`text`), and Sign Out.
function Join({ logo, code, qr, text, onSignOut }: { logo?: string; code: string; qr: string; text: string; onSignOut: () => void }) {
  return (
    <Welcome logo={logo} title="Waiting for Approval" text={text} other={{ label: "Sign Out", run: onSignOut }} note="The owner of this network can see everything in it.">
      <div aria-label="Code" data-slot="join-code" className="my-3 font-mono text-4xl font-semibold tracking-widest select-all">{code}</div>
      <div className="overflow-hidden rounded-lg [&_svg]:block [&_svg]:size-44" dangerouslySetInnerHTML={{ __html: qr }} />
    </Welcome>
  )
}

/// Waiting for the window: the logo, and a small spinner under it.
function Starting({ logo }: { logo?: string }) {
  return (
    <Stage logo={logo}>
      <Spinner className="mt-3 size-5" />
    </Stage>
  )
}

/// A screen in the window that couldn't open: the problem, where it would be.
function Unavailable({ title, text }: { title: string; text?: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center overflow-auto p-10">
      <Problem title={title} text={text} className="w-full max-w-md text-left" />
    </div>
  )
}

export { Welcome, Join, Starting, Unavailable, transition }
