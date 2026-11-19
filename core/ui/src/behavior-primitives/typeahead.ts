// Typeahead: letters typed on a list move to the next item that starts with them, as menus, listboxes and selects do.
// Copied from Vercel's UI SDK (Apache-2.0) at ab55044caee66cc8228fa7ed3525088a1b7b5af7:
// packages/core/src/foundation/text/typeahead.ts (`createTypeahead`, `isTypeaheadKey`), with how text is folded for
// matching from foundation/text/match.ts (`createFolder`) and the user's language from foundation/text/locale.ts
// (`toLocale`, `userLocale`). A label is what its item says, which the component knows, not text read from the DOM.
import type { Keyed } from "../types/ui.d.ts"

// Letters NFD can't take apart, like ø or ł, are tried against these.
const latinLetters = "abcdefghijklmnopqrstuvwxyz"
const diacritics = /\p{Diacritic}/gu
const latin = /\p{Script=Latin}/u
const ascii = /^[\u0000-\u007f]*$/
// Locales whose lowercase differs from the default, like Turkish dotless i.
const specialCase = /^(tr|az|lt)\b/i

/** A language tag Intl accepts, read from a BCP 47 or POSIX one (`en_US`), or undefined. */
function toLocale(tag: string | null | undefined) {
  // A POSIX locale's encoding and modifier go, as in `de_DE.UTF-8@euro`.
  try {
    return Intl.getCanonicalLocales(tag ? tag.split(/[.@]/)[0].replaceAll("_", "-") : [])[0]
  } catch {}
}

/** The user's language, which their typing is in. */
const userLocale = () => (typeof navigator === "undefined" ? undefined : toLocale(navigator.languages?.[0] ?? navigator.language))

/**
 * Folds text for matching in one locale: `fold("Łódź")` is `"lodz"` in English, while Swedish keeps `"ä"` apart from
 * `"a"`. The result has the text's length, so indexes into it are indexes into the text.
 */
function createFolder(locale?: string) {
  const collator = new Intl.Collator(locale, { sensitivity: "base", usage: "search" })
  const lower = specialCase.test(locale ?? "") ? (text: string) => text.toLocaleLowerCase(locale) : (text: string) => text.toLowerCase()
  const chars = new Map<string, string>()
  const texts = new Map<string, string>()

  // One character's folded form, the same length, decided once by the collator.
  function foldChar(char: string) {
    let folded = chars.get(char)
    if (folded !== undefined) return folded
    const lowered = lower(char)
    if (char < "\u0080") {
      chars.set(char, lowered)
      return lowered
    }
    const stripped = lower(char.normalize("NFD").replace(diacritics, ""))
    if (stripped.length === char.length && stripped !== lowered && collator.compare(char, stripped) === 0) folded = stripped
    else if (latin.test(char)) folded = [...latinLetters].find((letter) => collator.compare(char, letter) === 0)
    folded ??= lowered.length === char.length ? lowered : char
    chars.set(char, folded)
    return folded
  }

  return (text: string) => {
    let folded = texts.get(text)
    if (folded !== undefined) return folded
    if (ascii.test(text)) folded = lower(text)
    else {
      folded = ""
      for (const char of text) folded += foldChar(char)
    }
    // Bounded, so labels that come and go can't grow it forever.
    if (texts.size > 50_000) texts.clear()
    texts.set(text, folded)
    return folded
  }
}

let folder: ((text: string) => string) | undefined
/** Text folded for matching in the user's language: case and accents that don't count there are gone. */
const foldText = (text: string) => (folder ??= createFolder(userLocale()))(text)

// Comparable form of a label in the user's language: case and accents that
// don't count there are gone, and so is surrounding space. "Éditer" matches
// "e".
const normalize = (text: string) => foldText(text).trim()

/** Whether a key press types a character a typeahead should search for: one printable character, without a shortcut
 * modifier. */
export const isTypeaheadKey = (event: Keyed) => event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey

function searchStart(current: number, query: string) {
  return query.length === 1 ? current + 1 : Math.max(current, 0)
}

/**
 * Typeahead as the WAI-ARIA Authoring Practices describe it for menus, listboxes and trees: typed characters build a
 * query that clears after `timeout` ms, and the next item whose label starts with it is the match. Pressing the same
 * character again steps through the items that start with it, unless a label starts with the repeated characters,
 * like "Llama" for "ll". Case and accents fold the way the user's language folds them.
 *
 * Returns `search(key, labels, current)`, which gives the index to move to or -1, with `reset()` and `active()`
 * (whether a query is in progress, so Space continues it rather than activating).
 */
export function createTypeahead(timeout = 500) {
  let query = ""
  let lastKey = ""
  let stepping = false
  let timer: ReturnType<typeof setTimeout> | undefined

  function reset() {
    query = ""
    lastKey = ""
    stepping = false
    clearTimeout(timer)
  }

  function scan(labels: string[], prefix: string, start: number) {
    const count = labels.length
    for (let step = 0; step < count; step++) {
      const index = (((start + step) % count) + count) % count
      if (normalize(labels[index]).startsWith(prefix)) return index
    }
    return -1
  }

  function search(key: string, labels: string[], current: number) {
    // Space only continues a query. Before one, it's the key that activates.
    if (key.length !== 1 || (key === " " && !query)) return -1
    clearTimeout(timer)
    timer = setTimeout(reset, timeout)
    const char = normalize(key) || key
    stepping = char === lastKey && (stepping || query.length === 1)
    lastKey = char
    query += char
    if (stepping) {
      const whole = scan(labels, query, Math.max(current, 0))
      return whole !== -1 ? whole : scan(labels, char, searchStart(current, char))
    }
    return scan(labels, query, searchStart(current, query))
  }

  return Object.assign(search, { reset, active: () => query.length > 0 })
}
