// A page's semantic tree as text, the same from the DOM and from what a native host draws (record.js), so the two can
// be compared: each node's role and name, its states, and its words, as a screen reader and an agent read them. It's
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

/** An element's implicit role, as HTML-AAM has it, for the elements the UI library draws with. */
function implicit(element: Element): string | undefined {
  const tag = element.localName
  if (/^h[1-6]$/.test(tag)) return "heading"
  if (tag === "a" && element.hasAttribute("href")) return "link"
  if (tag === "img") return (element as HTMLImageElement).alt ? "img" : undefined
  if (tag === "input") {
    const type = (element as HTMLInputElement).type
    return type === "checkbox" ? "checkbox" : type === "radio" ? "radio" : type === "search" ? "searchbox" : "textbox"
  }
  if (tag === "header") return element.parentElement?.closest("article, aside, main, nav, section") ? undefined : "banner"
  return ({ button: "button", textarea: "textbox", ul: "list", ol: "list", li: "listitem", main: "main", nav: "navigation", hr: "separator", dialog: "dialog", summary: "button" } as Record<string, string>)[tag]
}

/** The semantic tree of what's drawn in `element`, as the DOM has it. */
export function fromDom(element: Element): Semantic {
  const children: Semantic[] = []
  for (const child of element.childNodes) {
    if (child.nodeType === 3) children.push({ text: child.textContent ?? "", states: {}, children: [] })
    else if (child.nodeType === 1) {
      const inside = child as Element
      const style = getComputedStyle(inside)
      if ((inside as HTMLElement).hidden || (inside as HTMLElement).inert || inside.getAttribute("aria-hidden") === "true" || style.display === "none" || style.visibility === "hidden" || inside.localName === "svg") continue
      children.push(fromDom(inside))
    }
  }
  const role = element.getAttribute("role") ?? implicit(element)
  const found: Record<string, string> = {}
  for (const state of states) {
    const value = element.getAttribute(`aria-${state}`)
    if (value !== null) found[state] = value
  }
  if (role === "heading") found.level ??= element.localName.slice(1)
  if ((element as HTMLButtonElement).disabled) found.disabled = "true"
  const name = element.getAttribute("aria-label") ?? (element.localName === "img" ? (element as HTMLImageElement).alt : undefined)
  return { role: role === "presentation" || role === "none" ? undefined : role, name, states: found, children }
}
