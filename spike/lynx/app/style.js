// The strict style subset (protocol.ts `Style`) as CSS declarations. `colour(name)` resolves a colour token: the DOM
// host gives `var(--name)`, Lynx a value from tokens.css. Box defaults are flex column in both hosts (React Native's).
const SPACE = { gap: 16, gutter: 16 }
const SIZE = { control: 36, icon: 16, readable: 672 }
const RADIUS = { 0: 0, sm: 6, md: 8, lg: 10, xl: 14, full: 9999 }
// size px, weight, tracking em (core/ui/src/tokens.css: --text-*, --*-weight, --*-tracking)
export const FONT = {
  body: [14, 450, 0],
  label: [14, 550, 0],
  title: [28, 650, -0.015],
  figure: [28, 600, -0.025],
  subheadline: [15, 450, 0],
  footnote: [13, 450, 0],
  "caption-1": [12, 450, 0],
  "caption-2": [11, 450, 0],
}

const px = (v) => (typeof v === "number" ? `${v}px` : v)
const space = (v) => px(SPACE[v] ?? v)
const size = (v) => px(SIZE[v] ?? v)

export function css(style, colour, lynx) {
  const out = []
  const add = (k, v) => out.push(`${k}:${v}`)
  for (const [k, v] of Object.entries(style || {})) {
    if (v == null) continue
    switch (k) {
      case "padding": case "paddingTop": case "paddingRight": case "paddingBottom": case "paddingLeft":
      case "margin": case "gap": case "rowGap": case "columnGap":
        add(kebab(k), space(v)); break
      case "paddingInline": add("padding-left", space(v)); add("padding-right", space(v)); break
      case "paddingBlock": add("padding-top", space(v)); add("padding-bottom", space(v)); break
      case "marginInline": add("margin-left", space(v)); add("margin-right", space(v)); break
      case "marginBlock": add("margin-top", space(v)); add("margin-bottom", space(v)); break
      case "width": case "height": case "minWidth": case "minHeight": case "maxWidth": case "maxHeight": case "flexBasis":
        add(kebab(k), size(v)); break
      case "backgroundColor": case "color": case "borderColor": add(kebab(k), colour(v)); break
      case "borderWidth": case "borderTopWidth": case "borderRightWidth": case "borderBottomWidth": case "borderLeftWidth":
        add(kebab(k), `${v}px`); if (v) add(kebab(k).replace("width", "style"), "solid"); break
      case "borderRadius": add("border-radius", px(RADIUS[v])); break
      case "font": {
        const [s, w, t] = FONT[v]
        add("font-size", `${s}px`); add("font-weight", lynx ? (w >= 600 ? "bold" : "normal") : w)
        if (t) add("letter-spacing", lynx ? `${(t * s).toFixed(2)}px` : `${t}em`)
        break
      }
      case "textAlign": add("text-align", v === "start" ? "left" : v === "end" ? "right" : v); break
      case "lineClamp":
        // Lynx: the text's `text-maxline` attribute (set by the host); CSS: line-clamp.
        if (!lynx) { add("display", "-webkit-box"); add("-webkit-box-orient", "vertical"); add("-webkit-line-clamp", v); add("overflow", "hidden") }
        add("text-overflow", "ellipsis"); break
      case "fontVariantNumeric": if (!lynx) add("font-variant-numeric", v); break
      case "overflow": add("overflow", v === "scroll" ? (lynx ? "hidden" : "auto") : v); break
      default: add(kebab(k), typeof v === "number" && !["flexGrow", "flexShrink", "opacity", "aspectRatio"].includes(k) ? `${v}px` : v)
    }
  }
  return out.join(";")
}

const kebab = (s) => s.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)
