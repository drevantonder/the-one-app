// A name a screen uses and nothing declares, or imports and @one/ui doesn't give, only fails when that line runs, which
// may be a state nobody opens. A screen's own build keeps React and @one/ui out, so nothing else would say. This does,
// over every screen in the repo: `bun run build` stops here. `one`, the bridge, is on every page.
import { readdirSync } from "node:fs"
import path from "node:path"
import ts from "typescript"

const here = import.meta.dirname
const root = path.join(here, "..", "..", "..")
const screens = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const at = path.join(dir, entry.name)
    if (entry.isSymbolicLink() || ["node_modules", "dist"].includes(entry.name)) return []
    if (entry.isDirectory()) return screens(at)
    return /\.[jt]sx$/.test(entry.name) ? [at] : []
  })

const program = ts.createProgram(["src/screen/ui/src", "src/screen/home", "src/screen/settings", "src/onboarding", "apps"].flatMap((d) => screens(path.join(root, d))), {
  allowJs: true,
  checkJs: true,
  noEmit: true,
  skipLibCheck: true,
  jsx: ts.JsxEmit.Preserve,
  target: ts.ScriptTarget.ESNext,
  module: ts.ModuleKind.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  types: [],
  baseUrl: here,
  paths: { "@/*": ["./src/*"], "@one/ui": ["./src/index.ts"], "react": ["./node_modules/@types/react"], "react/*": ["./node_modules/@types/react/*"], "react-dom/*": ["./node_modules/@types/react-dom/*"], "motion/*": ["./node_modules/motion/*"] },
})

const cannotFindName = [2304, 2552]
const notExported = [2305, 2724]
const problems = program.getSourceFiles().filter((file) => !file.isDeclarationFile).flatMap((file) => program.getSemanticDiagnostics(file)).filter((d) => {
  const text = ts.flattenDiagnosticMessageText(d.messageText, "\n")
  return (cannotFindName.includes(d.code) && text !== "Cannot find name 'one'.") || notExported.includes(d.code)
})
for (const d of problems) {
  const { line } = d.file.getLineAndCharacterOfPosition(d.start)
  console.error(`${path.relative(root, d.file.fileName)}:${line + 1}: ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`)
}
if (problems.length) process.exit(1)
