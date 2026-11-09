// The kit as one script every page that opts in runs once (src/screen.rs): React and the components, kept on
// `OneUI` by the names screens import them by, which the page's import map gives as small modules (build.js).
import * as React from "react"
import * as jsxRuntime from "react/jsx-runtime"
import * as compilerRuntime from "react/compiler-runtime"
import * as ReactDOM from "react-dom"
import * as ReactDOMClient from "react-dom/client"
import * as ui from "./index"
import "./ui.css"

// A screen that throws while it draws would be blank for good (React takes down its whole tree): its root shows one problem
// instead, with Try Again, which draws what it last drew.
const guarded = {
  ...ReactDOMClient,
  createRoot(container: Element | DocumentFragment, options?: ReactDOMClient.RootOptions) {
    let last: React.ReactNode = null
    const root: ReactDOMClient.Root = ReactDOMClient.createRoot(container, {
      onUncaughtError: (error, info) => {
        console.error(error, info.componentStack)
        const retry = () => render(last)
        queueMicrotask(() => render(React.createElement("div", { className: "p-5" }, React.createElement(ui.Problem, { title: "This screen stopped working.", text: "Something in it broke.", action: { label: "Try Again", run: retry } }))))
      },
      ...options,
    })
    const render = root.render.bind(root)
    root.render = (node) => render((last = node))
    return root
  },
}

Object.assign(globalThis, {
  OneUI: {
    react: React,
    "react/jsx-runtime": jsxRuntime,
    "react/compiler-runtime": compilerRuntime,
    "react-dom": ReactDOM,
    "react-dom/client": guarded,
    "@one/ui": ui,
  },
})

// Windows' shell zooms its pages to 14/17 of the user's text scale, sizing the old kit's 17px text to its own (ui.css
// undoes the others'); this kit is sized as it is, so it keeps only their scale.
const webview = (globalThis as { chrome?: { webview?: unknown } }).chrome?.webview
if (webview) {
  document.addEventListener("DOMContentLoaded", () => {
    const zoom = parseFloat(document.documentElement.style.zoom)
    if (zoom) document.documentElement.style.zoom = String((zoom * 17) / 14)
  })
}
