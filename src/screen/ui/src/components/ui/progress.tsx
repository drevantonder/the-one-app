"use client"

import * as React from "react"
import { cn } from "@/lib/utils"
import { Progress as ProgressPrimitive } from "radix-ui"

/// shadcn's progress: a bar filled to `value`, a percent. With `meter`, for how much of something is used: neutral
/// until it needs you, amber from 70, red from 85.
function Progress({ className, value, meter = false, ...props }: Omit<React.ComponentProps<typeof ProgressPrimitive.Root>, "max"> & {
  meter?: boolean
}) {
  const shown = value == null ? undefined : Math.min(100, Math.max(0, value))
  const tone = meter && shown !== undefined ? (shown >= 85 ? "bg-destructive" : shown >= 70 ? "bg-warning" : "bg-foreground") : "bg-foreground"
  return (
    <ProgressPrimitive.Root
      data-slot="progress"
      value={shown}
      className={cn("relative h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}
      {...props}
    >
      <ProgressPrimitive.Indicator
        data-slot="progress-indicator"
        className={cn("h-full rounded-full transition-[width]", tone)}
        style={{ width: `${shown ?? 0}%` }}
      />
    </ProgressPrimitive.Root>
  )
}

export { Progress }
