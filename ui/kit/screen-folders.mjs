// Every folder of the client's own screens in React, from the repo's root: the frame's (ui/home, ui/settings) and each
// feature's (<feature>/screens/), as ui/src/ui_files.rs's `screen_folders` finds them. An app's (apps/) are its own.
import { existsSync, readdirSync } from "node:fs"
import path from "node:path"

export const screenFolders = (root) => [
  "ui/home",
  "ui/settings",
  ...readdirSync(root, { withFileTypes: true })
    .filter((e) => e.isDirectory() && !e.name.startsWith(".") && !["apps", "target", "node_modules"].includes(e.name) && existsSync(path.join(root, e.name, "screens")))
    .map((e) => `${e.name}/screens`)
    .sort(),
]
