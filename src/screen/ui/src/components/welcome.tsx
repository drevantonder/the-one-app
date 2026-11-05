import { Problem } from "@/components/problem"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { cn } from "@/lib/utils"

// What a client shows before its window: signing in, waiting to be let in, starting, and its devices not answering.

function Centered({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("flex h-full flex-col items-center justify-center gap-3 overflow-auto p-10 text-center", className)}>{children}</div>
}

/// Its logo and name, and the one thing to do (Sign In; Open, back from another site), as a button that runs or a link;
/// `problem`, why the last try didn't work.
function Welcome({ logo, name, action, problem }: {
  logo?: string
  name: string
  action: { label: string; run?: () => unknown; href?: string }
  problem?: string
}) {
  return (
    <Centered>
      {logo && <img src={logo} alt="" className="size-28" />}
      <h1 className="mt-2 mb-4 text-3xl font-semibold tracking-tight">{name}</h1>
      {action.href ? (
        <Button size="lg" data-slot="welcome-action" asChild><a href={action.href}>{action.label}</a></Button>
      ) : (
        <Button size="lg" data-slot="welcome-action" onClick={(e) => ((e.currentTarget.disabled = true), action.run?.())}>{action.label}</Button>
      )}
      {problem && <Problem title={problem} className="mt-4 w-full max-w-md text-left" />}
    </Centered>
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

/// A wait the person sees: a spinner and what it's for.
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
