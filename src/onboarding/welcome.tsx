import { Problem } from "@/components/problem"
import * as React from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { cn } from "@/lib/utils"

// What a client shows before its window, and auth's pages: signing in, waiting to be let in, starting, and its devices
// not answering.

function Centered({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("flex h-full flex-col items-center justify-center gap-3 overflow-auto p-10 text-center", className)}>{children}</div>
}

/// The first screen, and every page of auth's (pages.tsx, drawn to HTML when this kit is built): its logo
/// and `title`; `text`, a grey line under it; `field`, what to type first (a native app's address, a sign-in code), with
/// a line under it, which the button runs with; `children`, what the page lists; `callout`, what to know before going on;
/// `action`, the one thing to do, if any, as a button that runs or a link, named `invite` instead when what's typed is an invite
/// link; `other`, a second way in (a Mac's Join with a Pairing Code), or back from it; `note`, a grey line under them;
/// `problem`, why the last try didn't work. Near the top, not centred: what comes in later (a problem, a client's name)
/// grows it down, never moving what's already there. A callout and a problem have their places even when empty, for a
/// page's script to fill.
function Welcome({ logo, title, text, field, callout, children, action, other, note, problem }: {
  logo?: string
  title: string
  text?: string
  field?: { label: string; placeholder?: string; note?: string }
  callout?: string
  children?: React.ReactNode
  action?: { label: string; invite?: string; run?: (typed?: string) => unknown; href?: string }
  other?: { label: string; run?: () => unknown; href?: string }
  note?: string
  problem?: string
}) {
  const [typed, setTyped] = React.useState("")
  const [busy, setBusy] = React.useState(false)
  const run = () => (setBusy(true), action?.run?.(field ? typed : undefined))
  const wide = "mt-2 min-w-56"
  return (
    <div data-slot="welcome" className="flex h-full flex-col items-center overflow-auto px-6 pt-[max(40px,14vh)] pb-10">
      <div className="flex w-full max-w-sm flex-col items-center gap-3 text-center">
        {logo && <img data-slot="welcome-logo" src={logo} alt="" className="size-28" />}
        <h1 data-slot="welcome-title" className="mt-2 text-3xl font-semibold tracking-tight text-balance">{title}</h1>
        {text && <p data-slot="welcome-text" className="text-sm text-balance text-muted-foreground">{text}</p>}
        {field && (
          <form className="mt-2 w-full max-w-xs" onSubmit={(e) => (e.preventDefault(), typed.trim() && !busy && run())}>
            <Input data-slot="welcome-field" aria-label={field.label} placeholder={field.placeholder} value={typed} onChange={(e) => setTyped(e.target.value)}
              autoFocus autoCapitalize="none" autoCorrect="off" spellCheck={false} inputMode="url" />
            {field.note && <p className="mt-2 text-sm text-muted-foreground">{field.note}</p>}
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
          ? <Button variant="ghost" data-slot="welcome-other" asChild><a href={other.href}>{other.label}</a></Button>
          : <Button variant="ghost" data-slot="welcome-other" onClick={() => other.run?.()}>{other.label}</Button>)}
        {note && <p data-slot="welcome-note" className="text-sm text-balance text-muted-foreground">{note}</p>}
        <div data-slot="welcome-problem" className="mt-3 w-full text-left empty:hidden">{problem && <Problem title={problem} />}</div>
      </div>
    </div>
  )
}

/// A device waiting to be let in: its code, the code as a QR (`qr`, an SVG) that opens Approve with it, what to do with
/// each (`text`, `note`), and Sign Out.
function Join({ code, qr, text, note, onSignOut }: { code: string; qr: string; text: string; note: string; onSignOut: () => void }) {
  return (
    <Centered>
      <h1 className="text-3xl font-semibold tracking-tight">Waiting for Approval</h1>
      <p className="max-w-sm text-sm text-muted-foreground">{text}</p>
      <div aria-label="Code" data-slot="join-code" className="my-3 font-mono text-4xl font-semibold tracking-widest select-all">{code}</div>
      <div className="overflow-hidden rounded-lg [&_svg]:block [&_svg]:size-44" dangerouslySetInnerHTML={{ __html: qr }} />
      <p className="max-w-sm text-sm text-muted-foreground">{note}</p>
      <Button variant="outline" className="mt-3" onClick={onSignOut}>Sign Out</Button>
    </Centered>
  )
}

/// A wait the user sees: a spinner and what it's for.
function Starting({ label = "Starting" }: { label?: string }) {
  return (
    <Centered className="flex-row gap-2 text-sm text-muted-foreground">
      <Spinner />
      {label}
    </Centered>
  )
}

/// What stops the window: the problem, and one grey line under it (`note`, "Trying again…").
function Unavailable({ title, text, note }: { title: string; text?: string; note?: string }) {
  return (
    <Centered>
      <Problem title={title} text={text} className="w-full max-w-md text-left" />
      {note && <p className="text-sm text-muted-foreground">{note}</p>}
    </Centered>
  )
}

export { Welcome, Join, Starting, Unavailable }
