// Builds the renderer's frame into a folder, `bun build.js <out>`, as the edge serves it at `/renderer/`
// (core/edge/edge.js): its page and script, and the renderer as WebAssembly, built by Cargo, bound to its JS by
// wasm-bindgen and made smaller by binaryen's wasm-opt (mise.toml has both).
import { $ } from "bun"
import { copyFileSync, mkdirSync } from "node:fs"

const [out] = process.argv.slice(2)
const here = (path) => new URL(path, import.meta.url).pathname
const root = here("../../../")
mkdirSync(out, { recursive: true })
for (const file of ["index.html", "renderer.js"]) copyFileSync(here(file), `${out}/${file}`)
await $`cargo build -p web-renderer --target wasm32-unknown-unknown --profile web`.cwd(root).quiet()
await $`wasm-bindgen --target web --no-typescript --out-dir ${out} ${root}/target/wasm32-unknown-unknown/web/web_renderer.wasm`.quiet()
await $`wasm-opt -Oz --strip-debug --strip-producers ${out}/web_renderer_bg.wasm -o ${out}/web_renderer_bg.wasm`.quiet()
