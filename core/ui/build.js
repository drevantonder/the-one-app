// Builds the UI library into dist, which is the app `ui`. What runs in a client is in client/, as every app's is: ui.js
// every component, drawn by our renderer, web.js Solid's universal renderer telling it a page's mutations, solid.js
// and internal.js Solid and its internals (`solid-js/internal`), which the universal renderer reads with its views,
// and tokens.json and styles.json the tables it draws with. native/ is the same UI library where a client built before
// it was here asks for it. SKILL.md is the UI library's skill, and skills/ are Solid's, as the version apps are
// built with has them, Emil Kowalski's, pstack's and our design rules. build/ is how an app is built and checked against it: app.js and lint-app.js, with what they need to run, which a server
// installs and runs on an app's source.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import solid from "@solidjs/vite-plugin"
import { build } from "vite"
import { newMade, stylesPlugin, stylesTable, tokensCss, tokensTable } from "./tables.ts"
import { $ } from "bun"

// A path of the UI library's own, as this machine writes one (Windows' with its drive's letter).
const here = (path) => fileURLToPath(new URL(path, import.meta.url))
const text = (path) => readFileSync(here(path), "utf8")
/// Where what a client runs goes: dist/client, or with `bun build.js <folder>`, that folder, and nothing else is built,
/// as a program that has the UI library built in builds it for itself (clients/app/build.rs).
const into = process.argv[2]
const client = into ?? here("dist/client")

// Built afresh, but for dist/client itself, which Vite empties: a local network's edge serves it where it is, by a
// link (core/edge/edge.js), and stops if what it serves goes.
if (!into) for (const made of existsSync(here("dist")) ? readdirSync(here("dist")) : []) if (made !== "client") rmSync(here(`dist/${made}`), { recursive: true, force: true })
// The UI library's styles (src/styles.ts), made as each module is built.
const made = newMade()
// Solid, once, at an address of its own, so every page and module, whichever app built it, runs one Solid and one
// reactive runtime, whose proxies the universal renderer's views recognise.
await build({
  configFile: false,
  root: here("."),
  base: "./",
  logLevel: "warn",
  build: {
    outDir: client,
    emptyOutDir: true,
    sourcemap: true,
    rolldownOptions: {
      input: { solid: "solid-js", internal: "solid-js/internal" },
      preserveEntrySignatures: "strict",
      output: { entryFileNames: "[name].js", chunkFileNames: "solid-[hash].js" },
    },
  },
})
// The UI library (src/index.ts), and Solid's universal renderer as `@solidjs/web` (renderer.native.ts), which is what
// an app's own build imports of it (`createComponent`, `memo`).
await build({
  configFile: false,
  root: here("."),
  base: "./",
  logLevel: "warn",
  plugins: [stylesPlugin(made), solid()],
  resolve: { alias: { "@solidjs/web": here("src/drawing-primitives/renderer.native.ts") } },
  build: {
    outDir: client,
    emptyOutDir: false,
    sourcemap: true,
    rolldownOptions: {
      input: { web: here("src/drawing-primitives/renderer.native.ts"), ui: here("src/index.ts") },
      external: ["solid-js", "solid-js/internal"],
      // What an app imports from each is what each exports, whether or not the UI library uses it.
      preserveEntrySignatures: "strict",
      output: { entryFileNames: "[name].js", chunkFileNames: "ui-[hash].js", paths: { "solid-js": "/ui/client/solid.js", "solid-js/internal": "/ui/client/internal.js" } },
    },
  },
})
// Where a client built before it was here asks for it (`/ui/client/native/`), the same files.
mkdirSync(`${client}/native`, { recursive: true })
for (const file of readdirSync(client).filter((file) => /^(ui|web)(\.js|-.*\.js)(\.map)?$/.test(file))) cpSync(`${client}/${file}`, `${client}/native/${file}`)

// The tokens (src/tokens.ts) as CSS variables, which the app's window loads as they are (clients/app/tauri.conf.json),
// and as the renderer's table.
// Written only as it changes: a program that builds the UI library in (clients/app/build.rs) builds again as src/ does.
if (!existsSync(here("src/tokens.css")) || text("src/tokens.css") !== tokensCss()) writeFileSync(here("src/tokens.css"), tokensCss())
writeFileSync(`${client}/tokens.json`, JSON.stringify(tokensTable()))
writeFileSync(`${client}/styles.json`, JSON.stringify(stylesTable(made)))
// The licence of the UI SDK code it holds.
cpSync(here("src/UI-SDK-LICENSE.txt"), `${client}/UI-SDK-LICENSE.txt`)
if (into) process.exit()

/// The UI library's skill: every `///` comment that starts with an example, from index.ts and then from each file it
/// exports from, in its order. Each is of what's just under it, which a page can use (types/ui.d.ts exports it), and
/// is the only one of it: the build fails on one that isn't, so the skill tells of nothing a page can't import.
function skill() {
  const index = text("src/index.ts")
  const files = ["index.ts", ...new Set([...index.matchAll(/from "\.\/(.+?)"/g)].map(([, file]) => file))]
  const page = text("src/types/ui.d.ts")
  const exported = new Set([
    ...[...page.matchAll(/^export (?:declare )?(?:function|const|type|interface|class) (\w+)/gm)].map(([, name]) => name),
    ...[...page.matchAll(/^export (?:type )?\{([^}]+)\}/gm)].flatMap(([, names]) => names.split(",").map((name) => name.trim())),
  ])
  const told = new Set()
  return files.flatMap((file) =>
    [...text(`src/${file}`).matchAll(/(?:^\/\/\/.*\n)+/gm)].flatMap(({ 0: block, index: at, input }) => {
      const comment = block.replace(/^\/\/\/ ?/gm, "")
      if (file === "index.ts") return [comment]
      if (!comment.startsWith("`")) return []
      const under = input.slice(at + block.length).match(/^export (?:async )?(?:function|const|let|class|interface|type) (\w+)/)?.[1]
      if (!exported.has(under) || told.has(under))
        throw new Error(`src/${file}: \`///\` tells the ui skill of ${under ?? "what isn't exported under it"}, which ${told.has(under) ? "another already does" : "a page can't import (types/ui.d.ts)"}: make it a \`//\` comment, or export it`)
      told.add(under)
      return [comment]
    }),
  )
}

writeFileSync(here("dist/SKILL.md"), `---
name: ui
description: Read before writing or changing an app's screens, which are written in Solid 2.0 with the UI library's components.
---

${skill().join("\n")}`)

// The UI library's types, for an app written in TypeScript: declarations only, unchecked (`bun run typecheck` checks).
await $`${here("node_modules/.bin/tsc")} -p . --noEmit false --declaration --emitDeclarationOnly --noCheck --rewriteRelativeImportExtensions --rootDir src --outDir dist/types`.cwd(here("."))
// And the UI library as an app's page's tsconfig names it (tsconfig.app.json), which is declarations already.
cpSync(here("src/types"), here("dist/types/types"), { recursive: true })

for (const file of ["app.js", "lint-app.js", "tables.ts", "package.json", "bun.lock", "tsconfig.json", "tsconfig.app.json", "src/tokens.ts", "src/styles.ts", "src/drawing-primitives/renderer.ts"]) cpSync(here(file), here(`dist/build/${file}`))
cpSync(here("src/types"), here("dist/build/src/types"), { recursive: true })

// Solid's own skills, unchanged, and the cheatsheet its primer points to, beside it.
cpSync(here("node_modules/solid-js/skills"), here("dist/skills"), { recursive: true })
cpSync(here("node_modules/solid-js/CHEATSHEET.md"), here("dist/skills/solid-2.0/CHEATSHEET.md"))

// Emil Kowalski's skills (vendor/emil-kowalski-skills), each with its LICENSE, and a line in those that name React's
// APIs, after their frontmatter.
const emil = here("vendor/emil-kowalski-skills")
for (const name of ["gestures", "apple-design", "review-animations", "break-ui", "mobile-native"]) {
  cpSync(`${emil}/${name}`, here(`dist/skills/${name}`), { recursive: true })
  cpSync(`${emil}/LICENSE`, here(`dist/skills/${name}/LICENSE`))
}
// Lauren Tan's, from pstack (vendor/pstack), each with its LICENSE.
for (const name of ["typescript-best-practices", "principle-prove-it-works", "principle-fix-root-causes",
  "principle-model-the-domain", "principle-make-operations-idempotent", "principle-experience-first"]) {
  cpSync(here(`vendor/pstack/${name}`), here(`dist/skills/${name}`), { recursive: true })
  cpSync(here("vendor/pstack/LICENSE"), here(`dist/skills/${name}/LICENSE`))
}
for (const name of ["gestures", "review-animations"]) {
  const skill = here(`dist/skills/${name}/SKILL.md`)
  writeFileSync(skill, readFileSync(skill, "utf8").replace(/^(---\n[\s\S]*?\n---\n)/,
    "$1\nOur apps are written in Solid with the UI library (the `ui` skill), not React: where this names React's APIs, use Solid's.\n"))
}

// Our design rules, from DESIGN.md as it is: its Everywhere, which every app follows.
const everywhere = text("../../DESIGN.md").match(/^## Everywhere\n([\s\S]*?)(?=^## )/m)[1]
mkdirSync(here("dist/skills/design"), { recursive: true })
writeFileSync(here("dist/skills/design/SKILL.md"), `---
name: design
description: Read before designing an app's screens or deciding how one looks, reads or behaves. It's the network's own rules, the same for every app.
---

# Design
${everywhere}`)
