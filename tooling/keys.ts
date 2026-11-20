// What a passkey does on the sign-in page, done from a device that holds the key instead, for a network being built,
// where nobody has a passkey, and once on a live one to encrypt a group's or the network's key to its owner's:
//
//     bun tooling/keys.ts wrap <scope>      encrypts <scope>'s key to the key of whoever `one` acts as, both of which this
//                                           device holds, so their passkey opens it (`encryption`'s `store`, `wrapped`)
//     bun tooling/keys.ts servers [scope]   hands the keys of what's encrypted for whoever `one` acts as, or for
//                                           <scope>, to the servers waiting for them; only with ONE_AS, on a network
//                                           being built
//
// It's the sign-in page's own code (`apps/sign-in/edge/page/passkey.ts`), with the key opened from this device's copy
// rather than from a passkey: a device that holds a key can always hand it on, which is what the encryption app's skill
// says the design trusts. `one` is ONE_PROGRAM, or the installed one, and the device is ONE_HOME's.
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { homedir } from "node:os"

// passkey.ts asks a passkey's PRF for its network's host as it loads, which nothing here uses.
Object.assign(globalThis, { location: { host: "" } })
const { forServers, fromDevice, wrap } = await import("../apps/sign-in/edge/page/passkey.ts")

const [command, scope] = process.argv.slice(2)
const one = process.env.ONE_PROGRAM ?? "one"
const home = process.env.ONE_HOME ?? join(homedir(), ".local/share/app.theone.client")

/// A tool's answer, as `one` gives it, for whoever it acts as or for `subject`.
function tool(name: string, subject?: string, inputs?: object) {
  const env = { ...process.env, ...(subject && { ONE_SUBJECT: subject }) }
  const args = name.split(" ")
  // What's handed over is encrypted, so it may be seen on the command line.
  for (const [input, value] of Object.entries(inputs ?? {})) args.push(`--${input}`, JSON.stringify(value))
  const ran = Bun.spawnSync([one, ...args], { env, stderr: "inherit" })
  if (ran.exitCode !== 0) throw new Error(`one ${name} failed`)
  return JSON.parse(ran.stdout.toString())
}

/// The key of whoever `one` acts as, or of `subject`, opened from this device's copy, with whose it is.
async function held(subject?: string) {
  const holds = tool("encryption key", subject)
  if (!holds.slot) throw new Error(`This device doesn't hold ${holds.owner}'s key.`)
  const key = await fromDevice(new Uint8Array(readFileSync(join(home, "key"))), holds.owner, holds.device, holds.slot)
  return { key, owner: holds.owner as string, holds }
}

if (command === "wrap" && scope) {
  const [mine, theirs] = [await held(), await held(scope)]
  tool("encryption store", scope, { wrapped: await wrap(theirs.key, mine.key, mine.owner, scope) })
  console.log(`${scope}'s key is encrypted to ${mine.owner}'s.`)
} else if (command === "servers") {
  if (!process.env.ONE_AS) throw new Error("Only on a network being built, with ONE_AS: on any other, a passkey hands keys over.")
  const { key, holds } = await held(scope)
  type Server = { key: string }
  const waiting = holds.access.filter((artifact: { missing: Server[]; extra: Server[] }) => artifact.missing.length || artifact.extra.length)
  const artifacts = []
  for (const { artifact, epoch, servers, missing, extra } of waiting) {
    const should = [...servers, ...missing].map((server: Server) => server.key).filter((server: string) => !extra.some((other: Server) => other.key === server))
    artifacts.push(await forServers(key, artifact, epoch, should))
  }
  if (artifacts.length) tool("encryption store", scope, { artifacts })
  console.log(`${artifacts.length} artifact${artifacts.length === 1 ? "" : "s"} of ${holds.owner}'s handed to the servers waiting.`)
} else {
  console.error("bun tooling/keys.ts wrap <scope> | servers [scope]")
  process.exit(2)
}
