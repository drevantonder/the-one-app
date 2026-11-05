import * as React from "react"
import { CopyButton } from "@/components/copy-button"
import { highlight } from "@/lib/highlight"
import { parseMarkdown, type Block, type Item } from "@/lib/markdown"
import { cn } from "@/lib/utils"

// What the text's own inline marks (a link, an image, `code`) look like; src/lib/markdown.ts makes their HTML.
const inline = [
  "[&_a]:underline [&_a]:underline-offset-2 [&_img]:max-w-full",
  "[&_:not(pre)>code]:rounded-md [&_:not(pre)>code]:bg-muted [&_:not(pre)>code]:px-[0.4em] [&_:not(pre)>code]:py-[0.15em] [&_:not(pre)>code]:font-mono [&_:not(pre)>code]:text-[0.86em]",
].join(" ")

const heading = [
  "mt-6 mb-3 font-semibold leading-tight",
  "text-[1.75em] border-b pb-[0.3em]",
  "text-[1.4em] border-b pb-[0.3em]",
  "text-[1.15em]",
  "text-[1em]",
  "text-[0.9em] text-muted-foreground",
  "text-[0.9em] text-muted-foreground",
]

const Html = ({ as: Tag = "span", html, className }: { as?: React.ElementType; html: string; className?: string }) => (
  <Tag className={className} dangerouslySetInnerHTML={{ __html: html }} />
)

/// Code, coloured for its language, with Copy at its corner.
function Code({ text, lang }: { text: string; lang: string }) {
  const lines = React.useMemo(() => highlight(text, lang), [text, lang])
  return (
    <div className="relative mb-4" data-slot="markdown-code">
      <pre className="m-0 overflow-x-auto rounded-lg bg-muted py-3.5 pr-12 pl-4 font-mono text-[0.8125rem] leading-[1.45]">
        <code>
          {lines.map((tokens, n) => (
            <React.Fragment key={n}>
              {n > 0 && "\n"}
              {tokens.map(([kind, piece], i) => (kind ? <span key={i} className={`hl-${kind}`}>{piece}</span> : piece))}
            </React.Fragment>
          ))}
        </code>
      </pre>
      <CopyButton text={text} label="" variant="outline" size="sm" className="absolute top-1.5 right-1.5 bg-card" />
    </div>
  )
}

// Mermaid, pinned and checked, from a CDN, only once a page has a diagram to draw.
const MERMAID = {
  src: "https://cdn.jsdelivr.net/npm/mermaid@11.12.1/dist/mermaid.min.js",
  integrity: "sha384-LlKSgo4Eo5GuF/ZrstLti44dE+GC5XAJ7TSu0Nw9Q3vIZF2QMnkRcK7BUoLabYLF",
}
type Mermaid = { initialize: (options: object) => void; render: (id: string, text: string) => Promise<{ svg: string }> }
let mermaid: Promise<Mermaid> | undefined
let drawn = 0

// A theme colour as the rgb() Mermaid can read, whatever space the theme gives it in.
function colour(name: string) {
  const c = document.createElement("canvas").getContext("2d", { willReadFrequently: true })!
  c.fillStyle = getComputedStyle(document.documentElement).getPropertyValue(name).trim() || "#888"
  c.fillRect(0, 0, 1, 1)
  const [r, g, b] = c.getImageData(0, 0, 1, 1).data
  return `rgb(${r}, ${g}, ${b})`
}

function loadMermaid() {
  mermaid ||= new Promise<Mermaid>((ok, fail) => {
    const s = Object.assign(document.createElement("script"), { src: MERMAID.src, integrity: MERMAID.integrity, crossOrigin: "anonymous" })
    s.onload = () => ((window as { mermaid?: Mermaid }).mermaid ? ok((window as { mermaid?: Mermaid }).mermaid!) : fail(new Error("no mermaid")))
    s.onerror = fail
    document.head.append(s)
  }).then((m) => {
    const dark = matchMedia("(prefers-color-scheme: dark)").matches
    m.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: "base",
      fontFamily: getComputedStyle(document.body).fontFamily,
      themeVariables: {
        darkMode: dark, fontSize: "14px", background: "transparent",
        primaryColor: colour("--card"), primaryTextColor: colour("--foreground"), primaryBorderColor: colour("--muted-foreground"),
        secondaryColor: colour("--background"), tertiaryColor: colour("--background"), lineColor: colour("--muted-foreground"), textColor: colour("--foreground"),
        noteBkgColor: colour("--background"), noteTextColor: colour("--foreground"), noteBorderColor: colour("--border"),
      },
      flowchart: { curve: "basis", htmlLabels: false },
    })
    return m
  })
  return mermaid
}

/// A ```mermaid block: its code until the diagram is drawn, and for good if Mermaid can't draw it.
function Diagram({ text }: { text: string }) {
  const [svg, setSvg] = React.useState<string | null>(null)
  const box = React.useRef<HTMLDivElement>(null)
  React.useEffect(() => {
    let current = true
    const id = `mermaid-${++drawn}`
    loadMermaid()
      .then((m) => m.render(id, text))
      .then((r) => current && setSvg(r.svg))
      .catch(() => document.getElementById(`d${id}`)?.remove())
    return () => { current = false }
  }, [text])
  React.useLayoutEffect(() => {
    const el = box.current?.querySelector("svg")
    if (!el) return
    el.removeAttribute("height")
    el.querySelectorAll("rect").forEach((r) => !r.getAttribute("rx") && r.setAttribute("rx", "6"))
  }, [svg])
  return svg ? <div ref={box} className="mb-4" dangerouslySetInnerHTML={{ __html: svg }} /> : <Code text={text} lang="" />
}

function Entry({ item }: { item: Item }) {
  const [first, ...rest] = item.blocks
  // A tight list's first paragraph has no space around it.
  const lead = item.tight && first?.type === "paragraph"
  return (
    <li className={cn("[&>ol]:mt-[0.25em] [&>ol]:mb-0 [&>ul]:mt-[0.25em] [&>ul]:mb-0", item.task && "-ml-6 list-none")}>
      {item.task && <input type="checkbox" disabled checked={item.task === "done"} readOnly className="mr-[0.3em] mb-[0.2em] align-middle" />}
      {item.task && " "}
      {lead ? <Html html={first.html} /> : null}
      <Blocks blocks={lead ? rest : item.blocks} />
    </li>
  )
}

function Blocks({ blocks }: { blocks: Block[] }) {
  return blocks.map((b, i) => {
    switch (b.type) {
      case "heading":
        return <Html key={i} as={`h${b.level}` as "h1"} html={b.html} className={cn(heading[0], heading[b.level])} />
      case "paragraph":
        return <Html key={i} as="p" html={b.html} className="mb-4" />
      case "code":
        return b.lang === "mermaid" ? <Diagram key={i} text={b.text} /> : <Code key={i} text={b.text} lang={b.lang} />
      case "rule":
        return <hr key={i} className="my-6 h-px border-0 bg-border" />
      case "quote":
        return (
          <blockquote key={i} className="mb-4 border-l-[0.25em] px-[1em] text-muted-foreground [&>:last-child]:mb-0">
            <Blocks blocks={b.blocks} />
          </blockquote>
        )
      case "alert":
        return (
          <div key={i} data-kind={b.kind} className="mb-4 border-l-[0.25em] px-4 py-2 [&>:last-child]:mb-0">
            <p className="mb-1 font-semibold">{b.kind[0].toUpperCase() + b.kind.slice(1)}</p>
            <Blocks blocks={b.blocks} />
          </div>
        )
      case "table":
        return (
          <div key={i} className="mb-4 overflow-x-auto">
            <table className="border-collapse text-sm [&_td]:border [&_td]:px-[0.8rem] [&_td]:py-1.5 [&_th]:border [&_th]:px-[0.8rem] [&_th]:py-1.5 [&_th]:font-semibold [&_tr:nth-child(2n)_td]:bg-muted/50">
              <thead>
                <tr>{b.head.map((c, k) => <Html key={k} as="th" html={c} className={align(b.align[k])} />)}</tr>
              </thead>
              <tbody>
                {b.rows.map((row, r) => (
                  <tr key={r}>{row.map((c, k) => <Html key={k} as="td" html={c} className={align(b.align[k])} />)}</tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      case "list": {
        const List = b.ordered ? "ol" : "ul"
        return (
          <List key={i} start={b.ordered && b.start !== 1 ? b.start : undefined} className={cn("mb-4 pl-8 [&>li+li]:mt-[0.25em]", b.ordered ? "list-decimal" : "list-disc")}>
            {b.items.map((item, k) => <Entry key={k} item={item} />)}
          </List>
        )
      }
    }
  })
}

const align = (a?: string) => (a === "left" ? "text-left" : a === "center" ? "text-center" : a === "right" ? "text-right" : undefined)

/// Markdown as GitHub draws it: headings, lists and task lists, quotes and alerts, tables, code coloured for its language
/// with Copy, links, images and ```mermaid diagrams. Raw HTML is never drawn, only its text; a link goes only to
/// http(s), mailto, a place in the page or a path, and an image draws only from http(s), else its alt text shows.
///
/// A link to a path in the text (a README's link to another file) goes where `onLink(path)` says, given the path as it's
/// written; with no `onLink` it's an ordinary link. Keep `onLink` the same function while its page is, so the text isn't
/// read again.
function Markdown({ text, onLink, className }: { text: string; onLink?: (href: string) => void; className?: string }) {
  const blocks = React.useMemo(() => parseMarkdown(text), [text])
  return (
    <div
      data-slot="markdown"
      className={cn("text-[0.9375rem] leading-[1.6] break-words [&>:first-child]:mt-0 [&>:last-child]:mb-0", inline, className)}
      onClick={(e) => {
        const a = e.target instanceof Element ? e.target.closest<HTMLAnchorElement>("a[data-local]") : null
        if (!a || !onLink || e.defaultPrevented) return
        e.preventDefault()
        onLink(a.getAttribute("href") ?? "")
      }}
    >
      <Blocks blocks={blocks} />
    </div>
  )
}

export { Markdown }
