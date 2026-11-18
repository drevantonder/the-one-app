// Devices, on a page of its own: what its notification of a device waiting to sign in opens. Settings draws the same component, in the same grouped cards.
import { ListStyle, render } from "ui"
import Devices from "./devices.tsx"

render(() => (
  <ListStyle value="insetGrouped">
    <Devices />
  </ListStyle>
))
