// What Devices and Passkeys both draw with.
import { Button, Icon } from "ui"

export const on = (added: string | number) => new Date(added).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })

export const Remove = (props: { name: string; disabled?: boolean; onClick: () => void }) => (
  <Button variant="ghost" size="icon" aria-label={`Remove ${props.name}`} disabled={props.disabled} onClick={props.onClick}>
    <Icon name="trash" />
  </Button>
)
