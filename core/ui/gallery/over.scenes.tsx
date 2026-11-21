import { createSignal, For, onCleanup, Show } from "solid-js"
import { Alert, Button, Menu, Screen, Sheet, Text, toast, View, type Scenes } from "ui"

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

/// A toast with an action, and what its action did, as text, so a check can say.
const Toasts = () => {
  const [restored, setRestored] = createSignal(false)
  return (
    <View>
      <Button onPress={() => toast("Note deleted", { action: { label: "Undo", run: () => setRestored(true) } })}>Delete Note</Button>
      <Text>{restored() ? "Restored" : "Not restored"}</Text>
    </View>
  )
}

/// What people expect of a toast, from the UI SDK's own tests (tests/specs/toast.spec.ts at the commit toast.ts names):
/// it goes by itself after its time (8 s, with an action), not while the pointer rests on it nor while the page isn't
/// seen, and its action runs and closes it. A toast is found as upstream's tests find one, a list item in the
/// Notifications region named by what it says: its words alone are also what a screen reader was read (`announce`),
/// which stays after it goes.
const undo = { role: "button", name: "Undo" }
const deleted = { role: "listitem", name: "Note deleted" }
const toastChecks = {
  "It goes by itself after its time": ["Delete Note", { expect: { shown: deleted } }, { wait: 7500 }, { expect: { shown: deleted } }, { wait: 1000 }, { expect: { gone: deleted } }],
  "Hovered, it stays, and goes once the pointer leaves": ["Delete Note", { hover: undo }, { wait: 9000 }, { expect: { shown: deleted } }, { hover: { x: 195, y: 420 } }, { wait: 8500 }, { expect: { gone: deleted } }],
  "Its action runs and closes it": ["Delete Note", { wait: 300 }, undo, { expect: { text: "Restored" } }, { expect: { gone: deleted } }],
  "In the background, its time stands still": ["Delete Note", { away: 9000 }, { expect: { shown: deleted } }],
}

/// A menu whose items are asked for again and again, each answer new objects, as a page's data refreshing does (the
/// Home Screen's owners, asked for as it shows), kept by their keys (`For`'s `keyed`) so a press reaches what's under
/// it however often they come: made afresh instead, a press that goes down on one and up on its new one is lost, as on
/// the DOM, which a seed's page arriving late shows.
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
            <For each={items()} keyed={(owner) => owner.value}>
              {(owner) => <Menu.Item value={owner().value}>{owner().label}</Menu.Item>}
            </For>
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
  "Hovered, an item has the keyboard": ["Open Menu", { wait: 300 }, { hover: item("Rename") }, { expect: { focused: item("Rename") } }],
  "A press on an item that's off does nothing": ["Open Menu", { wait: 300 }, item("Turn Off"), { expect: { shown: { role: "menu" } } }, { key: "Escape" }, { expect: { text: "Chosen: nothing" } }],
  "Opened again, it's drawn where it opens": ["Open Menu", { expect: { shown: { role: "menu" } } }, { key: "Escape" }, { expect: { gone: { role: "menu" } } }, "Open Menu", { expect: { shown: { role: "menu" } } }],
  "Closed and opened again at once by its keys, it's drawn where it opens": [{ key: "Tab" }, { key: "Enter" }, { wait: 300 }, { key: "Escape" }, { key: "Enter" }, { expect: { shown: { role: "menu" } } }],
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

/// A pushed screen with a sheet over it, as Vault's Add Item is, whose own state outlives the screen. The screen says where
/// its Back goes after the sheet is open, so it registers with the device's Back later than the sheet does.
const Pushed = () => {
  const [adding, setAdding] = createSignal(false)
  const [pushed, setPushed] = createSignal(true)
  const [armed, setArmed] = createSignal(false)
  return (
    <Show when={pushed()} fallback={<Button onPress={() => setPushed(true)}>Vault</Button>}>
      <Screen heading="Vault" back onBack={armed() ? () => setPushed(false) : undefined}>
        <Button onPress={() => (setAdding(true), void setTimeout(() => setArmed(true), 100))}>Add Item</Button>
        <Sheet heading="New Item" open={adding()} onClose={() => setAdding(false)}>
          <Text>What a sheet holds.</Text>
        </Sheet>
      </Screen>
    </Show>
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
  // Hovered, as a pointer does: a computer's, in a narrow window.
  "Over the page/Toast, by itself": { render: () => <Toasts />, touch: false, checks: toastChecks },
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
  // Its glass is a computer's: under a finger an alert is drawn plain.
  "Over the page/An alert asked": {
    render: () => <Asking />,
    touch: false,
    checks: {
      "It opens over the page, its glass blurred": ["Use Key", { expect: { shown: { role: "alertdialog" } } }, { expect: { blur: true } }],
      "It dims the page behind it": ["Use Key", { wait: 400 }, { expect: { dimmed: true } }],
      "Its answer closes it": ["Use Key", { wait: 400 }, "Allow", { expect: { text: "Answered: yes" } }, { expect: { gone: { role: "alertdialog" } } }],
      "Escape closes it, its button with the keyboard": [{ key: "Tab" }, { key: "Enter" }, { wait: 400 }, { key: "Escape" }, { expect: { gone: { role: "alertdialog" } } }, { expect: { focused: { role: "button", name: "Use Key" } } }],
      "Its keyboard starts on what does least, and Tab stays in it": ["Use Key", { wait: 400 }, { expect: { focused: { role: "button", name: "Don't Allow" } } }, { key: "Tab" }, { expect: { focused: { role: "button", name: "Allow" } } }, { key: "Tab" }, { expect: { focused: { role: "button", name: "Don't Allow" } } }],
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
  // Android's OnBackPressedDispatcher and the web's top layer: what's drawn over the screen closes before it, however the
  // two registered.
  "Over the page/A sheet over a pushed screen": {
    render: () => <Pushed />,
    checks: {
      "Back closes only the sheet": ["Add Item", { wait: 400 }, { back: true }, { expect: { gone: { role: "dialog" } } }, { expect: { shown: "Add Item" } }],
      "Escape closes only the sheet": ["Add Item", { wait: 400 }, { key: "Escape" }, { expect: { gone: { role: "dialog" } } }, { expect: { shown: "Add Item" } }],
      "Back then closes the screen": ["Add Item", { wait: 400 }, { back: true }, { expect: { gone: { role: "dialog" } } }, { back: true }, { expect: { gone: "Add Item" } }, { expect: { shown: { role: "button", name: "Vault" } } }],
    },
  },
  "Over the page/Menu, closed": () => <OwnMenu />,
  "Over the page/Menu, open": { render: () => <OwnMenu />, taps: ["Open Menu"] },
  // Hovered, as a pointer does: a computer's, in a narrow window.
  "Over the page/Menu, what's chosen": { render: () => <Chosen />, touch: false, checks: menuChecks },
  "Over the page/Menu, its items asked for again": {
    render: () => <Refreshed />,
    checks: {
      "A press on an item reaches it as the items come again": ["Open Menu", { wait: 300 }, { press: item("Everyone"), hold: 120 }, { expect: { text: "Chosen: everyone" } }, { expect: { gone: { role: "menu" } } }],
    },
  },
} satisfies Scenes
