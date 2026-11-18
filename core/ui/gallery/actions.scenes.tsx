import { ActionBar, ActionPanel, Box, Keys, styles, Text, tokens, type Action, type Scenes } from "ui"

const look = styles({
  window: { position: "relative", height: tokens.grid[24], gap: tokens.grid[2] },
  row: { flexDirection: "row", gap: tokens.grid[3], alignItems: "center" },
})

const actions: Action[] = [
  { label: "Copy Token", keys: ["mod", "enter"], icon: "copy", onAction: () => {} },
  { label: "Edit", keys: ["mod", "E"], icon: "pencil-simple", onAction: () => {} },
  { label: "Add Field", icon: "plus", onAction: () => {} },
  { label: "Delete Item", keys: ["ctrl", "X"], icon: "trash", destructive: true, section: "danger", onAction: () => {} },
]

export default {
  "Actions/Keys": () => (
    <Box style={look.row}>
      <Keys keys={["mod", "K"]} />
      <Keys keys={["enter"]} />
      <Keys keys={["mod", "shift", "enter"]} />
      <Keys keys={["escape"]} />
    </Box>
  ),
  "Actions/Footer": {
    size: [750, 475],
    render: () => (
      <Box style={look.window}>
        <Text>What's under the footer.</Text>
        <ActionBar start={<Text>Vault</Text>} actions={actions} />
      </Box>
    ),
  },
  "Actions/Panel": {
    alone: true,
    size: [750, 475],
    render: () => <ActionPanel open onOpenChange={() => {}} actions={actions} />,
  },
} satisfies Scenes
