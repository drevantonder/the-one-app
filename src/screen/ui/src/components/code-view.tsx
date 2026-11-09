import * as React from "react"
import { ArrowUDownLeftIcon, MagnifyingGlassIcon, XIcon } from "@phosphor-icons/react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { CopyButton } from "@/components/copy-button"
import { useWidth } from "@/components/layout"
import { highlight, type Token } from "@/lib/highlight"
import { cn } from "@/lib/utils"

const fileSize = (n: number) => (n < 1024 ? `${n} Bytes` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`)

/// A line's tokens, coloured; `marks` are character ranges of the line's text to mark, as find has them.
function Pieces({ tokens, marks }: { tokens: readonly Token[]; marks?: readonly (readonly [number, number])[] }) {
  if (!tokens.length) return " "
  const piece = (kind: string, text: string, key: string) =>
    kind ? <span key={key} className={`hl-${kind}`}>{text}</span> : <React.Fragment key={key}>{text}</React.Fragment>
  if (!marks?.length) return tokens.map(([kind, text], i) => piece(kind, text, String(i)))
  const out: React.ReactNode[] = []
  let at = 0
  let m = 0
  tokens.forEach(([kind, text], i) => {
    const end = at + text.length
    let from = 0
    while (m < marks.length && marks[m][0] < end) {
      const [s, e] = marks[m]
      const a = Math.max(s, at) - at
      const b = Math.min(e, end) - at
      if (a > from) out.push(piece(kind, text.slice(from, a), `${i}.${from}`))
      out.push(<mark key={`${i}.${a}m`} className="rounded-sm bg-(--find) text-inherit">{piece(kind, text.slice(a, b), "m")}</mark>)
      from = b
      if (e <= end) m++
      else break
    }
    if (from < text.length) out.push(piece(kind, text.slice(from), `${i}.${from}`))
    at = end
  })
  return out
}

/// A line of the file. `current` is the match find is on; it's drawn again only when its own props change.
function Line({ n, tokens, marks, current, wrap }: {
  n: number
  tokens: readonly Token[]
  marks?: readonly (readonly [number, number])[]
  current: boolean
  wrap: boolean
}) {
  return (
    <tr data-line={n} data-current={current || undefined} className="data-[current]:bg-(--find)/20">
      <td
        data-n={n}
        className="w-px min-w-[3.2em] py-0 pr-3 pl-3 text-right align-top text-muted-foreground/70 tabular-nums select-none before:content-[attr(data-n)]"
      />
      <td className={cn("py-0 pr-4 align-top", wrap ? "whitespace-pre-wrap [overflow-wrap:anywhere]" : "whitespace-pre")}>
        <Pieces tokens={tokens} marks={marks} />
      </td>
    </tr>
  )
}

/// A file's code, read only: line numbers, colour for its language, Find (⌘F or Ctrl+F; Enter and ⇧Enter step through
/// the lines that match, case doesn't matter), Wrap (on to start with where there's little room) and Copy. It sizes
/// itself by the room it's in, and its text scrolls sideways when it doesn't wrap.
///
/// `path` is the file's path (or a language's name) to colour by; with neither the text stays plain. `header` is
/// the line count and size above the tools, on by default. `wrap` starts Wrap on or off, where it would start by room.
function CodeView({ text, path = "", wrap, header = true, className }: {
  text: string
  path?: string
  wrap?: boolean
  header?: boolean
  className?: string
}) {
  const [frame, width] = useWidth<HTMLDivElement>()
  const body = React.useRef<HTMLDivElement>(null)
  const find = React.useRef<HTMLInputElement>(null)

  const source = String(text ?? "").replace(/\n$/, "")
  const lines = React.useMemo(() => highlight(source, path), [source, path])
  const plain = React.useMemo(() => lines.map((l) => l.map((t) => t[1]).join("")), [lines])
  const bytes = React.useMemo(() => new TextEncoder().encode(String(text ?? "")).length, [text])

  const [asked, setWrap] = React.useState<boolean | null>(null)
  const wrapped = asked ?? wrap ?? (width > 0 && width < 600)

  // What's typed, and what's searched for a moment after: search runs as a user pauses.
  const [typed, setTyped] = React.useState("")
  const [query, setQuery] = React.useState("")
  React.useEffect(() => {
    const t = setTimeout(() => setQuery(typed), 150)
    return () => clearTimeout(t)
  }, [typed])

  // The lines that match, and each one's matches as ranges of its text.
  const hits = React.useMemo(() => {
    const needle = query.toLowerCase()
    const found = new Map<number, [number, number][]>()
    if (!needle) return found
    plain.forEach((line, n) => {
      const low = line.toLowerCase()
      for (let k = low.indexOf(needle); k >= 0; k = low.indexOf(needle, k + needle.length)) {
        if (!found.has(n)) found.set(n, [])
        found.get(n)!.push([k, k + needle.length])
      }
    })
    return found
  }, [plain, query])
  const order = React.useMemo(() => [...hits.keys()], [hits])

  // Which match it's on: the first, whenever the search changes.
  const [on, setOn] = React.useState({ query: "", at: 0 })
  const at = on.query === query ? on.at : 0
  const step = (d: number) => order.length && setOn({ query, at: (at + d + order.length) % order.length })
  React.useEffect(() => {
    if (!query || !order.length) return
    body.current?.querySelector(`[data-line="${order[at]}"]`)?.scrollIntoView({ block: "center" })
  }, [query, at, order])

  // ⌘F or Ctrl+F searches in the file, where the file is on the screen.
  React.useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f" && find.current?.isConnected) {
        e.preventDefault()
        find.current.focus()
        find.current.select()
      }
    }
    document.addEventListener("keydown", key)
    return () => document.removeEventListener("keydown", key)
  }, [])

  const count = !query ? "" : order.length ? `${at + 1} of ${order.length}` : "0"
  const current = order[at]
  return (
    <div
      ref={frame}
      data-slot="code-view"
      className={cn("@container w-full min-w-0 overflow-hidden rounded-xl border bg-card text-card-foreground", className)}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b py-1.5 pr-1.5 pl-3 text-sm text-muted-foreground">
        {header && (
          <span className="hidden min-h-8 flex-1 items-center font-mono text-xs whitespace-nowrap @md:flex">
            {lines.length.toLocaleString()} {lines.length === 1 ? "line" : "lines"} · {fileSize(bytes)}
          </span>
        )}
        <div className={cn("flex min-w-0 flex-1 items-center gap-1", !header && "@md:flex-none")}>
          <div className="relative min-w-0 flex-1 @md:w-56 @md:flex-none">
            <MagnifyingGlassIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
            <Input
              ref={find}
              type="text"
              value={typed}
              placeholder={width && width < 448 && header ? `Find in ${lines.length.toLocaleString()} ${lines.length === 1 ? "line" : "lines"}` : "Find in file"}
              aria-label="Find in file"
              enterKeyHint="search"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              className={cn("h-8 pl-8 text-sm", typed ? "pr-20" : "pr-3")}
              onChange={(e) => setTyped(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return
                e.preventDefault()
                // Enter on a new search runs it at once, without waiting out the pause.
                if (typed !== query) setQuery(typed)
                else step(e.shiftKey ? -1 : 1)
              }}
            />
            {typed && (
              <div className="absolute top-1/2 right-1 flex -translate-y-1/2 items-center gap-0.5">
                <span aria-live="polite" className="text-xs tabular-nums">{count}</span>
                <Button type="button" variant="ghost" size="icon-xs" aria-label="Clear" onClick={() => { setTyped(""); setQuery(""); find.current?.focus() }}>
                  <XIcon />
                </Button>
              </div>
            )}
          </div>
          <Button type="button" variant="ghost" size="sm" aria-pressed={wrapped} className="aria-pressed:bg-accent aria-pressed:text-accent-foreground" aria-label="Wrap lines" onClick={() => setWrap(!wrapped)}>
            <ArrowUDownLeftIcon />
            <span className="hidden @md:inline">Wrap</span>
          </Button>
          <CopyButton text={String(text ?? "")} labelClassName="hidden @md:inline" />
        </div>
      </div>
      <div ref={body} className="overflow-x-auto py-2">
        <table className="w-full border-collapse font-mono text-[12.5px] leading-[1.6] [tab-size:4]">
          <tbody>
            {lines.map((tokens, n) => (
              <Line key={n} n={n + 1} tokens={tokens} marks={hits.get(n)} current={n === current && !!query} wrap={wrapped} />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

/// One hunk of a file's diff, as a commit's diff has them: where it starts in the old file (`a`) and the new (`b`), and
/// its lines, each starting with " " (as it was and is), "-" (gone) or "+" (new).
type Hunk = { a: number; b: number; lines: string[] }

type Cell = { n: number; text: string; kind: "" | "add" | "del" } | null

// The tints: a changed line's, and its number's a step stronger (ui.css has the colours, light and dark).
const tint = { "": "", add: "bg-(--diff-add)", del: "bg-(--diff-del)" } as const
const tintNumber = { "": "", add: "bg-(--diff-add-number)", del: "bg-(--diff-del-number)" } as const

function Code({ text, path }: { text: string; path: string }) {
  const tokens = React.useMemo(() => highlight(text, path)[0] ?? [], [text, path])
  return <Pieces tokens={tokens} />
}

function LineNumber({ n, kind = "" }: { n?: number; kind?: "" | "add" | "del" }) {
  return (
    <td
      data-n={n}
      className={cn("w-px min-w-[3em] px-2 py-0 text-right align-top text-muted-foreground/70 tabular-nums select-none before:content-[attr(data-n)]", tintNumber[kind])}
    />
  )
}

/// A file's changes, as hunks: the line numbers of both sides, what went and what came in tinted, each line coloured for
/// the file's language (`path`), and each hunk under its `@@` line. One column of lines where the room is narrow, and
/// the old file beside the new where it's wide (`split` says which, instead of the room). Long lines wrap. It has no
/// frame of its own: put it in a card, with the file's name above it.
function DiffView({ hunks, path = "", split, className }: {
  hunks: Hunk[]
  path?: string
  split?: boolean
  className?: string
}) {
  const [frame, width] = useWidth<HTMLDivElement>()
  const beside = split ?? width >= 720
  const rows = hunks.map((h) => {
    let a = h.a
    let b = h.b
    // Unified: each line once, with its number on each side it has one.
    const unified = h.lines.map((l) => {
      const kind = l[0] === "+" ? "add" : l[0] === "-" ? "del" : ""
      return { kind, sign: l[0] === " " ? "" : l[0], text: l.slice(1), a: kind === "add" ? undefined : a++, b: kind === "del" ? undefined : b++ } as const
    })
    // Split: a run of removed lines beside the run of added ones that follows it.
    const pairs: [Cell, Cell][] = []
    a = h.a
    b = h.b
    for (let i = 0; i < h.lines.length; ) {
      if (h.lines[i][0] === " ") {
        const text = h.lines[i++].slice(1)
        pairs.push([{ n: a++, text, kind: "" }, { n: b++, text, kind: "" }])
        continue
      }
      const del: string[] = []
      const add: string[] = []
      while (i < h.lines.length && h.lines[i][0] === "-") del.push(h.lines[i++].slice(1))
      while (i < h.lines.length && h.lines[i][0] === "+") add.push(h.lines[i++].slice(1))
      for (let k = 0; k < Math.max(del.length, add.length); k++) {
        pairs.push([
          k < del.length ? { n: a++, text: del[k], kind: "del" } : null,
          k < add.length ? { n: b++, text: add[k], kind: "add" } : null,
        ])
      }
    }
    return { h, unified, pairs }
  })
  const code = "py-0 pr-3 pl-1 align-top whitespace-pre-wrap [overflow-wrap:anywhere]"
  const side = (cell: Cell) =>
    cell ? (
      <>
        <LineNumber n={cell.n} kind={cell.kind} />
        <td className={cn(code, "w-1/2", tint[cell.kind])}><Code text={cell.text} path={path} /></td>
      </>
    ) : (
      <>
        <td className="w-px min-w-[3em] bg-muted/50" />
        <td className={cn(code, "w-1/2 bg-muted/50")} />
      </>
    )
  return (
    <div ref={frame} data-slot="diff-view" className={cn("w-full min-w-0 overflow-x-auto", className)}>
      <table className="w-full border-collapse font-mono text-xs leading-[1.55] [tab-size:4]">
        <tbody>
          {rows.map(({ h, unified, pairs }, k) => (
            <React.Fragment key={k}>
              <tr>
                <td colSpan={beside ? 4 : 3} className="bg-muted px-3 py-0.5 text-muted-foreground">@@ −{h.a} +{h.b} @@</td>
              </tr>
              {beside
                ? pairs.map(([old, now], i) => <tr key={i}>{side(old)}{side(now)}</tr>)
                : unified.map((l, i) => (
                  <tr key={i}>
                    <LineNumber n={l.a} kind={l.kind} />
                    <LineNumber n={l.b} kind={l.kind} />
                    <td className={cn(code, tint[l.kind])}>
                      <span className="inline-block w-[1.2em] text-muted-foreground select-none">{l.sign}</span>
                      <Code text={l.text} path={path} />
                    </td>
                  </tr>
                ))}
            </React.Fragment>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export { CodeView, DiffView, type Hunk }
