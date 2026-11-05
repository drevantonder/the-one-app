import { cn } from "@/lib/utils"

/// A Phosphor icon by its name ("magnifying-glass", "check-circle-fill"), at the text's size and colour, for a screen
/// with no build of its own: the bridge asks its host for the icon's path and fills it in (bridge.js). A screen built
/// with Vite imports the icons it uses from @phosphor-icons/react instead.
function Icon({ name, className }: { name: string; className?: string }) {
  return (
    <svg
      key={name}
      data-slot="icon"
      data-icon={name}
      viewBox="0 0 256 256"
      fill="currentColor"
      aria-hidden="true"
      className={cn("size-4 shrink-0", className)}
    />
  )
}

export { Icon }
