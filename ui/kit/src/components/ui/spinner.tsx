import { CircleNotchIcon } from "@phosphor-icons/react"
import { cn } from "@/lib/utils"

function Spinner({ className }: { className?: string }) {
  return <CircleNotchIcon role="status" aria-label="Loading" data-slot="spinner" className={cn("size-4 animate-spin text-muted-foreground", className)} />
}

export { Spinner }
