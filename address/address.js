// Runs what sits at a network's address on this machine, with nothing deployed, or deploys it to Cloudflare:
//
//     bun address.js dev [what `wrangler dev` takes]
//     bun address.js deploy
//
// That's the pieces in this folder, and each plugin's part at the address: a plugin that has one holds an ordinary
// Worker in the folder `address` of its own (`plugins/<name>/address`), with a `wrangler.jsonc` and a
// `package.json` that names the first part of each path it answers (`"paths": ["mcp"]`). The gateway's list of
// paths is made here from those. A part is built in `parts/<name>`, a copy as deep in the repo as the folder it's
// of, so nothing that building leaves behind is among its plugin's files, and what it reaches in the repo by a
// path from itself it still reaches. It's given three bindings beside its own:
//
// - `OWN` writes and removes blobs of its plugin's own artifact for the network, where its plugin's servers read.
// - `NETWORK` says who the network's servers are, and whether a key is one of the network's own.
// - `RELAY` passes a call to a server and gives its answer, or tells every server something.
import { cpSync, mkdirSync, rmSync } from "node:fs";
import { basename, dirname } from "node:path";
import { $, Glob } from "bun";

const here = import.meta.dir;
const [what, ...rest] = process.argv.slice(2);
if (!["dev", "deploy"].includes(what)) throw new Error("bun address.js dev, or bun address.js deploy");

/** The pieces every address has, each by the first part of the paths it answers. The web client's files are the rest. */
const PIECES = {
  // iroh's own two paths: its relay protocol, and the probe that times how far away the relay is.
  relay: "RELAY",
  ping: "RELAY",
  signer: "SIGNER",
  link: "WAY_IN",
};
const services = [
  { binding: "RELAY", service: "one-relay" },
  { binding: "SIGNER", service: "one-signer" },
  { binding: "WAY_IN", service: "one-way-in" },
  { binding: "WEB", service: "one-web" },
];

// The web client's files have the kit among them, as every client has it: a page here is built against it, as
// an app is (`ui/app.js`), and finds it at `/ui/app/`.
rmSync(`${here}/web/files/ui`, { recursive: true, force: true });
cpSync(`${here}/../ui/dist/app`, `${here}/web/files/ui/app`, { recursive: true });

rmSync(`${here}/parts`, { recursive: true, force: true });
mkdirSync(`${here}/parts`);
const paths = { ...PIECES };
const parts = [];
for (const config of new Glob("*/address/wrangler.jsonc").scanSync(`${here}/../plugins`)) {
  const plugin = config.split("/")[0];
  const part = `${here}/parts/${plugin}`;
  cpSync(`${here}/../plugins/${dirname(config)}`, part, { recursive: true });
  const about = await Bun.file(`${part}/package.json`).json();
  await $`bun install --frozen-lockfile`.cwd(part).quiet();
  if (about.scripts?.build) await $`bun run build`.cwd(part).quiet();

  const worker = (await import(`${part}/wrangler.jsonc`)).default;
  const binding = `PART_${plugin.toUpperCase().replace(/-/g, "_")}`;
  for (const path of about.paths ?? []) {
    if (paths[path]) throw new Error(`${plugin} answers /${path}, which ${paths[path]} answers already`);
    paths[path] = binding;
  }
  services.push({ binding, service: worker.name });
  worker.services = [
    ...(worker.services ?? []),
    { binding: "OWN", service: "one-signer", entrypoint: "Own", props: { plugin } },
    { binding: "NETWORK", service: "one-signer", entrypoint: "Network" },
    { binding: "RELAY", service: "one-relay" },
  ];
  await Bun.write(`${part}/wrangler.jsonc`, JSON.stringify(worker, null, 2));
  parts.push(`parts/${plugin}/wrangler.jsonc`);
}
const gateway = { name: "one-gateway", main: "../gateway/gateway.js", compatibility_date: "2026-11-01", services, vars: { PATHS: paths } };
await Bun.write(`${here}/parts/gateway.jsonc`, JSON.stringify(gateway, null, 2));

const wrangler = (...args) => Bun.spawn(["bun", "x", "wrangler", ...args], { cwd: here, stdio: ["inherit", "inherit", "inherit"] }).exited;
const pieces = ["relay", "signer", "way-in", "web"].map((piece) => `${piece}/wrangler.jsonc`);
if (what === "dev") {
  process.exit(await wrangler("dev", ...["parts/gateway.jsonc", ...pieces, ...parts].flatMap((config) => ["-c", config]), ...rest));
}
// A Worker is deployed after the ones it's bound to, and the signer with its secrets, which only it holds.
for (const config of ["web/wrangler.jsonc", "signer/wrangler.jsonc", "relay/wrangler.jsonc", ...parts, "way-in/wrangler.jsonc", "parts/gateway.jsonc"]) {
  const secrets = basename(dirname(config)) === "signer" ? ["--secrets-file", "signer/secrets"] : [];
  const failed = await wrangler("deploy", "-c", config, ...secrets);
  if (failed) process.exit(failed);
}
