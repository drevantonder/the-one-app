// Lynx's background thread: the app's sandbox (PrimJS, one runtime per LynxView). It runs the app and sends each
// update to the main thread, which draws it; taps and typing come back as `publishEvent(<id>:<name>, data)`.
import "./lynx-globals.js"
import { start } from "./screen.js"

const app = lynx.getApp()
const event = start((update) => {
  lynx.getNativeApp().callLepusMethod("applyOps", { data: JSON.stringify(update), patchOptions: {} }, () => {})
})
app.publishEvent = (handler, data) => {
  const [id, name] = handler.split(":")
  event({ v: 1, op: "event", id: Number(id), name, value: data?.detail?.value })
}
