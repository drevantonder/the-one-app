// The harness process: here, or on `--ssh`'s device, and the scenes it lists and draws.
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { basename, join } from "node:path"
import { capped, checking, dark, env, FACES, harness, out, ssh, stopped } from "./options.js"
import { COMPONENTS, library, pageFolder } from "./pages.js"
import { device, devices, hostScore } from "./tiers.js"


/// The folder on `--ssh`'s device that each run there is in, with the UI library and the harness beside them, made
/// as the first run needs it and taken away as this ends.
let remote
function onDevice() {
  if (remote) return remote
  const said = (command) => {
    const ran = Bun.spawnSync(["ssh", ssh, command], { stdout: "pipe", stderr: "pipe" })
    if (ran.exitCode !== 0) throw new Error(`${ssh}: ${command}: ${ran.stderr.toString().trim()}`)
    return ran.stdout.toString().trim()
  }
  remote = `${said("echo %TEMP%")}\\one-motion-${process.pid}`
  const staged = join(out, ".device")
  rmSync(staged, { recursive: true, force: true })
  cpSync(library("client"), join(staged, "ui"), { recursive: true, dereference: true })
  cpSync(harness, join(staged, "screenshot.exe"))
  for (const face of FACES) cpSync(face, join(staged, basename(face)))
  const sent = Bun.spawnSync(["sh", "-c", `tar -C "${staged}" -cf - . | ssh ${ssh} 'mkdir "${remote}" && tar -xf - -C "${remote}"'`], { stdout: "pipe", stderr: "pipe" })
  if (sent.exitCode !== 0) throw new Error(`${ssh}: the harness didn't go there: ${sent.stderr.toString().trim()}`)
  process.on("exit", () => Bun.spawnSync(["ssh", ssh, `rmdir /s /q "${remote}"`]))
  return remote
}

/// Runs the harness on `--ssh`'s device, in a folder of its own there with what it's given: its page, its answers and
/// its requests. → what it said on stdout and stderr, and whether it failed; its `end.json` back in `folder`.
export async function runThere(at, folder, args, lines) {
  const base = onDevice()
  const run = `${base}\\${basename(folder)}-${Math.random().toString(36).slice(2, 8)}`
  const staged = join(folder, ".device")
  rmSync(staged, { recursive: true, force: true })
  cpSync(at, join(staged, "page"), { recursive: true, dereference: true })
  cpSync(join(folder, "answers.json"), join(staged, "answers.json"))
  writeFileSync(join(staged, "requests.jsonl"), lines)
  const there = args.map((arg) => (arg === at ? `${run}\\page` : arg === join(folder, "answers.json") ? `${run}\\answers.json` : arg === "-" ? `${run}\\requests.jsonl` : arg.startsWith(folder) ? `${run}\\${basename(arg)}` : arg === library("client") ? `${base}\\ui` : arg))
  // Each argument quoted as Windows' C runtime reads one, a quote in it escaped (an `--errors` list's), and the whole
  // quoted for the shell here.
  const command = `mkdir "${run}" && tar -xf - -C "${run}" && "${base}\\screenshot.exe" ${there.map((arg) => `"${arg.replaceAll('"', '\\"')}"`).join(" ")} & type "${run}\\end.json" 1>&2 & rmdir /s /q "${run}"`
  const ran = Bun.spawn(["sh", "-c", `tar -C "${staged}" -cf - . | ssh ${ssh} '${command.replaceAll("'", "'\\''")}'`], { stdout: "pipe", stderr: "pipe" })
  const [stdout, stderr] = await Promise.all([new Response(ran.stdout).text(), new Response(ran.stderr).text(), ran.exited])
  rmSync(staged, { recursive: true, force: true })
  // Its tree (its nodes, a JSON array), which it writes at its end, comes back after what it said, if it said anything.
  const tree = /^[[{]/.test(stderr) ? 0 : Math.max(stderr.lastIndexOf("\n["), stderr.lastIndexOf("\n{")) + 1 || -1
  if (tree >= 0) writeFileSync(join(folder, "end.json"), stderr.slice(tree))
  return { stdout, stderr: tree >= 0 ? stderr.slice(0, tree) : stderr, failed: ran.exitCode !== 0 || !stdout.trim() }
}

/// A motion played in a real window on `--ssh`'s device, in its person's session, by the `window` example
/// (`--present`): the inspector's requests it's `played`, each at its time, after the scene's `taps`, in a window
/// opened afresh for each of three rounds. A scheduled task runs it there, and says it's done on a named pipe, which is
/// every session's, that this waits on there, five minutes at most. → what it said of each motion's frames, as JSON
/// lines, and whether it failed.
export async function presentThere(at, folder, scene, played, taps) {
  // A locked screen's windows are drawn but not shown, so presenting costs nothing there.
  if (Bun.spawnSync(["ssh", ssh, 'tasklist /FI "IMAGENAME eq LogonUI.exe" /NH']).stdout.toString().includes("LogonUI")) return { said: [], stderr: `${ssh}'s screen is locked, so a window there isn't shown: unlock it first`, failed: true }
  const base = onDevice()
  const name = `${basename(folder)}-${Math.random().toString(36).slice(2, 8)}`.replace(/[^\w-]/g, "")
  const run = `${base}\\${name}`
  const staged = join(folder, ".device")
  rmSync(staged, { recursive: true, force: true })
  cpSync(at, join(staged, "page"), { recursive: true, dereference: true })
  cpSync(join(folder, "answers.json"), join(staged, "answers.json"))
  writeFileSync(join(staged, "played.json"), JSON.stringify(played))
  writeFileSync(join(staged, "before.jsonl"), taps.map((tap, id) => JSON.stringify({ v: 1, id: id + 1, ...tap })).join("\n"))
  const args = ["page", "--play", "played.json", "--before", "before.jsonl", "--ui", `${base}\\ui`, "--answers", "answers.json", "--size", `${scene.size[0]}x${scene.size[1]}`, "--rounds", "3", ...panel(scene)]
  writeFileSync(join(staged, "run.cmd"), ["@echo off", `cd /d "${run}"`, `"${base}\\screenshot.exe" ${args.map((arg) => `"${arg}"`).join(" ")} > said.txt 2> err.txt`, "echo %errorlevel% > exit.txt", `echo done> \\\\.\\pipe\\${name}`].join("\r\n"))
  writeFileSync(
    join(staged, "wait.ps1"),
    [
      // Its person may write to it from their session, which a pipe's own permissions don't let them.
      "$security = New-Object System.IO.Pipes.PipeSecurity",
      "$security.AddAccessRule((New-Object System.IO.Pipes.PipeAccessRule([System.Security.Principal.WindowsIdentity]::GetCurrent().User, 'ReadWrite', 'Allow')))",
      `$pipe = New-Object System.IO.Pipes.NamedPipeServerStream('${name}', 'In', 1, 'Byte', 'None', 0, 0, $security)`,
      `schtasks /Create /F /SC ONCE /ST 23:59 /TN ${name} /TR '"${run}\\run.cmd"' /IT | Out-Null`,
      `schtasks /Run /TN ${name} | Out-Null`,
      "$null = $pipe.WaitForConnectionAsync().Wait(300000)",
      "$pipe.Dispose()",
      `schtasks /Delete /F /TN ${name} | Out-Null`,
    ].join("\r\n"),
  )
  const sent = Bun.spawnSync(["sh", "-c", `tar -C "${staged}" -cf - . | ssh ${ssh} 'mkdir "${run}" && tar -xf - -C "${run}"'`], { stdout: "pipe", stderr: "pipe" })
  rmSync(staged, { recursive: true, force: true })
  if (sent.exitCode !== 0) return { stdout: "", stderr: sent.stderr.toString(), failed: true }
  const task = `powershell -NoProfile -ExecutionPolicy Bypass -File "${run}\\wait.ps1" & type "${run}\\exit.txt" & type "${run}\\said.txt" & type "${run}\\err.txt" 1>&2 & rmdir /s /q "${run}"`
  const ran = Bun.spawn(["ssh", ssh, task], { stdout: "pipe", stderr: "pipe" })
  const [stdout, stderr] = await Promise.all([new Response(ran.stdout).text(), new Response(ran.stderr).text(), ran.exited])
  const [exit, ...said] = stdout.split("\n")
  return { said: said.filter((line) => line.trim().startsWith("{")).map((line) => JSON.parse(line)), stderr, failed: exit.trim() !== "0" }
}


/// Whether `scene`'s window is a panel: a computer's launcher is drawn as one on every computer, its glass the system's
/// (`native::launcher`), as its client says it is.
export const panelled = (scene) => {
  const client = scene.answers?.["one/client"]
  return Boolean(client?.launcher)
}

/// The harness's flags that open `scene`'s window as the app does: a launcher as a panel with a computer's corners,
/// 18 pt (clients/app/src/computer.rs `CORNER`), over the system's glass, in every harness alike.
export const panel = (scene) => (panelled(scene) ? ["--panel", "18"] : [])

/// The harness run on a scene, with `requests`: each answer, and its stderr. → `{ answers, said }`.
export async function run(at, folder, scene, requests, seed = 0) {
  const answers = { ...scene.answers, "ui/initialize": scene.answers?.["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
  writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
  const lines = requests.map((request, id) => JSON.stringify({ v: 1, id: id + 1, ...request })).join("\n")
  // Its pictures decode as slowly as on the slowest device of its kind: as this machine's CPU is to that one's, at
  // that one's device pixels a point, where it's drawn at one here; on a device, as that decodes them.
  const slowest = devices.tiers.find((tier) => tier.name === (scene.touch ? "low-end Android" : "Intel UHD laptop at 4K"))
  const slower = hostScore && !ssh ? ["--slower", String((hostScore / slowest.cpu) * slowest.scale ** 2)] : []
  const args = [at, join(folder, "end.png"), join(folder, "end.json"), "--ui", library("client"), "--answers", join(folder, "answers.json"), "--size", `${scene.size[0]}x${scene.size[1]}`, "--scale", String(device?.scale ?? 1), "--requests", "-", ...(dark ? ["--dark"] : []), ...(scene.touch ? ["--touch"] : []), ...(scene.reducedMotion ? ["--reduced-motion"] : []), ...(scene.errors ? ["--errors", JSON.stringify(scene.errors)] : []), ...(scene.safeArea ? ["--safe-area", scene.safeArea.join(",")] : []), ...(scene.latency ? ["--latency", String(scene.latency)] : []), "--time-zone", scene.timeZone ?? "UTC", ...(scene.hz ? ["--hz", String(scene.hz)] : []), ...(seed ? ["--seed", String(seed)] : []), ...panel(scene), ...slower]
  if (ssh) {
    const { stdout, stderr, failed } = await runThere(at, folder, args, lines)
    return { answers: stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line)), said: stderr.trim(), failed }
  }
  // Its pipelines compile as slowly as the Surface's (`open.pipelines_ms`), so a frame that waits for some is a long one.
  const compiling = { ONE_PIPELINES_MS: String(devices.measured.surface.open.pipelines_ms) }
  const harnessed = Bun.spawn(capped([harness, ...args]), { stdin: new TextEncoder().encode(lines), stdout: "pipe", stderr: "pipe", env: { ...env, ...compiling } })
  const [stdout, stderr] = await Promise.all([new Response(harnessed.stdout).text(), new Response(harnessed.stderr).text(), harnessed.exited])
  return { answers: stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line)), said: [stderr.trim(), stopped(harnessed.exitCode)].filter(Boolean).join("\n"), failed: harnessed.exitCode !== 0 }
}


/// A phone's safe area, top, right, bottom and left, as an iPhone with a Dynamic Island has it upright (14 Pro to 17):
/// its status bar and its home indicator.
export const IPHONE = [59, 0, 34, 0]
/// The narrowest a window is that isn't a phone's, in points: Material's compact width class ends there.
export const COMPACT = 600

/// A scene as a person has it: a window narrower than COMPACT is a phone's, under a finger and with IPHONE's safe area,
/// unless the scene says otherwise (`touch: false` for a narrow computer window, which it says it's for, or its own
/// `safeArea`); a wider one is a computer's.
export const phone = ({ touch, safeArea, ...scene }) => {
  const fingered = touch ?? scene.size[0] < COMPACT
  return { ...scene, touch: fingered, safeArea: safeArea ?? (fingered ? IPHONE : undefined) }
}

/// A folder's scenes that have motions, or with `all`, every one an app's page draws, each with its page, link,
/// answers, taps, size and touch.
export async function scenesOf(target, built, folder, all = false) {
  if (existsSync(join(target, "client/scenes.json"))) {
    const scenes = JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))
    const whole = (scene) => (scene.like ? { ...whole(scenes[scene.like]), taps: undefined, ...scene, answers: { ...whole(scenes[scene.like]).answers, ...scene.answers } } : scene)
    return Object.entries(scenes)
      .map(([name, scene]) => ({ name, ...whole(scene) }))
      .filter((scene) => all || (checking ? scene.checks : scene.motions))
      .map(({ name, page, component, uses = [], answers = {}, taps = [], size = [390, 844], touch, timeZone = "UTC", safeArea, errors, latency, motions, checks, seeds }) => {
        const [at, link] = (page ?? (component ? `${COMPONENTS}/${component}.html` : "client/index.html")).split("#")
        return phone({ name, page: at, link, uses, answers, taps, size, touch, timeZone, safeArea, errors, latency, motions, checks, seeds })
      })
  }
  // The gallery lists its own, drawn at `#scenes`.
  const listing = { link: "scenes", size: [390, 844], answers: {} }
  mkdirSync(folder, { recursive: true })
  const at = pageFolder(built, "index.html", folder)
  const listed = await run(at, folder, listing, [])
  const text = JSON.stringify(JSON.parse(readFileSync(join(folder, "end.json"), "utf8"))).match(/"name":"(\[.*?\])"/)?.[1]
  if (!text) throw new Error(`The gallery listed no scenes: ${listed.said}`)
  return JSON.parse(JSON.parse(`"${text}"`))
    .filter((scene) => all || (checking ? scene.checks : scene.motions))
    .map(({ name, taps = [], size = [390, 844], touch, safeArea, errors, motions, checks, seeds }) => phone({ name, page: "index.html", link: name, answers: {}, taps, size, touch, safeArea, errors, motions, checks, seeds }))
}
