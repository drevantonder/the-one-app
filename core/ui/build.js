// Builds the UI library into dist, which is the app `ui`. What runs in a client is in client/, as every app's is: ui.css
// is the one stylesheet, ui.js every component, solid.js and web.js are Solid, and zag.js is Zag's Solid adapter,
// which the UI library and every app share (app.js leaves them out of an app). SKILL.md is the UI library's skill, and skills/ are Solid's, as the version apps are
// built with has them, Emil Kowalski's, pstack's, Modern Web Guidance and our design rules. build/ is how an app is built against it: app.js, with what it needs to run, which a server
// installs and runs on an app's source.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { transform } from "lightningcss"
import solid from "@solidjs/vite-plugin"
import { build } from "vite"
import { $ } from "bun"

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
    outDir: "dist/client",
    sourcemap: true,
    rolldownOptions: {
      input: { ui: here("src/index.ts"), solid: "solid-js", web: "@solidjs/web", zag: "@zag-js/solid" },
      // What an app imports from each is what each exports, whether or not the UI library uses it.
      preserveEntrySignatures: "strict",
      output: { entryFileNames: "[name].js", chunkFileNames: "ui-[hash].js" },
    },
  },
})

const css = text("src/tokens.css") + text("src/ui.css")
writeFileSync(here("dist/client/ui.css"), transform({ filename: "ui.css", code: Buffer.from(css), minify: true }).code)
// The typeface, beside the stylesheet that names it (tokens.css).
for (const file of ["Allo-Variable.woff2", "Allo-LICENSE.txt"]) cpSync(here(`src/${file}`), here(`dist/client/${file}`))

/// The UI library's skill: every `///` comment that starts with an example, from index.ts and then from each file it
/// exports from, in its order.
function skill() {
  const index = text("src/index.ts")
  const files = ["index.ts", ...new Set([...index.matchAll(/from "\.\/(.+?)"/g)].map(([, file]) => file))]
  return files.flatMap((file) =>
    [...text(`src/${file}`).matchAll(/(?:^\/\/\/.*\n)+/gm)]
      .map(([comment]) => comment.replace(/^\/\/\/ ?/gm, ""))
      .filter((comment) => file === "index.ts" || comment.startsWith("`")),
  )
}

writeFileSync(here("dist/SKILL.md"), `---
name: ui
description: Read before writing or changing an app's screens, which are written in Solid 2.0 with the UI library's components.
---

${skill().join("\n")}`)

// The UI library's types, for an app written in TypeScript: declarations only, unchecked (`bun run typecheck` checks).
await $`${here("node_modules/.bin/tsc")} -p . --noEmit false --declaration --emitDeclarationOnly --noCheck --rewriteRelativeImportExtensions --rootDir src --outDir dist/types`.cwd(here("."))

for (const file of ["app.js", "package.json", "bun.lock"]) cpSync(here(file), here(`dist/build/${file}`))

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

// Modern Web Guidance, as this repo's own agents have it (.agents/skills), with its LICENSE and README.
cpSync(here("../../.agents/skills/modern-web-guidance"), here("dist/skills/modern-web-guidance"), { recursive: true })

// Our design rules, from DESIGN.md as it is: its Everywhere, which every app follows.
const everywhere = text("../../DESIGN.md").match(/^## Everywhere\n([\s\S]*?)(?=^## )/m)[1]
mkdirSync(here("dist/skills/design"), { recursive: true })
writeFileSync(here("dist/skills/design/SKILL.md"), `---
name: design
description: Read before designing an app's screens or deciding how one looks, reads or behaves. It's the network's own rules, the same for every app.
---

# Design
${everywhere}`)
