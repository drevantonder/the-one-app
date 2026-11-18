// People & Groups, on a page of its own, for links that open it. Settings draws the same component, in the same grouped cards.
import { ListStyle, render } from "ui"
import PeopleAndGroups from "./people.tsx"

render(() => (
  <ListStyle value="insetGrouped">
    <PeopleAndGroups />
  </ListStyle>
))
