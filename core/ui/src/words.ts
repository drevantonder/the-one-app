// The words the UI library says itself and how it reads a tool's error, which every client has, a native host's too.
import { createStore } from "solid-js"

/// The few words the UI library says itself, in English until a page says otherwise with `words`.
export const [messages, setMessages] = createStore({
  actions: "Actions",
  back: "Back",
  cantShowPage: "Can't show this page",
  cantShowHere: (what: string) => `Can't show this ${what.toLowerCase()} here yet`,
  cancel: "Cancel",
  clear: "Clear",
  close: "Close",
  copied: "Copied",
  copy: "Copy",
  details: "Details",
  exitFullScreen: "Exit Full Screen",
  forward: "Forward",
  fullScreen: "Full Screen",
  hideDetails: "Hide Details",
  loading: "Loading",
  more: "More",
  noActions: "No actions",
  notDrawnHere: "This One App doesn't draw it yet. It shows in a browser.",
  ok: "OK",
  open: "Open",
  searchActions: "Search for actions…",
  showDetails: "Show Details",
  tryAgain: "Try Again",
})

/// What a person reads of a tool's error: its first paragraph. What follows a blank line is for an agent, as MCP's
/// text is the model's: which tool to call next, or what went wrong inside an app.
export const readable = (error: string): string => error.split(/\n\s*\n/, 1)[0]

/// All of an error a screen shows: what a tool said, whole, where it's an `Error` `call` threw (its `cause`), or
/// the text itself.
export const whole = (error: string | Error): string => (error instanceof Error ? (typeof error.cause === "string" ? error.cause : error.message) : error)

/// `words({ back: "Zurück", cancel: "Abbrechen", close: "Schließen" })`: says the UI library's own words in the user's
/// language, once, for every component on the page. A word the UI library doesn't say throws the words it does.
export function words(theirs: Partial<Record<keyof typeof messages, string>>): void {
  for (const name in theirs) if (!(name in messages)) throw new Error(`The UI library has no word "${name}". It has: ${Object.keys(messages).join(", ")}`)
  setMessages((words) => void Object.assign(words, theirs))
}
