"use client"

import * as React from "react"
import { cn } from "@/lib/utils"
import { Avatar as AvatarPrimitive } from "radix-ui"

// What someone with no picture gets: a steady colour from their id.
const COLORS = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"]

function colorOf(id: string | number) {
  let h = 0
  for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0
  return COLORS[h % COLORS.length]
}

function initialsOf(name: string) {
  return String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase()
}

/// shadcn's avatar: a circle with a picture (`AvatarImage`) and what shows until it's there or without one
/// (`AvatarFallback`). `size` is `sm`, `default` or `lg`, or a number of pixels.
///
/// Given a `name` it draws their initials itself, grey; with an `id` too, white on a colour that stays the same for
/// that id, so a person or group is the same colour wherever they're shown.
function Avatar({ className, size = "default", name, id, style, children, ...props }: React.ComponentProps<typeof AvatarPrimitive.Root> & {
  size?: "sm" | "default" | "lg" | number
  name?: string
  id?: string | number
}) {
  const px = typeof size === "number"
  const named = name !== undefined
  return (
    <AvatarPrimitive.Root
      data-slot="avatar"
      data-size={px ? undefined : size}
      aria-hidden={named || undefined}
      className={cn(
        "group/avatar relative flex size-8 shrink-0 overflow-hidden rounded-full select-none data-[size=lg]:size-10 data-[size=sm]:size-6",
        className
      )}
      style={{ ...(px && { width: size, height: size }), ...style }}
      {...props}
    >
      {children}
      {named && (
        <AvatarFallback
          className={cn("font-semibold", id === undefined ? "text-muted-foreground" : "text-white")}
          style={{ ...(px && { fontSize: size * 0.4 }), ...(id !== undefined && { background: colorOf(id) }) }}
        >
          {initialsOf(name)}
        </AvatarFallback>
      )}
    </AvatarPrimitive.Root>
  )
}

function AvatarImage({ className, ...props }: React.ComponentProps<typeof AvatarPrimitive.Image>) {
  return <AvatarPrimitive.Image data-slot="avatar-image" className={cn("aspect-square size-full", className)} {...props} />
}

function AvatarFallback({ className, ...props }: React.ComponentProps<typeof AvatarPrimitive.Fallback>) {
  return (
    <AvatarPrimitive.Fallback
      data-slot="avatar-fallback"
      className={cn("flex size-full items-center justify-center rounded-full bg-muted text-sm group-data-[size=sm]/avatar:text-xs", className)}
      {...props}
    />
  )
}

export { Avatar, AvatarFallback, AvatarImage }
