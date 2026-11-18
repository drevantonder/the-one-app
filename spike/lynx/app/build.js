// Builds both hosts' assets into the Android app: the Lynx bundle (main thread + background, encoded by Lynx's own
// encoder, @lynx-js/tasm, with no ReactLynx) and the WebView's page. `bun build.js`
import { readFileSync, writeFileSync, mkdirSync, cpSync } from "node:fs"
import { encode } from "@lynx-js/tasm"
import { build } from "esbuild"

const here = import.meta.dir
const out = `${here}/../android/app/src/main/assets`
mkdirSync(`${out}/web`, { recursive: true })

// tokens.css's colours, light, as Lynx takes them (rgba), from oklch.
const tokensCss = readFileSync(`${here}/../../../core/ui/src/tokens.css`, "utf8")
const tokens = {}
for (const [, name, light] of tokensCss.matchAll(/--([a-z-]+):\s*(?:light-dark\()?(oklch\([^)]*\))/g)) tokens[name] = rgba(light)
writeFileSync(`${here}/tokens.gen.js`, `export const TOKENS = ${JSON.stringify(tokens)}\n`)

function rgba(oklch) {
  const [l, c, h, a = 1] = oklch.match(/[\d.]+%?/g).map((v) => (v.endsWith("%") ? parseFloat(v) / 100 : parseFloat(v)))
  const hr = (h * Math.PI) / 180
  const A = c * Math.cos(hr), B = c * Math.sin(hr)
  const L_ = l + 0.3963377774 * A + 0.2158037573 * B, M_ = l - 0.1055613458 * A - 0.0638541728 * B, S_ = l - 0.0894841775 * A - 1.291485548 * B
  const L = L_ ** 3, M = M_ ** 3, S = S_ ** 3
  const lin = [4.0767416621 * L - 3.3077115913 * M + 0.2309699292 * S, -1.2684380046 * L + 2.6097574011 * M - 0.3413193965 * S, -0.0041960863 * L - 0.7034186147 * M + 1.707614701 * S]
  const [r, g, b] = lin.map((x) => Math.round(255 * Math.min(1, Math.max(0, x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055))))
  return `rgba(${r},${g},${b},${a})`
}

// esbuild, down to ES2019: Lynx's PrimJS has no logical assignment (`??=`), which Solid 2.0 uses.
async function bundle(entry) {
  const r = await build({ entryPoints: [`${here}/${entry}`], bundle: true, format: "iife", target: "es2019", minify: true, write: false, conditions: ["browser"], define: { "process.env.NODE_ENV": '"production"' } })
  return r.outputFiles[0].text
}

const main = await bundle("lynx-main.js")
const mainFirst = await bundle("lynx-main-first.js")
const background = await bundle("lynx-background.js")
const web = await bundle("web-host.js")

// The encode options rspeedy's template plugin passes Lynx's encoder, with our code in place of ReactLynx's.
const app = `(function(){'use strict';function n({tt}){tt.define('/app-service.js',function(e,module,_,i,l,u,a,c,s,f,p,d,h,v,g,y,lynx){${background}});return tt.require('/app-service.js');}return{init:n}})()`
for (const [name, root, manifest] of [["notifications", main, app], ["notifications-main", mainFirst, "(function(){return{init:function(){}}})()"]]) {
const result = encode({
  compilerOptions: {
    enableFiberArch: true, useLepusNG: true, enableReuseContext: true, bundleModuleMode: "ReturnByFunction", templateDebugUrl: "",
    debugInfoOutside: true, defaultDisplayLinear: false, enableCSSInvalidation: true, enableCSSSelector: true, enableLepusDebug: false,
    enableRemoveCSSScope: true, targetSdkVersion: "3.2", defaultOverflowVisible: true,
  },
  sourceContent: {
    dsl: "react_nodiff", appType: "card",
    config: { lepusStrict: true, enableA11y: true, enableAccessibilityElement: true, enableNewGesture: false, removeDescendantSelectorScope: true, debugMetadataUrl: "", enableFlexBasisZeroPercent: true },
  },
  css: { cssMap: {}, cssSource: {} },
  lepusCode: { root, lepusChunk: {}, filename: "main-thread.js" },
  manifest: { "/app-service.js": manifest },
  customSections: {},
})
if (result.status !== 0 && !result.buffer?.length) throw new Error(`encode: ${result.error_msg}`)
writeFileSync(`${out}/${name}.lynx.bundle`, result.buffer)
console.log(name, result.buffer.length)
}

writeFileSync(
  `${out}/web/index.html`,
  `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>${tokensCss.replace(/@font-face\s*{[^}]*}/, "")}
html,body,#root{margin:0;height:100%;font-family:system-ui,sans-serif;background:var(--background);color:var(--foreground)}*{box-sizing:border-box}input{border:0;outline:0}</style></head>
<body><div id="root" style="display:flex;flex-direction:column"></div><script>${web}</script></body></html>`,
)
cpSync(`${here}/icons`, `${out}/web/icons`, { recursive: true })
console.log("main", main.length, "background", background.length, "web", web.length)
