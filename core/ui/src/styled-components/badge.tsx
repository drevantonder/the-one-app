import { omit } from "solid-js"
import type { JSX } from "@solidjs/web"

/** A `Badge`'s props: a `<span>`'s, and how it looks. */
export interface BadgeProps extends JSX.HTMLAttributes<HTMLSpanElement> {
  variant?: "default" | "secondary" | "destructive" | "outline"
}

/// `<Badge variant="destructive" aria-label="3 waiting">3</Badge>`: a short count or word beside what it's about, as
/// shadcn's Badge. `destructive` is the red count of what waits for someone, as on an iPhone's icons; `default`,
/// `secondary` and `outline` are for a word that only says what something is. Anything else is a `<span>`'s.
export function Badge(props: BadgeProps) {
  const own = omit(props, "variant", "class", "children")
  return (
    <span {...own} class={["ui-badge", props.class]} data-variant={props.variant ?? "default"}>
      {props.children}
    </span>
  )
}
