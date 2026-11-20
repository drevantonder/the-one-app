import { createSignal, For, onCleanup } from "solid-js"
import { Alert, Button, Menu, Sheet, Text, toast, View, type Scenes } from "ui"

const OwnMenu = (props: { onSelect?: (value: string) => void; onOpenChange?: (open: boolean) => void }) => (
  <Menu.Root onSelect={props.onSelect} onOpenChange={props.onOpenChange}>
    <Menu.Trigger aria-label="Open Menu" />
    <Menu.Positioner>
      <Menu.Content>
        <Menu.Group label="Install for">
          <Menu.Item value="me">Just Me</Menu.Item>
          <Menu.Item value="family">Family</Menu.Item>
        </Menu.Group>
        <Menu.Separator />
        <Menu.Item value="rename">Rename</Menu.Item>
        <Menu.Item value="off" disabled>
          Turn Off
        </Menu.Item>
        <Menu.Item value="delete" destructive>
          Delete
        </Menu.Item>
      </Menu.Content>
    </Menu.Positioner>
  </Menu.Root>
)

/// A menu and what was chosen from it, as text, so a check can say, kept only while whoever opened it waits for the
/// choice, as the Home Screen's owner switch does: closing it before telling the choice reads as dismissed.
const Chosen = () => {
  const [chosen, setChosen] = createSignal("nothing")
  // Plain, as a state machine outside the page's signals hears each at once.
  let waiting = false
  return (
    <View>
      <OwnMenu onOpenChange={(open) => (waiting = open)} onSelect={(value) => waiting && setChosen(value)} />
      <Text>{`Chosen: ${chosen()}`}</Text>
    </View>
  )
}

/// A menu whose items are asked for again and again, each answer new objects, as a page's data refreshing does (the
/// Home Screen's owners, asked for as it shows): a press must reach what's under it however often they come.
const Refreshed = () => {
  const owners = () => [{ value: "me", label: "Just Me" }, { value: "family", label: "Family" }, { value: "everyone", label: "Everyone" }]
  const [items, setItems] = createSignal(owners())
  const [chosen, setChosen] = createSignal("nothing")
  const asking = setInterval(() => setItems(owners()), 30)
  onCleanup(() => clearInterval(asking))
  return (
    <View>
      <Menu.Root onSelect={setChosen}>
        <Menu.Trigger aria-label="Open Menu" />
        <Menu.Positioner>
          <Menu.Content>
            <For each={items()}>{(owner) => <Menu.Item value={owner.value}>{owner.label}</Menu.Item>}</For>
          </Menu.Content>
        </Menu.Positioner>
      </Menu.Root>
      <Text>{`Chosen: ${chosen()}`}</Text>
    </View>
  )
}

const item = (name: string) => ({ role: "menuitem", name })
const trigger = { role: "button", name: "Open Menu" }

/// What people expect of a menu, from the UI SDK's own tests (tests/specs/menu.spec.ts at the commit menu.ts names).
const menuChecks = {
  "A press opens it, its first item with the keyboard": ["Open Menu", { expect: { shown: { role: "menu" } } }, { expect: { focused: item("Just Me") } }],
  "Enter opens it on its first item": [{ key: "Tab" }, { key: "Enter" }, { expect: { focused: item("Just Me") } }],
  "The arrows move through it, wrapping": [{ key: "Tab" }, { key: "Enter" }, { key: "ArrowDown" }, { expect: { focused: item("Family") } }, { key: "ArrowUp" }, { key: "ArrowUp" }, { expect: { focused: item("Delete") } }, { key: "ArrowDown" }, { expect: { focused: item("Just Me") } }],
  "Home and End go to its ends": [{ key: "Tab" }, { key: "Enter" }, { key: "End" }, { expect: { focused: item("Delete") } }, { key: "Home" }, { expect: { focused: item("Just Me") } }],
  "A letter goes to the next item it starts": [{ key: "Tab" }, { key: "Enter" }, { key: "r" }, { expect: { focused: item("Rename") } }, { wait: 1000 }, { key: "d" }, { expect: { focused: item("Delete") } }],
  "Enter chooses, closes it and gives its button the keyboard back": [{ key: "Tab" }, { key: "Enter" }, { key: "ArrowDown" }, { key: "Enter" }, { expect: { text: "Chosen: family" } }, { expect: { gone: { role: "menu" } } }, { expect: { focused: trigger } }],
  "A press on an item chooses it and closes it": ["Open Menu", { wait: 300 }, item("Rename"), { expect: { text: "Chosen: rename" } }, { expect: { gone: { role: "menu" } } }],
  "A press on an item that's off does nothing": ["Open Menu", { wait: 300 }, item("Turn Off"), { expect: { shown: { role: "menu" } } }, { key: "Escape" }, { expect: { text: "Chosen: nothing" } }],
  "Opened again, it's drawn where it opens": ["Open Menu", { expect: { shown: { role: "menu" } } }, { key: "Escape" }, { expect: { gone: { role: "menu" } } }, "Open Menu", { expect: { shown: { role: "menu" } } }],
  "Opened again as it closes, it's drawn where it opens": ["Open Menu", { wait: 300 }, { key: "Escape" }, "Open Menu", { expect: { shown: { role: "menu" } } }],
  "Escape closes it, its button with the keyboard": ["Open Menu", { key: "Escape" }, { expect: { gone: { role: "menu" } } }, { expect: { focused: trigger } }],
  "A press outside closes it": ["Open Menu", { wait: 300 }, { x: 380, y: 830 }, { expect: { gone: { role: "menu" } } }, { expect: { text: "Chosen: nothing" } }],
  "Tab closes it": [{ key: "Tab" }, { key: "Enter" }, { key: "Tab" }, { expect: { gone: { role: "menu" } } }],
}

/// A button that opens an alert, and what was answered.
const Asking = () => {
  const [open, setOpen] = createSignal(false)
  const [answer, setAnswer] = createSignal("nothing")
  const close = (said: string) => () => (setAnswer(said), setOpen(false))
  return (
    <View>
      <Button onPress={() => setOpen(true)}>Use Key</Button>
      <Text>{`Answered: ${answer()}`}</Text>
      <Alert heading="Vault wants to use a key" text="Sent only to api.github.com." open={open()} onClose={close("dismissed")} action={<><Button variant="outline" onPress={close("no")}>Don't Allow</Button><Button onPress={close("yes")}>Allow</Button></>} />
    </View>
  )
}

/// A button that opens a sheet.
const Adding = () => {
  const [open, setOpen] = createSignal(false)
  return (
    <View>
      <Button onPress={() => setOpen(true)}>Add Note</Button>
      <Sheet heading="New Note" open={open()} onClose={() => setOpen(false)}>
        <Text>What a sheet holds.</Text>
        <Button onPress={() => setOpen(false)}>Save</Button>
      </Sheet>
    </View>
  )
}

export default {
  "Over the page/Sheet": {
    alone: true,
    render: () => (
      <Sheet heading="Add Note" open onClose={() => {}}>
        <Text>What a sheet holds, which scrolls where it's taller than the window allows.</Text>
        <Button>Save</Button>
      </Sheet>
    ),
  },
  "Over the page/Toast": {
    alone: true,
    taps: ["Delete Note"],
    render: () => <Button onPress={() => toast("Note deleted", { action: { label: "Undo", run: () => {} } })}>Delete Note</Button>,
  },
  "Over the page/A tool's error as a toast": {
    alone: true,
    taps: ["Sync"],
    render: () => <Button onPress={() => toast(new Error("Couldn't reach the network\n\nnotes_sync: connection refused (os error 111)"))}>Sync</Button>,
  },
  "Over the page/A tool's error in full": {
    alone: true,
    taps: ["Sync", "Details"],
    render: () => <Button onPress={() => toast(new Error("Couldn't reach the network\n\nnotes_sync: connection refused (os error 111)"))}>Sync</Button>,
  },
  "Over the page/Alert": {
    alone: true,
    render: () => (
    <Alert
      heading="Vault wants to use a key"
      text="Sent only to api.github.com."
      open
      onClose={() => {}}
      action={
        <>
          <Button variant="outline">Don't Allow</Button>
          <Button>Allow</Button>
        </>
      }
    />
    ),
  },
  // What people expect of a dialog, from the UI SDK's own tests (tests/specs/dialog.spec.ts at the commit dialog.ts
  // names).
  "Over the page/An alert asked": {
    render: () => <Asking />,
    checks: {
      "It opens over the page, its glass blurred": ["Use Key", { expect: { shown: { role: "alertdialog" } } }, { expect: { blur: true } }],
      "Its answer closes it": ["Use Key", { wait: 400 }, "Allow", { expect: { text: "Answered: yes" } }, { expect: { gone: { role: "alertdialog" } } }],
      "Escape closes it, its button with the keyboard": [{ key: "Tab" }, { key: "Enter" }, { wait: 400 }, { key: "Escape" }, { expect: { gone: { role: "alertdialog" } } }, { expect: { focused: { role: "button", name: "Use Key" } } }],
      // The keyboard goes to the alert itself, so a screen reader reads its heading first (the UI SDK's goes to its
      // least harmful action), and Tab goes round its actions.
      "Its keyboard is in it, and Tab stays in it": ["Use Key", { wait: 400 }, { expect: { focused: { role: "alertdialog" } } }, { key: "Tab" }, { expect: { focused: { role: "button", name: "Don't Allow" } } }, { key: "Tab" }, { expect: { focused: { role: "button", name: "Allow" } } }, { key: "Tab" }, { expect: { focused: { role: "button", name: "Don't Allow" } } }],
    },
  },
  "Over the page/A sheet opened": {
    render: () => <Adding />,
    checks: {
      "It opens over the page": ["Add Note", { expect: { shown: { role: "dialog" } } }],
      "Escape closes it": ["Add Note", { wait: 400 }, { key: "Escape" }, { expect: { gone: { role: "dialog" } } }],
      "A press outside closes it": ["Add Note", { wait: 400 }, { x: 195, y: 20 }, { expect: { gone: { role: "dialog" } } }],
      "Its button closes it": ["Add Note", { wait: 400 }, "Save", { expect: { gone: { role: "dialog" } } }],
    },
  },
  "Over the page/Menu, closed": () => <OwnMenu />,
  "Over the page/Menu, open": { render: () => <OwnMenu />, taps: ["Open Menu"] },
  "Over the page/Menu, what's chosen": { render: () => <Chosen />, checks: menuChecks },
  "Over the page/Menu, its items asked for again": {
    render: () => <Refreshed />,
    checks: {
      "A press on an item reaches it as the items come again": ["Open Menu", { wait: 300 }, { press: item("Everyone"), hold: 120 }, { expect: { text: "Chosen: everyone" } }, { expect: { gone: { role: "menu" } } }],
    },
  },
} satisfies Scenes
