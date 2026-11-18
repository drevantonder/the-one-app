import { Alert, Button, Menu, Sheet, Text, type Scenes } from "ui"

const OwnMenu = () => (
  <Menu.Root>
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
  "Over the page/Menu, closed": () => <OwnMenu />,
  "Over the page/Menu, open": { render: () => <OwnMenu />, taps: ["Open Menu"] },
} satisfies Scenes
