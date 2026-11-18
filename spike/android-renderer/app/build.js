// Builds both hosts' assets into the Android app: each screen as one ES module for the native host's QuickJS
// (`strict/<screen>.js`, which sets `globalThis.ui` as the UI library does for a native host), and the WebView's page.
// `bun build.js`
import { readFileSync, writeFileSync, mkdirSync, cpSync } from "node:fs"
import { build } from "esbuild"

const here = import.meta.dir
const out = `${here}/../android/app/src/main/assets`
mkdirSync(`${out}/web`, { recursive: true })
mkdirSync(`${out}/strict`, { recursive: true })
const tokensCss = readFileSync(`${here}/../../../core/ui/src/tokens.css`, "utf8")

async function bundle(contents, format, target) {
  const r = await build({
    stdin: { contents, resolveDir: here, loader: "js" },
    bundle: true, format, target, minify: true, write: false, conditions: ["browser"],
    define: { "process.env.NODE_ENV": '"production"' },
  })
  return r.outputFiles[0].text
}

for (const screen of ["screen", "form"]) {
  const js = await bundle(
    `import { start } from "./${screen}.js"\nconst event = start((update) => host.update(update))\nglobalThis.ui = { event, message() {} }\n`,
    "esm", "es2023",
  )
  writeFileSync(`${out}/strict/${screen === "screen" ? "notifications" : screen}.js`, js)
  console.log(screen, js.length)
}

const web = await bundle(`import "./web-host.js"`, "iife", "es2020")
writeFileSync(
  `${out}/web/index.html`,
  `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>${tokensCss.replace(/@font-face\s*{[^}]*}/, "")}
html,body,#root{margin:0;height:100%;font-family:system-ui,sans-serif;background:var(--background);color:var(--foreground)}*{box-sizing:border-box}input{border:0;outline:0}</style></head>
<body><div id="root" style="display:flex;flex-direction:column"></div><script>${web}</script></body></html>`,
)
cpSync(`${here}/icons`, `${out}/web/icons`, { recursive: true })
cpSync(`${here}/icons`, `${out}/strict/icons`, { recursive: true })
console.log("web", web.length)
