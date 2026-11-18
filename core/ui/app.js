// Builds an app: `bun app.js <source> <out>`. A server runs it, as the UI library it serves holds it (build.js), when a
// app with an app is deployed, on the app's source as it was deployed, and serves what it writes to <out>. Each
// `.html` in <source> is a page, and each module its package.json `exports` (`"./profile": "./profile.tsx"`) is built as
// `<name>.js`, with its exports, for another app's page to import: a component it draws. What every app shares (Solid,
// Zag's Solid adapter and the UI library) is left out of it, and imported from the app `ui`, which the network serves
// once, so every page and every module it imports, whichever app built it, shares one Solid; the import map a server
// serves a page with names the same (`OneCore.Mcp`), for what's imported by name as it runs. A page the edge serves
// itself is built here too. A build that fails says why on stderr, a problem to a line with its file and line, and
// exits with 1.
//
// A page's classes in the strict style vocabulary (src/strict/style.ts), and the UI library's, are drawn as Tailwind
// draws them, from the tokens (src/strict/tailwind.js), in one stylesheet after the UI library's: every word in the
// app's source that's in the vocabulary, as Tailwind's own scanner finds them, and the UI library's (`classes.txt`,
// which build.js writes beside this). One stylesheet, so each class's place among the others is Tailwind's, whoever
// named it.
import { existsSync, readdirSync, readFileSync } from "node:fs"
import { dirname, extname, join, relative } from "node:path"
import { fileURLToPath } from "node:url"
import solid from "@solidjs/vite-plugin"
import { build, parseAst } from "vite"
import { named } from "./src/strict/style.ts"
import { utilities } from "./src/strict/tailwind.js"

const [source, out] = process.argv.slice(2)

// What an app imports that the app `ui` serves, by where it's served (ui/build.js).
const shared = { "solid-js": "/ui/client/solid.js", "@solidjs/web": "/ui/client/web.js", "@zag-js/solid": "/ui/client/zag.js", ui: "/ui/client/ui.js" }

const pages = readdirSync(source, { recursive: true }).filter((path) => path.endsWith(".html") && !path.split("/").includes("node_modules"))
// The modules it exports, by their names: `./profile` is `profile`.
const manifest = join(source, "package.json")
const exported = Object.entries((existsSync(manifest) && JSON.parse(readFileSync(manifest, "utf8")).exports) || {}).map(([name, file]) => [name.replace(/^\.\//, ""), join(source, file)])

const here = (path) => fileURLToPath(new URL(path, import.meta.url))
const words = (text) => text.split(/[\s"'`{}()<>,;=]+/)
const code = readdirSync(source, { recursive: true }).filter((path) => /\.[jt]sx?$/.test(path) && !path.split("/").includes("node_modules"))
const classes = [...words(readFileSync(here("classes.txt"), "utf8")), ...named(code.flatMap((path) => words(readFileSync(join(source, path), "utf8"))))]
const strict = await utilities(classes)

try {
  await build({
    configFile: false,
    root: source,
    // Its TypeScript is read as the UI library's is, whatever the app's own tsconfig.json says for its editor: a strict
    // app's extends one of the UI library's (tsconfig.strict.json), which isn't beside it where a server builds it.
    tsconfig: fileURLToPath(new URL("tsconfig.json", import.meta.url)),
    // Its files find each other wherever the client serves the app.
    base: "./",
    logLevel: "silent",
    plugins: [
      solid(),
      // Every page has the UI library's look before any of it has run, in the Colour Mode its client chose.
      {
        name: "ui",
        transformIndexHtml: () => [
          { tag: "script", attrs: { src: "/ui/client/colour-mode.js" }, injectTo: "head-prepend" },
          { tag: "link", attrs: { rel: "stylesheet", href: "/ui/client/ui.css" }, injectTo: "head-prepend" },
        ],
      },
      // Its classes, after the UI library's look and before the app's own.
      {
        name: "strict",
        enforce: "pre",
        resolveId: (id) => (id === "/strict.css" ? "\0strict.css" : undefined),
        load: (id) => (id === "\0strict.css" ? strict : undefined),
        transformIndexHtml: { order: "pre", handler: () => [{ tag: "link", attrs: { rel: "stylesheet", href: "/strict.css" }, injectTo: "head-prepend" }] },
      },
      // A module another page imports brings its own look: the stylesheets it imports, which a page links in its
      // HTML, it links in the page that imports it as it runs.
      {
        name: "exported-css",
        generateBundle(_, bundle) {
          for (const chunk of Object.values(bundle)) {
            if (chunk.type !== "chunk" || !exported.some(([name]) => name === chunk.name) || !chunk.viteMetadata?.importedCss.size) continue
            const here = dirname(chunk.fileName)
            const links = [...chunk.viteMetadata.importedCss].map((css) => `\ndocument.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href: new URL(${JSON.stringify(`./${relative(here, css)}`)}, import.meta.url).href }))`)
            // After its code and before its sourcemap's comment, so the lines the sourcemap maps stay where they were.
            chunk.code = chunk.code.replace(/(\n\/\/# sourceMappingURL=.*)?$/, (comment = "") => links.join("") + comment)
          }
        },
      },
    ],
    build: {
      outDir: out,
      emptyOutDir: true,
      sourcemap: true,
      // Every browser and web view the app runs in preloads modules itself, so a page doesn't load and run a polyfill first.
      modulePreload: { polyfill: false },
      rolldownOptions: {
        input: { ...Object.fromEntries(pages.map((page) => [page.replace(/\.html$/, ""), join(source, page)])), ...Object.fromEntries(exported) },
        external: Object.keys(shared),
        // A module another page imports keeps its name and what it exports.
        preserveEntrySignatures: "exports-only",
        output: { paths: shared, entryFileNames: (entry) => (exported.some(([name]) => name === entry.name) ? "[name].js" : "assets/[name]-[hash].js") },
      },
    },
  })
} catch (error) {
  for (const problem of error.errors ?? [error]) console.error(describe(problem))
  process.exit(1)
}

/// A problem as the compiler says it, with where: the file as the app's source names it, its line and column, and
/// the lines around it. Solid's compiler says only what's wrong, so a file it couldn't read is read again by Vite's
/// own parser, which says where.
function describe(problem) {
  let { message, loc: at = {}, frame } = problem
  const file = at.file ?? problem.id
  if (file && !at.line) {
    try {
      parseAst(readFileSync(file, "utf8"), { lang: extname(file).slice(1) }, file)
    } catch (parsed) {
      if (parsed.loc) {
        at = parsed.loc
        frame = parsed.frame
        message = parsed.message.split("\n")[1] ?? message
      }
    }
  }
  const where = file ? `${relative(source, file)}${at.line ? `:${at.line}:${at.column}` : ""}: ` : ""
  return `${where}${message.replace(`${file}: `, "").replaceAll(source + "/", "")}${frame ? `\n${frame}` : ""}`
}
