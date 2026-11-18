// A sign-in form, as a strict app would write one: a name, an email (the username a password manager fills), a
// password and a submit. What the app last got from each field is shown under it, so a check sees what reached JS.
import { createSignal } from "solid-js"
import { h, text, mount } from "./solid-host.js"

const FIELD = { height: "control", paddingInline: 12, borderRadius: "md", backgroundColor: "muted", color: "foreground", font: "body" }
const LABEL = { font: "label", color: "foreground" }

export function start(onUpdate) {
  const [name, setName] = createSignal("")
  const [email, setEmail] = createSignal("")
  const [password, setPassword] = createSignal("")
  const [sent, setSent] = createSignal("")
  const submit = () => setSent(`Signed in as ${email() || "nobody"} (${name() || "no name"}), password ${password().length} characters`)

  const field = (label, props) =>
    h(
      "box",
      { style: { gap: 4 } },
      text({ style: LABEL }, label),
      h("input", { "aria-label": label, style: FIELD, ...props }),
    )

  return mount(
    () =>
      h(
        "box",
        { role: "main", style: { flexDirection: "column", padding: "gutter", gap: "gap", backgroundColor: "background", height: "100%" } },
        text({ role: "heading", "aria-level": 1, style: { font: "title", color: "foreground" } }, "Sign in"),
        field("Name", { autocomplete: "name", autocapitalize: "words", enterkeyhint: "next", onInput: setName }),
        field("Email", { type: "email", autocomplete: "username", autocapitalize: "none", spellcheck: false, enterkeyhint: "next", onInput: setEmail }),
        field("Password", {
          type: "password",
          autocomplete: "current-password",
          enterkeyhint: "go",
          onInput: setPassword,
          onKeyDown: (key) => key === "Enter" && submit(),
        }),
        h(
          "box",
          {
            role: "button",
            onClick: submit,
            style: { height: "control", alignItems: "center", justifyContent: "center", borderRadius: "md", backgroundColor: "primary" },
          },
          text({ style: { font: "label", color: "primary-foreground" } }, "Sign In"),
        ),
        text({ role: "status", style: { font: "footnote", color: "muted-foreground" } }, () => `Name: ${name()} · Email: ${email()} · Password: ${"•".repeat(password().length)}`),
        text({ role: "status", style: { font: "footnote", color: "foreground" } }, sent),
      ),
    onUpdate,
  )
}
