import { WarningCircleIcon } from "@phosphor-icons/react"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button, LoadingButton } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/// A problem, shown on the thing it's about: a symbol, a short title, one sentence, and the action that fixes it. `warn`
/// for what can wait; `quiet` for what doesn't need the user (it mends itself), in no colour; otherwise it's red. Its
/// action shows it's working until `run` is done, so pressing Try Again never looks like nothing happened.
function Problem({ title, text, action, warn = false, quiet = false, className }: {
  title: string
  text?: string
  action?: { label: string; run: () => unknown }
  warn?: boolean
  quiet?: boolean
  className?: string
}) {
  return (
    <Alert data-slot="problem" className={cn(warn ? "text-warning" : quiet ? "text-foreground *:data-[slot=alert-description]:text-muted-foreground [&>svg]:text-muted-foreground" : "text-destructive", "max-w-2xl", className)}>
      <WarningCircleIcon weight="fill" />
      <AlertTitle className="line-clamp-none">{title}</AlertTitle>
      {(text || action) && (
        <AlertDescription>
          {text && <p>{text}</p>}
          {action && (
            <LoadingButton variant="outline" size="sm" className="mt-2 text-foreground" run={action.run}>
              {action.label}
            </LoadingButton>
          )}
        </AlertDescription>
      )}
    </Alert>
  )
}

/// What's wrong with a whole window, as one thin grey line across its top (being offline: it mends itself), with an action
/// only where pressing it does something.
function Banner({ children, action, className }: {
  children: React.ReactNode
  action?: { label: string; run: () => unknown }
  className?: string
}) {
  return (
    <div data-slot="banner" role="status" className={cn("flex min-h-9 shrink-0 items-center gap-3 border-b px-4 py-1 text-[13px] text-muted-foreground", className)}>
      <p className="min-w-0 flex-1 leading-snug">{children}</p>
      {action && <Button variant="ghost" size="xs" className="-mr-2" onClick={() => action.run()}>{action.label}</Button>}
    </div>
  )
}

export { Problem, Banner }
