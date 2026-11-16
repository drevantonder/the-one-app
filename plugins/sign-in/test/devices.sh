#!/usr/bin/env bash
# Tests, on a network people sign in to (`tooling/network <folder> --edge --owner alice`), what a device keeps as it's
# let in, taken out and let in again (`Session` in core/client). A device shows its code until its person approves it
# where they signed in with a passkey, and a code that's stopped says so. A device that's removed is out once its
# network's address says so, which a server's word alone doesn't decide: it drops its token and keeps whose it was
# and what it kept, says it was removed, and shows a new code. The same person letting it in again finds what it kept
# still there; someone else signing in on it finds none of it.
#
#     cargo build --release && mise exec -- plugins/sign-in/test/devices.sh
set -euo pipefail
set -m # The network is a job with a process group of its own.

root=$(cd "$(dirname "$0")/../../.." && pwd)
folder=$(mktemp -d)
"$root/tooling/network" "$folder" --edge --owner alice >"$folder/network.log" 2>&1 &
network=$!
trap 'kill -- "-$network" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$folder"' EXIT
grep -q -m1 "A network is running" <(tail -F --pid=$network -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }

# The people are browsers holding a passkey each, and the devices are the CLI in homes of their own.
cd "$root/core/edge"
FOLDER=$folder ONE=$root/target/release/one bun -e "$(cat <<'EOF'
import { chromium } from "playwright"
import { createPrivateKey, createPublicKey } from "node:crypto"
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"

const { FOLDER: folder, ONE: one } = process.env
const log = readFileSync(`${folder}/network.log`, "utf8")
const address = log.match(/A network is running in \S+, at (\S+)\./)[1]
const say = (what) => console.log(`\n== ${what}`)
let failed = 0
const check = (ok, what) => {
  console.log(`  ${ok ? "yes" : "NO "}  ${what}`)
  if (!ok) failed++
}

// Each person from an address of their own, as Better Auth limits how often one address asks.
const browser = await chromium.launch()
let people = 0
async function person(join, name) {
  const context = await browser.newContext({ extraHTTPHeaders: { "cf-connecting-ip": `10.0.0.${++people}` } })
  const page = await context.newPage()
  const cdp = await context.newCDPSession(page)
  await cdp.send("WebAuthn.enable")
  await cdp.send("WebAuthn.addVirtualAuthenticator", { options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } })
  await page.goto(join)
  await page.fill("input[name=name]", name)
  await page.click("text=Create a passkey")
  await page.waitForURL((url) => !url.pathname.startsWith("/sign-in"))
  return page
}
/** Answers the code a device shows in `page`, by its link. */
const answers = (page, yes = true) => async (link) => {
  await page.goto(link)
  await page.getByRole("button", { name: yes ? "It matches" : "Stop" }).click()
  await page.locator(".ui-sign-in .done").waitFor()
}

function device(name) {
  const home = `${folder}/devices/${name}`
  mkdirSync(home, { recursive: true })
  writeFileSync(`${home}/network`, JSON.stringify({ address }))
  return home
}
/** The key a device is known by. */
function keyOf(home) {
  const der = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), readFileSync(`${home}/key`)])
  return Buffer.from(createPublicKey(createPrivateKey({ key: der, format: "der", type: "pkcs8" })).export({ format: "der", type: "spki" }).subarray(-32)).toString("hex")
}
/** Runs `one` on a device: what it said, and its code. The link a device shows is answered by `answer`. */
async function run(home, args, answer) {
  const { ONE_AS, ONE_SUBJECT, ONE_LINK, ...env } = process.env
  const child = Bun.spawn([one, ...args], { env: { ...env, ONE_HOME: home }, stdout: "pipe", stderr: "pipe" })
  let said = ""
  for await (const chunk of child.stderr) {
    said += new TextDecoder().decode(chunk)
    const link = said.match(/(http:\/\/\S+user_code=[A-Z0-9]+)/)?.[1]
    if (link && answer) await answer(link, said).catch((error) => (child.kill(), (said += `\n${error}`))).finally(() => (answer = undefined))
  }
  return { code: await child.exited, out: await new Response(child.stdout).text(), said }
}
const config = (home) => JSON.parse(readFileSync(`${home}/network`, "utf8"))
const kept = (home) => existsSync(`${home}/cache/kept`) && existsSync(`${home}/files/kept`)

say("A device is let in once its person approves its code, and a code that's stopped says so")
const alice = await person(log.match(/users\/alice joins at (\S+)/)[1], "Alice")
const desk = device("desk")
let ran = await run(desk, ["sign-in", "devices"], answers(alice))
check(ran.code === 0, `alice approves the desk`)
const tablet = device("tablet")
ran = await run(tablet, ["sign-in", "devices"], answers(alice, false))
check(ran.code !== 0 && /denied where you signed in/.test(ran.said), "a tablet she stops says it was denied")

say("A device that's removed keeps what it kept, until someone else signs in on it")
const phone = device("phone")
await run(phone, ["sign-in", "devices"], answers(alice))
for (const folder of ["cache", "files"]) mkdirSync(`${phone}/${folder}`, { recursive: true }), writeFileSync(`${phone}/${folder}/kept`, "alice's")
await run(desk, ["sign-in", "remove_device", "--device", keyOf(phone)])
ran = await run(phone, ["sign-in", "devices"])
check(/removed from your network/.test(ran.said), "the phone, asking with its token, says it was removed")
check(!config(phone).token && config(phone).who === "users/alice" && kept(phone), "it dropped its token, and kept whose it was and what it kept")
ran = await run(phone, ["sign-in", "devices"], answers(alice))
check(ran.code === 0 && /removed from your network/.test(ran.said), "opened again it says so with a new code, and alice lets it in")
check(kept(phone), "what it kept is still hers")
const bob = await person((await run(desk, ["sign-in", "invite", "--name", "bob"])).out.match(/(http:\/\/\S+#invite=[0-9a-f]+)/)[1], "Bob")
await run(desk, ["sign-in", "remove_device", "--device", keyOf(phone)])
await run(phone, ["sign-in", "devices"])
ran = await run(phone, ["sign-in", "devices"], answers(bob))
check(ran.code === 0 && config(phone).who === "users/bob" && !existsSync(`${phone}/cache/kept`) && !existsSync(`${phone}/files/kept`), "bob signs in on it, and finds none of alice's")

await browser.close()
console.log(failed ? `\n${failed} failed` : "\nAll passed.")
process.exit(failed ? 1 : 0)
EOF
)"
