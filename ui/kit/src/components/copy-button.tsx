import * as React from "react"
import { CheckIcon, CopyIcon } from "@phosphor-icons/react"
import { Button } from "@/components/ui/button"

type Bridge = { copy?: (text: string) => Promise<void>; tap?: (style?: string) => void }

/// A button that copies `text` and says so: a check and "Copied" for a moment. `label` is its text, or none for an
/// icon alone. It copies through the window's host where the screen has one (the bridge's `one.copy`), else the browser.
function CopyButton({ text, label = "Copy", variant = "ghost", size = "sm", className, labelClassName }: {
  text: string
  label?: string
  variant?: React.ComponentProps<typeof Button>["variant"]
  size?: React.ComponentProps<typeof Button>["size"]
  className?: string
  /// For the label's own classes, to hide it where there's no room (the icon stays).
  labelClassName?: string
}) {
  const [copied, setCopied] = React.useState(false)
  const timer = React.useRef<ReturnType<typeof setTimeout>>(undefined)
  React.useEffect(() => () => clearTimeout(timer.current), [])
  const copy = async () => {
    const one = (globalThis as { one?: Bridge }).one
    if (one?.copy) await one.copy(text)
    else await navigator.clipboard.writeText(text)
    one?.tap?.("success")
    setCopied(true)
    clearTimeout(timer.current)
    timer.current = setTimeout(() => setCopied(false), 1500)
  }
  const Glyph = copied ? CheckIcon : CopyIcon
  return (
    <Button
      data-slot="copy-button"
      type="button"
      variant={variant}
      size={label ? size : size === "sm" ? "icon-sm" : "icon"}
      className={className}
      aria-label={copied ? "Copied" : label || "Copy"}
      onClick={copy}
    >
      <Glyph />
      {label && <span className={labelClassName}>{copied ? "Copied" : label}</span>}
    </Button>
  )
}

export { CopyButton }
