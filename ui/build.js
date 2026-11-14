// Builds the kit into dist, which is the plugin `ui`: what a server hands to clients. ui.css is the one stylesheet,
// ui.js every component, solid.js and web.js are Solid, which the kit and every app share (app.js leaves them out
// of an app), and SKILL.md is the kit's skill.
import { readFileSync, writeFileSync } from "node:fs"
import { transform } from "lightningcss"
import solid from "@solidjs/vite-plugin"
import { build } from "vite"

const here = (path) => new URL(path, import.meta.url).pathname
const text = (path) => readFileSync(here(path), "utf8")

await build({
  configFile: false,
  root: here("."),
  // Its files find each other wherever the client serves them.
  base: "./",
  logLevel: "warn",
  plugins: [solid()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
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
writeFileSync(here("dist/ui.css"), transform({ filename: "ui.css", code: Buffer.from(css), minify: true }).code)

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
