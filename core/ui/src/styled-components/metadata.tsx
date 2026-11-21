// What something is, as label and value rows, as Raycast's Detail.Metadata: written once on the drawing
// primitives, the same on every host.
import { children, For, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Text, View } from "#primitives"
import { touch } from "#bridge"
import { List, Row } from "./list.tsx"
import { negative, styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

const look = styles({
  // On a computer, Raycast's: small rows on the window, every other one faintly striped, the label grey on the left
  // and the value on the right.
  rows: { marginInline: negative(tokens.grid[2]), font: tokens.type.small, color: tokens.colour.label },
  row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: tokens.grid[3], minHeight: tokens.grid[7], paddingInline: tokens.grid[2], borderRadius: tokens.radius.small },
  striped: { backgroundColor: tokens.colour.stripe },
  title: { flexShrink: 0, font: tokens.type.small, color: tokens.colour.secondaryLabel },
  text: { flexDirection: "row", alignItems: "center", gap: tokens.grid[1.5], minWidth: 0, font: tokens.type.small, textAlign: "end" },
  value: { font: tokens.type.small, lineClamp: 1 },
})

type Label = { title: JSX.Element; text?: JSX.Element }

/// `<Metadata><Metadata.Label title="Username" text="drevan" /><Metadata.Label title="Added" text="14 Nov" /></Metadata>`:
/// what something is, a label and its value on each row, as Raycast's Detail.Metadata: small, every other row
/// striped, on a computer; under a finger, a grouped list of them, as iOS's Settings has. `text` may be words or what
/// shows it (an app's icon and name).
export function Metadata(props: { children?: JSX.Element }) {
  const labels = children(() => props.children)
  const all = () => labels.toArray() as unknown as Label[]
  return (
    <Show
      when={!touch()}
      fallback={
        <List>
          <For each={all()}>{(label) => <Row heading={label.title} end={label.text} />}</For>
        </List>
      }
    >
      <View role="list" style={look.rows}>
        <For each={all()}>
          {(label, at) => (
            <View role="listitem" style={[look.row, at() % 2 === 0 && look.striped]}>
              <Text style={look.title}>{label.title}</Text>
              <View style={look.text}>{typeof label.text === "string" ? <Text style={look.value}>{label.text}</Text> : label.text}</View>
            </View>
          )}
        </For>
      </View>
    </Show>
  )
}

/** One row: its label (`title`) and its value (`text`). It's what `Metadata` draws, so it draws nothing itself. */
Metadata.Label = (props: Label) => props as unknown as JSX.Element
