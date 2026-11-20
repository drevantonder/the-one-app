// The apps used lately on this device, and Android's shortcuts to apps.
import { icons, launcher, names } from "./apps.js"
import { open } from "./client.js"
/** @import { View } from "./host.js" */

/// The apps used lately on this device (`recent`) are what a long press on Android's app icon lists (`shortcuts`).
const RECENT = 8
/// The apps used lately on this device, by their keys, newest first.
/** @type {string[]} */
let recent = []
try {
  recent = JSON.parse(localStorage.getItem("recent") ?? "[]")
} catch {}
/// The apps shown since they were opened, which coming in front again doesn't move.
/** @type {WeakSet<View>} */
const fronted = new WeakSet()
/// An app has come in front: opened, it's the newest used.
/** @param {View} view */
export function used(view) {
  if (view.entrypoint === launcher || fronted.has(view)) return
  fronted.add(view)
  recent = [view.key, ...recent.filter((was) => was !== view.key)].slice(0, RECENT)
  try {
    localStorage.setItem("recent", JSON.stringify(recent))
  } catch {}
  shortcuts()
}
/// The apps used lately that are still this user's, the open ones first.
function recents() {
  const listed = recent
    .map((at) => ({ key: at, subject: at.slice(0, at.indexOf("|")) || undefined, entrypoint: at.slice(at.indexOf("|") + 1) }))
    .filter(({ entrypoint }) => names.has(entrypoint) && entrypoint !== launcher)
  return [...listed.filter(({ key }) => open.has(key)), ...listed.filter(({ key }) => !open.has(key))]
}

/// Android's shortcuts to apps here (`MainActivity`): the apps used lately, newest first, which a long press on the
/// app's icon lists, and every app installed, so one put on the home screen greys out once it's gone. An app opened
/// for a group has no link of its own to open it by, so it isn't among them.
let posted = ""
export async function shortcuts() {
  if (!globalThis.androidShortcuts) return
  const ones = recents().filter(({ subject }) => !subject)
  const now = JSON.stringify([ones.map(({ entrypoint }) => entrypoint), [...names.keys()]])
  if (now === posted) return
  posted = now
  const recent = await Promise.all(ones.map(({ entrypoint }) => shortcut(entrypoint)))
  globalThis.androidShortcuts.postMessage(JSON.stringify({ recent: recent.filter(Boolean), installed: [...names.keys()] }))
}

/// An app's shortcut as Android takes one: its entrypoint, name, and icon as a PNG of its square, 108 dp across at
/// the densest screens, as an adaptive icon is drawn, since Android draws no SVG. → nothing where it has no icon.
/** @param {string} entrypoint @returns {Promise<{ id: string, name: string, icon: string } | undefined>} */
export async function shortcut(entrypoint) {
  const src = icons.get(entrypoint)
  if (!src) return undefined
  const picture = new Image()
  picture.src = src
  await picture.decode()
  const canvas = Object.assign(document.createElement("canvas"), { width: 432, height: 432 })
  canvas.getContext("2d")?.drawImage(picture, 0, 0, 432, 432)
  return { id: entrypoint, name: names.get(entrypoint) || entrypoint, icon: canvas.toDataURL("image/png").slice("data:image/png;base64,".length) }
}
