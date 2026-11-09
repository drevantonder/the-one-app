// Small parts of Repos' pages: GitHub's mark, a user, a label as GitHub draws it, an issue's state, +/− counts, and
// a Markdown field to write in.
import { useState } from "react"
import { cn, CopyButton, Hint, Icon, Markdown, Tabs, TabsList, TabsTrigger, Textarea } from "@one/ui"
import { isGithub } from "./use.jsx"

export function GitHubMark({ className }) {
  return (
    <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" className={cn("size-4 shrink-0", className)}>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  )
}

export const RepoIcon = ({ repo, github = isGithub(repo), className }) =>
  github ? <GitHubMark className={className} /> : <Icon name="book" className={className} />

export function LabelChip({ l, className }) {
  const c = /^[0-9a-f]{6}$/i.test(l.color) ? l.color : "ededed"
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(c.slice(i, i + 2), 16))
  return (
    <span
      className={cn("inline-block max-w-full shrink-0 truncate rounded-full px-2 py-px align-middle text-xs font-medium ring-1 ring-black/10 ring-inset", className)}
      style={{ background: `#${c}`, color: 0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#1f2328" : "#fff" }}
    >
      {l.name}
    </span>
  )
}

export const StateIcon = ({ issue, className }) => (
  <Icon
    name={issue.state === "open" ? "record" : issue.state_reason === "not_planned" ? "prohibit" : "check-circle"}
    className={cn("size-4", issue.state !== "open" && "text-muted-foreground", className)}
  />
)

export function Counts({ add, del, className }) {
  return (
    <span className={cn("whitespace-nowrap tabular-nums", className)}>
      <span className="text-(--hl-ins)">+{add.toLocaleString()}</span> <span className="text-(--hl-del)">−{del.toLocaleString()}</span>
    </span>
  )
}

export function MarkdownField({ value, onChange, placeholder = "", rows = 6, autoFocus, label }) {
  const [preview, setPreview] = useState(false)
  return (
    <div className="flex flex-col gap-2">
      <Tabs value={preview ? "preview" : "write"} onValueChange={(v) => setPreview(v === "preview")}>
        <TabsList>
          <TabsTrigger value="write">Write</TabsTrigger>
          <TabsTrigger value="preview">Preview</TabsTrigger>
        </TabsList>
      </Tabs>
      {preview ? (
        value.trim() ? <Markdown text={value} /> : <Hint>Nothing to preview.</Hint>
      ) : (
        <Textarea value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} style={{ minHeight: `calc(${rows}lh + 1.125rem)` }} className="resize-y" autoFocus={autoFocus} aria-label={label || placeholder || "Text"} />
      )}
    </div>
  )
}

export const SmallCopy = ({ text, className }) => (
  <CopyButton text={text} label="" size="sm" className={cn("size-6 [&_svg:not([class*='size-'])]:size-3", className)} />
)
