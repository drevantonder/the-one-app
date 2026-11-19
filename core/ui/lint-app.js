// `bun run lint:strict apps/<app>/client`, from the repo's root: type checks an app's page against the UI library
// alone, as its tsconfig.json says by extending tsconfig.strict.json, with TypeScript's own checker. It fails on an element of
// the DOM's, `document`, `window` or any other Web API, a stylesheet, and a prop a native host can't draw. A server runs it
// too, as the UI library it serves holds it (build.js), on the app it deploys, where an entrypoint says its `runtime_version`
// (`OneCore.Entrypoint.strict/3`). Its errors are TypeScript's, a line each: file, line, column and message, as the folder names it.
// A raw colour or length in a static style is a warning, not an error: "make it a token" (a line each, as an error's, but
// `warning:`), which a deploy gives with its own.
import { readdirSync, readFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { newMade, styled } from "./tables.ts"

const folder = process.argv[2]
if (!folder) throw new Error("Which app's page? bun run lint:strict apps/<app>/client")
const { extends: base } = JSON.parse(readFileSync(join(folder, "tsconfig.json"), "utf8"))
if (!String(base).endsWith("tsconfig.strict.json")) {
  console.error(`${folder} isn't checked against the UI library: its tsconfig.json extends ${base ?? "nothing"}, not core/ui/tsconfig.strict.json`)
  process.exit(1)
}
const tsc = Bun.spawnSync([process.execPath, fileURLToPath(new URL("node_modules/typescript/bin/tsc", import.meta.url)), "-p", ".", "--pretty", "false"], { cwd: folder, stdout: "inherit", stderr: "inherit" })

// What its styles and tokens (`styles()`, `defineVars()`, `createTheme()`) can't be made of, as the build would refuse it,
// and what they warn of.
const root = resolve(folder)
const made = newMade()
let refused = false
for (const path of readdirSync(root, { recursive: true })) {
  if (!/\.[jt]sx?$/.test(path) || path.split("/").includes("node_modules")) continue
  try {
    styled(readFileSync(join(root, path), "utf8"), join(root, path), made)
  } catch (error) {
    console.error(error.message.replaceAll(`${root}/`, ""))
    refused = true
  }
}
for (const warning of made.warnings) console.log(warning.replaceAll(`${root}/`, ""))
process.exit(tsc.exitCode || refused ? 1 : 0)
