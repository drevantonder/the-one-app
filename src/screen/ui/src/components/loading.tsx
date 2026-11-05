import * as React from "react"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Problem } from "@/components/problem"

/// A failure's first sentence, without "Try again." (what the button says), as the window words it (`one.plain`).
function failureSentence(err: unknown): string {
  const one = (globalThis as unknown as { one: { plain: (e: unknown) => { title: string } } }).one
  return one.plain(err).title.replace(/(\. Try again)?\.$/, "")
}

/// What a failed first load says (`problem` is `one.plain`'s answer), with Try Again.
function LoadFailure({ problem, retry }: { problem: { title: string; next?: string }; retry: () => unknown }) {
  return <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: retry }} />
}

/// A button that's working: disabled, with a spinner, until its `run` is done.
function LoadingButton({ run, children, ...props }: Omit<React.ComponentProps<typeof Button>, "onClick"> & { run: () => unknown }) {
  const [busy, setBusy] = React.useState(false)
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

/// Grey shapes where a list's rows will be, once it's taken longer than a blink, so a quick load shows nothing between.
function ListSkeleton({ rows = 2 }: { rows?: number }) {
  const [late, setLate] = React.useState(false)
  React.useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  if (!late) return null
  return (
    <div data-slot="list-skeleton" className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, i) => <Skeleton key={i} className="h-14 w-full max-w-2xl" />)}
    </div>
  )
}

/// One grey line with a spinner, for what's under way.
function Loading({ children }: { children: React.ReactNode }) {
  return (
    <p data-slot="loading" className="flex items-center gap-2 text-sm text-muted-foreground">
      <Spinner />
      {children}
    </p>
  )
}

export { failureSentence, Loading, LoadFailure, LoadingButton, ListSkeleton }
