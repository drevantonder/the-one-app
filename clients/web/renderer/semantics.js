// The page's semantic tree (the renderer's `Screen::semantics`, what VoiceOver and TalkBack read natively) as an ARIA
// tree a browser's screen reader reads, as Flutter web's semantics are (flutter/engine, lib/web_ui/lib/src/engine/
// semantics): an element for each node, kept by its id and changed in place as the tree changes, where it's drawn and
// see-through, so a pointer reaches the canvas under it. What a screen reader does to one is the renderer's to do
// (`act`): a press, the keyboard given to it, or moving to it, which shows it.
//
// Moving to a node off the canvas, a screen reader has the browser scroll it into sight (Chrome's
// `ScrollToMakeVisible`, centred if it's out, or `focus()`'s own scroll), as it does in a page of its own. The tree is
// drawn in a box of its own that only scrolls that way, so the page and its canvas stay where they are, `AWAY` in from
// its start, so a node above or before the canvas can be scrolled to too, as Flutter web's scrollables keep a neutral
// scroll the browser moves from. Where it's scrolled to says which node it showed, which the renderer reveals
// (`Screen::reveal`, as natively for AccessKit's `ScrollIntoView`), and the box goes back.

/** How far in from its box's start the tree is drawn, in CSS pixels, which as far before the canvas can be scrolled to. */
const AWAY = 1_000_000

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
  /** Where each node is drawn, from the canvas's top left. @type {Map<number, number[]>} */
  const rects = new Map()
  // The tree's plane, as large as the box it's in and `AWAY` more each way, which that box is scrolled `AWAY` into.
  const plane = document.createElement("div")
  plane.style.cssText = `position:absolute;left:0;top:0;width:calc(100% + ${2 * AWAY}px);height:calc(100% + ${2 * AWAY}px)`
  root.append(plane)
  const home = () => root.scrollLeft === AWAY && root.scrollTop === AWAY || root.scrollTo(AWAY, AWAY)
  home()
  root.addEventListener("scroll", () => {
    const by = [root.scrollLeft - AWAY, root.scrollTop - AWAY]
    if (!by[0] && !by[1]) return
    home()
    const shown = showed(by, [root.clientWidth, root.clientHeight])
    if (shown !== undefined) act(shown, "reveal")
  })

  /**
   * The node the browser scrolled the tree `by` to show, in a view `size`: the one with the keyboard, if it's what was
   * out of sight, or else the one whose box it scrolled just into sight or into the middle, as a browser shows one,
   * the largest of those, as a control is what a screen reader stops on, not the words inside it.
   * @param {number[]} by @param {number[]} size
   */
  const showed = (by, size) => {
    const out = (/** @type {number[]} */ rect) => [0, 1].some((axis) => rect[axis] < 0 || rect[axis] + rect[axis + 2] > size[axis])
    // How far `by` is, on each axis, from where the browser would have scrolled to show `rect`: not at all where it
    // shows already, else into the middle or just in.
    const off = (/** @type {number[]} */ rect) =>
      [0, 1].reduce((sum, axis) => {
        const [start, length, view] = [rect[axis], rect[axis + 2], size[axis]]
        const shows = start >= 0 && start + length <= view
        return sum + (shows ? Math.abs(by[axis]) : Math.min(Math.abs(by[axis] - (start + length / 2 - view / 2)), Math.abs(by[axis] - (start < 0 ? start : start + length - view))))
      }, 0)
    const focused = [...elements].find(([, element]) => element === document.activeElement)?.[0]
    if (focused !== undefined && out(rects.get(focused) ?? [0, 0, 0, 0])) return focused
    let best
    for (const [id, rect] of rects) {
      if (!out(rect)) continue
      const score = [off(rect), rect[2] * rect[3]]
      if (!best || score[0] < best[1][0] - 0.5 || (Math.abs(score[0] - best[1][0]) <= 0.5 && score[1] > best[1][1])) best = [id, score]
    }
    return best?.[0]
  }
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
    node.rect ? rects.set(node.id, node.rect) : rects.delete(node.id)
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
    const top = (tree ?? []).map((node) => element(node, [-AWAY, -AWAY]))
    top.forEach((child, at) => plane.children[at] !== child && plane.insertBefore(child, plane.children[at] ?? null))
    while (plane.children.length > top.length) /** @type {Element} */ (plane.lastElementChild).remove()
    for (const id of elements.keys()) if (!shown.has(id)) (elements.delete(id), rects.delete(id))
    home()
  }
}
