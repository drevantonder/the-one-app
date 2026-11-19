// Builds the renderer's frame into a folder, `bun build.js <out>`, as the edge serves it at `/renderer/`
// (core/edge/edge.js): its page and scripts, and the renderer as WebAssembly, built by Cargo, bound to its JS by
// wasm-bindgen and made smaller by binaryen's wasm-opt (mise.toml has both).
import { $ } from "bun"
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { brotliCompressSync, constants } from "node:zlib"

const [out] = process.argv.slice(2)
const here = (path) => new URL(path, import.meta.url).pathname
const root = here("../../../")
mkdirSync(out, { recursive: true })
for (const file of ["index.html", "renderer.js", "text-input.js", "semantics.js", "check.html", "check.js"]) copyFileSync(here(file), `${out}/${file}`)
// The gate's check page, an app's page built as a server builds one (core/ui/app.js), which check.html hosts.
await $`bun ${root}/core/ui/dist/build/app.js ${here("check")} ${out}/check`.quiet()
await $`cargo build -p web-renderer --target wasm32-unknown-unknown --profile web`.cwd(root).quiet()
await $`wasm-bindgen --target web --no-typescript --out-dir ${out} ${root}/target/wasm32-unknown-unknown/web/web_renderer.wasm`.quiet()
await $`wasm-opt -Oz --strip-debug --strip-producers ${out}/web_renderer_bg.wasm -o ${out}/web_renderer_bg.wasm`.quiet()
// Kept compressed at Brotli's most too, which the edge sends as it's kept (core/edge/web/web.ts).
const wasm = readFileSync(`${out}/web_renderer_bg.wasm`)
writeFileSync(`${out}/web_renderer_bg.wasm.br`, brotliCompressSync(wasm, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_LGWIN]: 24, [constants.BROTLI_PARAM_SIZE_HINT]: wasm.length } }))
