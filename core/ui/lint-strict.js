// `bun run lint:strict apps/<app>/client`, from the repo's root: type checks an app's page as strict, as its
// tsconfig.json says it is by extending tsconfig.strict.json, with TypeScript's own checker. It fails on an element of
// the DOM's, `document`, `window` or any other Web API, a stylesheet, and a prop a native host can't draw. A server runs it
// too, as the UI library it serves holds it (build.js), on the app it deploys, where an entrypoint says its `runtime_version`
// (`OneCore.Entrypoint.strict/3`). Its errors are TypeScript's, a line each: file, line, column and message, as the folder names it.
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

const folder = process.argv[2]
if (!folder) throw new Error("Which app's page? bun run lint:strict apps/<app>/client")
const { extends: base } = JSON.parse(readFileSync(join(folder, "tsconfig.json"), "utf8"))
if (!String(base).endsWith("tsconfig.strict.json")) {
  console.error(`${folder} isn't strict: its tsconfig.json extends ${base ?? "nothing"}, not core/ui/tsconfig.strict.json`)
  process.exit(1)
}
const tsc = Bun.spawnSync([process.execPath, fileURLToPath(new URL("node_modules/typescript/bin/tsc", import.meta.url)), "-p", ".", "--pretty", "false"], { cwd: folder, stdout: "inherit", stderr: "inherit" })
process.exit(tsc.exitCode)
