// AI, on a page of its own, for links that open it. Settings draws the same component, in the same grouped cards.
import { ListStyle, render } from "ui"
import AI from "./ai.tsx"

render(() => (
  <ListStyle value="insetGrouped">
    <AI />
  </ListStyle>
))
