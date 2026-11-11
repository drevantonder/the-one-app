// Code as tokens, one list per line, coloured the way GitHub colours it: comments, strings, numbers, keywords, types,
// functions and tags, per language. Small on purpose; a file in a language it doesn't know stays plain.

/// A piece of a line: its kind ("kw", "str", "com", "num", "lit", "prop", "attr", "var", "fn", "type", "tag", "ins",
/// "del", "strong"; "" for plain) and its text, which CodeView draws as `hl-<kind>`.
export type Token = readonly [kind: string, text: string]

type Language = {
  line?: string
  block?: [string, string]
  quotes?: string
  triple?: boolean
  kw?: Set<string>
  lit?: Set<string>
  macros?: boolean
  attrs?: string
  vars?: boolean
  pre?: boolean
  keys?: boolean
  nocase?: boolean
  plainTypes?: boolean
  css?: boolean
  markup?: boolean
  md?: boolean
  diff?: boolean
}

const words = (s: string) => new Set(s.split(" "))
const C = { line: "//", block: ["/*", "*/"] as [string, string], quotes: `"'` }

const LANGS: Record<string, Language> = {
  js: { ...C, quotes: "\"'`", kw: words("async await break case catch class const continue debugger default delete do else export extends finally for from function if import in instanceof let new of return static super switch this throw try typeof var void while with yield as satisfies get set"), lit: words("true false null undefined NaN Infinity") },
  ts: { ...C, quotes: "\"'`", kw: words("abstract any as async await boolean break case catch class const constructor continue declare default delete do else enum export extends finally for from function if implements import in infer instanceof interface is keyof let module namespace never new number object of private protected public readonly return satisfies static string super switch symbol this throw try type typeof unknown var void while yield"), lit: words("true false null undefined") },
  rs: { ...C, quotes: '"', kw: words("as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while"), lit: words("true false None Some Ok Err"), macros: true, attrs: "#" },
  swift: { ...C, quotes: '"', kw: words("actor any as associatedtype async await break case catch class continue default defer deinit do else enum extension fallthrough fileprivate final for func guard if import in init inout internal is lazy let mutating nonisolated open operator override private protocol public repeat rethrows return self Self some static struct subscript super switch throw throws try typealias var weak where while"), lit: words("true false nil"), attrs: "@" },
  go: { ...C, quotes: "\"'`", kw: words("break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var"), lit: words("true false nil iota") },
  c: { ...C, kw: words("auto break case char const continue default do double else enum extern float for goto if inline int long register return short signed sizeof static struct switch typedef union unsigned void volatile while class namespace template typename public private protected virtual new delete this using bool"), lit: words("true false NULL nullptr"), pre: true },
  java: { ...C, kw: words("abstract assert break case catch class const continue default do else enum extends final finally for if implements import instanceof interface native new package private protected public return static super switch synchronized this throw throws transient try void volatile while var val fun object when is in data sealed override companion suspend"), lit: words("true false null"), attrs: "@" },
  py: { line: "#", quotes: `"'`, triple: true, kw: words("and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case"), lit: words("True False None self") },
  sh: { line: "#", quotes: `"'`, plainTypes: true, kw: words("if then else elif fi for while do done case esac in function return local export set unset echo exit source"), vars: true },
  rb: { line: "#", quotes: `"'`, kw: words("begin class def do else elsif end ensure for if in module next rescue return then unless until when while yield require"), lit: words("true false nil self") },
  css: { block: ["/*", "*/"], quotes: `"'`, css: true },
  sql: { line: "--", block: ["/*", "*/"], quotes: `"'`, kw: words("select from where and or not insert into values update set delete create table index primary key on as join left right inner outer group by order having limit offset union all distinct case when then else end is null like in exists with text integer real blob default references"), nocase: true },
  toml: { line: "#", quotes: `"'`, lit: words("true false"), keys: true },
  yaml: { line: "#", quotes: `"'`, lit: words("true false null yes no"), keys: true },
  json: { quotes: '"', lit: words("true false null"), keys: true },
  html: { markup: true },
  md: { md: true },
  diff: { diff: true },
}

const EXT: Record<string, string> = {
  js: "js", mjs: "js", cjs: "js", jsx: "js", ts: "ts", tsx: "ts", mts: "ts", rs: "rs", swift: "swift", go: "go",
  c: "c", h: "c", cc: "c", cpp: "c", hpp: "c", m: "c", mm: "c", java: "java", kt: "java", kts: "java", scala: "java", cs: "java",
  py: "py", sh: "sh", bash: "sh", zsh: "sh", fish: "sh", rb: "rb", css: "css", scss: "css", less: "css", sql: "sql",
  toml: "toml", lock: "toml", yaml: "yaml", yml: "yaml", json: "json", jsonc: "json", webmanifest: "json",
  html: "html", htm: "html", xml: "html", svg: "html", plist: "html", vue: "html", md: "md", mdx: "md", markdown: "md",
  diff: "diff", patch: "diff",
  javascript: "js", typescript: "ts", rust: "rs", python: "py", shell: "sh", console: "sh", ruby: "rb", golang: "go", kotlin: "java", objc: "c",
}
const NAMES: Record<string, string> = { Dockerfile: "sh", Makefile: "sh", Justfile: "sh", ".gitignore": "sh", ".env": "sh", "Cargo.lock": "toml" }

function languageOf(path = ""): string | null {
  const name = String(path).split("/").pop() ?? ""
  return NAMES[name] || EXT[name.toLowerCase()] || EXT[(name.split(".").pop() ?? "").toLowerCase()] || null
}

function tokens(src: string, L: Language): Token[] {
  const out: Token[] = []
  let i = 0
  let plain = ""
  const flush = () => {
    if (plain) out.push(["", plain])
    plain = ""
  }
  const push = (kind: string, text: string) => {
    flush()
    out.push([kind, text])
    i += text.length
  }
  const rest = () => src.slice(i)
  while (i < src.length) {
    const ch = src[i]
    let m: RegExpExecArray | null
    if (L.line && src.startsWith(L.line, i)) {
      const end = src.indexOf("\n", i)
      push("com", src.slice(i, end < 0 ? src.length : end))
      continue
    }
    if (L.block && src.startsWith(L.block[0], i)) {
      const end = src.indexOf(L.block[1], i + 2)
      push("com", src.slice(i, end < 0 ? src.length : end + L.block[1].length))
      continue
    }
    if (L.triple && (src.startsWith('"""', i) || src.startsWith("'''", i))) {
      const q = src.substr(i, 3)
      const end = src.indexOf(q, i + 3)
      push("str", src.slice(i, end < 0 ? src.length : end + 3))
      continue
    }
    if (L.quotes?.includes(ch) && !(L.macros && ch === "'" && /^'[a-z_]\w*(?!')/i.test(rest()) && !/^'.'/.test(rest()))) {
      let j = i + 1
      while (j < src.length && src[j] !== ch && !(src[j] === "\n" && ch !== "`")) j += src[j] === "\\" ? 2 : 1
      const s = src.slice(i, j + 1)
      push(L.keys && /^\s*:/.test(src.slice(j + 1, j + 20)) ? "prop" : "str", s)
      continue
    }
    if (L.attrs && ch === L.attrs && (m = /^[#@]!?\[?[\w:]+/.exec(rest()))) {
      push("attr", m[0])
      continue
    }
    if (L.vars && ch === "$" && (m = /^\$(\{[^}]*\}|\w+|.)/.exec(rest()))) {
      push("var", m[0])
      continue
    }
    if (L.pre && ch === "#" && (m = /^#\s*\w+/.exec(rest()))) {
      push("kw", m[0])
      continue
    }
    if ((m = /^(0x[\da-f_]+|\d[\d_]*(\.\d[\d_]*)?(e[+-]?\d+)?)[a-z%]*/i.exec(rest())) && !/\w/.test(src[i - 1] || "")) {
      push("num", m[0])
      continue
    }
    if ((m = /^[A-Za-z_$][\w$]*/.exec(rest())) && !/[\w$]/.test(src[i - 1] || "")) {
      const w = m[0]
      const k = L.nocase ? w.toLowerCase() : w
      const after = src.slice(i + w.length, i + w.length + 2)
      if (
        L.keys &&
        /^(\s*[=:])/.test(src.slice(i + w.length, i + w.length + 40).replace(/^[\w.-]*/, "")) &&
        /^\s*$/.test(src.slice(src.lastIndexOf("\n", i - 1) + 1, i).replace(/[-\s[]/g, ""))
      ) {
        push("prop", w)
        continue
      }
      if (L.kw?.has(k)) push("kw", w)
      else if (L.lit?.has(w)) push("lit", w)
      else if (L.macros && after[0] === "!" && after[1] !== "=") push("fn", w + "!")
      else if (after[0] === "(") push("fn", w)
      else if (/^[A-Z][a-z]/.test(w) && L.kw && !L.plainTypes) push("type", w)
      else {
        plain += w
        i += w.length
      }
      continue
    }
    plain += ch
    i++
  }
  flush()
  return out
}

function markup(src: string): Token[] {
  const out: Token[] = []
  const re = /(<!--[\s\S]*?(?:-->|$))|(<\/?)([\w:.-]+)((?:\s+[^\s=>/]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)(\s*\/?>)?|([^<]+|<)/g
  let m: RegExpExecArray | null
  while ((m = re.exec(src)) && m[0]) {
    if (m[1]) out.push(["com", m[1]])
    else if (m[3]) {
      out.push(["", m[2]], ["tag", m[3]])
      const attrs = /(\s+)([^\s=>/]+)(?:(\s*=\s*)("[^"]*"|'[^']*'|[^\s>]+))?/g
      let a: RegExpExecArray | null
      while ((a = attrs.exec(m[4] || ""))) {
        out.push(["", a[1]], ["attr", a[2]])
        if (a[3]) out.push(["", a[3]], ["str", a[4]])
      }
      if (m[5]) out.push(["", m[5]])
    } else out.push(["", m[6]])
  }
  return out
}

function cssTokens(src: string): Token[] {
  return tokens(src, { block: ["/*", "*/"], quotes: `"'` }).flatMap(([kind, text]): Token[] => {
    if (kind) return [[kind, text]]
    const out: Token[] = []
    const re = /(@[\w-]+)|(#[\da-f]{3,8}\b)|(-?\d*\.?\d+(?:px|em|rem|%|vh|vw|s|ms|deg|fr)?\b)|([\w-]+)(?=\s*:[^;{]*[;}\n])|([^@#\d\w-]+|[\w-]+|.)/gi
    let m: RegExpExecArray | null
    while ((m = re.exec(text)) && m[0]) out.push([m[1] ? "kw" : m[2] || m[3] ? "num" : m[4] ? "prop" : "", m[0]])
    return out
  })
}

const mdInline = (s: string): Token[] =>
  s
    .split(/(`[^`]*`|\*\*[^*]+\*\*|\[[^\]]*\]\([^)]*\))/)
    .filter(Boolean)
    .map((p): Token => [/^`/.test(p) ? "str" : /^\*\*/.test(p) ? "strong" : /^\[/.test(p) ? "fn" : "", p])

function lineTokens(src: string, kind: "md" | "diff"): Token[] {
  return src.split("\n").flatMap((l, n): Token[] => {
    const nl: Token[] = n ? [["", "\n"]] : []
    if (kind === "diff") return [...nl, [/^\+/.test(l) ? "ins" : /^-/.test(l) ? "del" : /^@@/.test(l) ? "fn" : "", l]]
    if (/^#{1,6}\s/.test(l)) return [...nl, ["kw", l]]
    if (/^\s*([-*+]|\d+\.)\s/.test(l)) {
      const m = /^\s*([-*+]|\d+\.)/.exec(l)!
      return [...nl, ["lit", m[0]], ...mdInline(l.slice(m[0].length))]
    }
    if (/^\s*>/.test(l)) return [...nl, ["com", l]]
    return [...nl, ...mdInline(l)]
  })
}

/// `text` as one list of tokens per line, in the language `pathOrLanguage` names (a file's path, its extension, or a
/// language's name). Fenced code inside Markdown stays plain: telling where fences start isn't worth the code.
export function highlight(text: string, pathOrLanguage = ""): Token[][] {
  const key = LANGS[pathOrLanguage] ? pathOrLanguage : languageOf(pathOrLanguage)
  const L = key ? LANGS[key] : undefined
  const src = String(text ?? "")
  if (!L || src.length > 1e6) return src.split("\n").map((l) => (l ? [["", l] as Token] : []))
  const toks = L.markup ? markup(src) : L.css ? cssTokens(src) : L.md ? lineTokens(src, "md") : L.diff ? lineTokens(src, "diff") : tokens(src, L)
  const lines: Token[][] = [[]]
  for (const [kind, t] of toks) {
    t.split("\n").forEach((part, k) => {
      if (k) lines.push([])
      if (part) lines[lines.length - 1].push([kind, part])
    })
  }
  return lines
}
