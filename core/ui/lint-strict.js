// `bun run lint:strict apps/<app>/client`, from the repo's root: checks that an app's page is strict, as its
// tsconfig.json says it is by extending tsconfig.strict.json, with TypeScript's own checker. It fails on an element of
// the DOM's, `document`, `window` or any other Web API, a stylesheet, and a prop a native host can't draw.
import { readFileSync } from "node:fs"
import { join } from "node:path"

const folder = process.argv[2]
if (!folder) throw new Error("Which app's page? bun run lint:strict apps/<app>/client")
const { extends: base } = JSON.parse(readFileSync(join(folder, "tsconfig.json"), "utf8"))
if (!String(base).endsWith("core/ui/tsconfig.strict.json")) {
  console.error(`${folder} isn't strict: its tsconfig.json extends ${base ?? "nothing"}, not core/ui/tsconfig.strict.json`)
  process.exit(1)
}
const tsc = Bun.spawnSync([new URL("node_modules/.bin/tsc", import.meta.url).pathname, "-p", folder], { stdout: "inherit", stderr: "inherit" })
process.exit(tsc.exitCode)
