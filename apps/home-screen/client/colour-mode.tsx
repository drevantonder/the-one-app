// Appearance, on a page of its own, for links that open it. Settings draws the same component, in the same grouped cards.
import { ListStyle, render } from "ui"
import Appearance from "./appearance.tsx"

render(() => (
  <ListStyle value="insetGrouped">
    <Appearance />
  </ListStyle>
))
