// Builds the kit into dist, which is the plugin `ui`. What runs in a client is in app/, as every plugin's is: ui.css
// is the one stylesheet, ui.js every component, solid.js and web.js are Solid, which the kit and every app share
// (app.js leaves them out of an app). SKILL.md is the kit's skill, and skills/ are Solid's, as the version apps are
// built with has them. build/ is how an app is built against it: app.js, with what it needs to run, which a server
// installs and runs on an app's source.
import { cpSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { transform } from "lightningcss"
import solid from "@solidjs/vite-plugin"
import { build } from "vite"

const here = (path) => new URL(path, import.meta.url).pathname
const text = (path) => readFileSync(here(path), "utf8")

rmSync(here("dist"), { recursive: true, force: true })
await build({
  configFile: false,
  root: here("."),
  // Its files find each other wherever the client serves them.
  base: "./",
  logLevel: "warn",
  plugins: [solid()],
  build: {
    outDir: "dist/app",
    sourcemap: true,
    rolldownOptions: {
      input: { ui: here("src/index.js"), solid: "solid-js", web: "@solidjs/web" },
      // What an app imports from each is what each exports, whether or not the kit uses it.
      preserveEntrySignatures: "strict",
      output: { entryFileNames: "[name].js", chunkFileNames: "ui-[hash].js" },
    },
  },
})

const css = text("src/tokens.css") + text("src/ui.css")
writeFileSync(here("dist/app/ui.css"), transform({ filename: "ui.css", code: Buffer.from(css), minify: true }).code)

/// The kit's skill: every `///` comment that starts with an example, from index.js and then from each file it
/// exports from, in its order.
function skill() {
  const index = text("src/index.js")
  const files = ["index.js", ...new Set([...index.matchAll(/from "\.\/(.+?)"/g)].map(([, file]) => file))]
  return files.flatMap((file) =>
    [...text(`src/${file}`).matchAll(/(?:^\/\/\/.*\n)+/gm)]
      .map(([comment]) => comment.replace(/^\/\/\/ ?/gm, ""))
      .filter((comment) => file === "index.js" || comment.startsWith("`")),
  )
}

writeFileSync(here("dist/SKILL.md"), `---
name: ui
description: Read before writing or changing an app's screens, which are written in Solid 2.0 with the kit's components.
---

${skill().join("\n")}`)

for (const file of ["app.js", "package.json", "bun.lock"]) cpSync(here(file), here(`dist/build/${file}`))

// Solid's own skills, unchanged, and the cheatsheet its primer points to, beside it.
cpSync(here("node_modules/solid-js/skills"), here("dist/skills"), { recursive: true })
cpSync(here("node_modules/solid-js/CHEATSHEET.md"), here("dist/skills/solid-2.0/CHEATSHEET.md"))
