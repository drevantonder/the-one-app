// A page's semantic tree as text, from what a host draws (record.js), so one drawing can be compared with another:
// each node's role and name, its states, and its words, as a screen reader and an agent read them. It's
// Playwright's ARIA snapshot's shape: a node with no role of its own is left out, with what's in it in its place.

/** A node of the semantic tree. */
export interface Semantic {
  role?: string
  name?: string
  /** Words, for a run of text. */
  text?: string
  states: Record<string, string>
  children: Semantic[]
}

/** The roles named by what's in them, as ARIA has it. */
const fromContent = new Set(["button", "link", "heading", "menuitem", "tab", "option", "checkbox", "radio", "switch"])
/** The states written out, and the ones written only when they're so. */
const states = ["level", "expanded", "checked", "disabled", "selected", "busy", "current"]
const whenSo = new Set(["disabled", "selected", "busy", "current"])

/** What's in a node, as ARIA names something from its content: words, and what's in it by its own name. */
const words = (node: Semantic): string => node.text ?? node.name ?? node.children.map(words).filter(Boolean).join(" ")
const tidy = (text: string) => text.replace(/\s+/g, " ").trim()

/** The tree as lines, `- role "name" [state=value]`, what's in each indented under it. */
export function outline(node: Semantic): string {
  return lines(node.children, "").join("\n")
}

function lines(nodes: Semantic[], indent: string): string[] {
  // What has no role of its own is what's in it; words side by side are one run.
  const shown: Semantic[] = []
  for (const node of nodes.flatMap(function open(node): Semantic[] { return node.role || node.text !== undefined ? [node] : node.children.flatMap(open) })) {
    const before = shown.at(-1)
    if (node.text !== undefined && before?.text !== undefined) before.text = `${before.text} ${node.text}`
    else shown.push(node.text !== undefined ? { ...node } : node)
  }
  return shown.flatMap((node) => {
    if (node.text !== undefined) return tidy(node.text) ? [`${indent}- text: ${tidy(node.text)}`] : []
    const name = tidy(node.name ?? (fromContent.has(node.role!) ? words(node) : ""))
    const said = states.filter((state) => node.states[state] !== undefined && (!whenSo.has(state) || node.states[state] === "true")).map((state) => `${state}=${node.states[state]}`)
    const head = `${indent}- ${node.role}${name ? ` ${JSON.stringify(name)}` : ""}${said.length ? ` [${said.join(", ")}]` : ""}`
    const inside = fromContent.has(node.role!) ? [] : lines(node.children, `${indent}  `)
    return [inside.length ? `${head}:` : head, ...inside]
  })
}
