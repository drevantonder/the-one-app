// The page's semantic tree (the renderer's `Screen::semantics`, what VoiceOver and TalkBack read natively) as an ARIA
// tree a browser's screen reader reads, as Flutter web's semantics are (flutter/engine, lib/web_ui/lib/src/engine/
// semantics): an element for each node, kept by its id and changed in place as the tree changes, where it's drawn and
// see-through, so a pointer reaches the canvas under it. What a screen reader does to one is the renderer's to do
// (`act`): a press, the keyboard given to it, or moving to it, which shows it.

/** The states a node has, as ARIA names them. */
const STATES = ["selected", "checked", "disabled", "expanded", "pressed", "required", "invalid", "current", "busy", "readonly", "multiline"]

/**
 * Keeps an ARIA tree in `root` as each semantic tree it's given (`[{id, role, name, level, value, states, actions,
 * rect: [x, y, width, height], children}]`, in CSS pixels from the canvas's top left).
 * @param {HTMLElement} root @param {(id: number, action: string) => void} act
 */
export function semantics(root, act) {
  /** @type {Map<number, HTMLElement>} */
  const elements = new Map()
  /** @type {Set<number>} */
  let shown = new Set()

  /** @param {any} node @param {number[]} within */
  const element = (node, within) => {
    let made = elements.get(node.id)
    const tag = node.role === "text" ? "span" : "div"
    if (!made || made.localName !== tag) {
      made?.remove()
      made = document.createElement(tag)
      made.style.cssText = "position:absolute;margin:0;padding:0;overflow:visible;white-space:pre"
      const id = node.id
      made.addEventListener("click", (event) => (event.preventDefault(), act(id, "press")))
      made.addEventListener("focus", () => act(id, "focus"))
      elements.set(id, made)
    }
    const [x, y, width, height] = node.rect ?? [within[0], within[1], 0, 0]
    Object.assign(made.style, { left: `${x - within[0]}px`, top: `${y - within[1]}px`, width: `${width}px`, height: `${height}px` })
    if (node.role === "text") {
      if (made.textContent !== (node.name ?? "")) made.textContent = node.name ?? ""
    } else {
      made.setAttribute("role", node.role ?? "group")
      node.name ? made.setAttribute("aria-label", node.name) : made.removeAttribute("aria-label")
      node.level ? made.setAttribute("aria-level", String(node.level)) : made.removeAttribute("aria-level")
      node.value != null ? made.setAttribute("aria-valuetext", String(node.value)) : made.removeAttribute("aria-valuetext")
      for (const state of STATES) node.states?.[state] != null ? made.setAttribute(`aria-${state}`, String(node.states[state])) : made.removeAttribute(`aria-${state}`)
      // What takes the keyboard is focusable, so a screen reader can move to it and give it the keyboard.
      node.actions?.includes("focus") ? (made.tabIndex = -1) : made.removeAttribute("tabindex")
    }
    const children = (node.children ?? []).map((/** @type {any} */ child) => element(child, [x, y]))
    // In order, moved only where they're out of it, so what a screen reader is on stays where it is.
    children.forEach((child, at) => made.children[at] !== child && made.insertBefore(child, made.children[at] ?? null))
    while (made.children.length > children.length) /** @type {Element} */ (made.lastElementChild).remove()
    shown.add(node.id)
    return made
  }

  return (/** @type {any[]} */ tree) => {
    shown = new Set()
    const top = (tree ?? []).map((node) => element(node, [0, 0]))
    top.forEach((child, at) => root.children[at] !== child && root.insertBefore(child, root.children[at] ?? null))
    while (root.children.length > top.length) /** @type {Element} */ (root.lastElementChild).remove()
    for (const id of elements.keys()) if (!shown.has(id)) elements.delete(id)
  }
}
