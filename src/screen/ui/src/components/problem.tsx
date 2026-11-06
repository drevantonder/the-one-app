import { WarningCircleIcon } from "@phosphor-icons/react"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/// A problem, shown on the thing it's about: a symbol, a short title, one sentence, and the action that fixes it. `warn`
/// for what can wait; `quiet` for what doesn't need the person (it mends itself), in no colour; otherwise it's red.
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
            <Button variant="outline" size="sm" className="mt-2 text-foreground" onClick={() => action.run()}>
              {action.label}
            </Button>
          )}
        </AlertDescription>
      )}
    </Alert>
  )
}

export { Problem }
