// What Devices and Sign In & Security both draw with.
import { Button, Icon } from "ui"

export const on = (added: string | number) => new Date(added).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })

export const Remove = (props: { name: string; disabled?: boolean; onPress: () => void }) => (
  <Button variant="ghost" size="icon" aria-label={`Remove ${props.name}`} disabled={props.disabled} onPress={props.onPress}>
    <Icon name="trash" />
  </Button>
)
