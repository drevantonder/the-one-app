import path from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// The kit: ui.js and ui.css, one script with React and the components. A screen built as an app's would be imports
// `shared` by the names the page's import map gives.
export const shared = ["react", "react/jsx-runtime", "react/compiler-runtime", "react-dom", "react-dom/client", "@one/ui"]

export default defineConfig({
  plugins: [react({ compiler: true }), tailwindcss()],
  // Welcome (../../onboarding) is outside this folder: React is this folder's.
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") }, dedupe: ["react", "react-dom", "motion"] },
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  build: {
    outDir: "dist",
    emptyOutDir: false,
    reportCompressedSize: false,
    lib: { entry: "src/main.ts", formats: ["iife"], name: "OneUIKit", fileName: () => "ui.js", cssFileName: "ui" },
  },
})
