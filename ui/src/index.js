// The kit: every element, defined as it's imported, and the few things a page asks for in passing.
//
// A `///` comment that starts with an example is the kit's guide: build.js gathers them, in the order a page loads
// them, as the guide an agent is handed with the kit.

/// `<link rel="stylesheet" href="../ui/ui.css">` and `<script type="module" src="../ui/ui.js"></script>` in an
/// app's page give it the kit: the theme, light and dark, as the custom properties ui.css sets (`--foreground`,
/// `--muted-foreground`, `--border`, `--warning`, `--radius`, `--gap`), and the elements below, which look the
/// same on every client. The app's own module imports what it calls from the same file:
/// `import { call, changed, toast } from "../ui/ui.js"`. What an element's example shows as an attribute is a
/// property too (`row.heading = "Coffee"`), `hidden` hides any of them, and the page lays out what's its own
/// with its own CSS. A page shows pictures only from `data:` addresses and its plugin's own files.
import "./icon.js"
import "./spinner.js"
import "./button.js"
import "./input.js"
import "./number.js"
import "./checkbox.js"
import "./radio.js"
import "./file.js"
import "./select.js"
import "./date-picker.js"
import "./list.js"
import "./screen.js"
import "./split.js"
import "./menu.js"
import "./tabs.js"
import "./states.js"

export { call, changed, leave, upload } from "./bridge.js"
export { words } from "./element.js"
export { go } from "./screen.js"
export { confirm } from "./sheet.js"
export { toast } from "./toast.js"
