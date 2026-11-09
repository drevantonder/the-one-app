import { Icon } from "@/components/icon"
import { cn } from "@/lib/utils"

type App = { name: string; title?: string; icon?: string; color?: string; image?: string; image_kind?: string }

/// What an app's tile is filled with, its colour's gradient or its picture, as a CSS `background`.
function iconBackground(app: App) {
  const color = /^#[0-9a-f]{3,8}$/i.test(app.color || "") ? app.color! : "#8e8e93"
  const image = iconImage(app)
  return image
    ? `${app.image_kind === "icon" ? "" : "white "}center / ${app.image_kind === "icon" ? "cover" : "70%"} no-repeat url(${JSON.stringify(image)})`
    : `linear-gradient(color-mix(in srgb, ${color} 80%, white), ${color})`
}
const iconImage = (app: App) => (/^(https:|data:image\/|\/v1\/images\/[0-9a-f]+$)/.test(app.image || "") ? app.image! : "")

/// An app's icon: a tile in the app's own colour (its `color`, as its app.js says), with its icon, or its first letter,
/// in white; or its picture, a finished icon edge to edge, else a logo inset on white.
function AppIcon({ app, size = 36, className }: { app: App; size?: number; className?: string }) {
  const image = iconImage(app)
  const letter = String(app.title || app.name || "?")[0].toUpperCase()
  return (
    <span
      data-slot="app-icon"
      className={cn("inline-grid shrink-0 place-items-center overflow-hidden font-semibold text-white", className)}
      style={{
        width: size,
        height: size,
        borderRadius: size * 0.225,
        fontSize: size * 0.42,
        background: iconBackground(app),
      }}
    >
      {!image && (app.icon ? <Icon name={`${app.icon}-fill`} className="size-[55%]" /> : letter)}
    </span>
  )
}

export { AppIcon, type App }
