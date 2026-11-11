import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/// An empty screen or list: one grey line, and a button only when there's an obvious first step.
function Empty({ children, action, className }: {
  children: React.ReactNode
  action?: { label: string; run: () => unknown }
  className?: string
}) {
  return (
    <div data-slot="empty" className={cn("flex flex-col items-start gap-4 py-2 text-sm text-muted-foreground", className)}>
      <p>{children}</p>
      {action && <Button onClick={() => action.run()}>{action.label}</Button>}
    </div>
  )
}

export { Empty }
