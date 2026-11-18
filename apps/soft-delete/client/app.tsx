// Recently Deleted, on a page of its own, for links that open it. Settings draws the same component, in the same grouped cards.
import { ListStyle, render } from "ui"
import RecentlyDeleted from "./deleted.tsx"

render(() => (
  <ListStyle value="insetGrouped">
    <RecentlyDeleted />
  </ListStyle>
))
