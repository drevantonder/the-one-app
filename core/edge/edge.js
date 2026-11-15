// Runs a network's edge on this machine, with nothing deployed, or deploys it to Cloudflare:
//
//     bun edge.js dev [what `wrangler dev` takes]
//     bun edge.js deploy [https://<the network's address>]
//
// Deployed with the network's address, the gateway answers there: Cloudflare attaches that name to it, making its
// DNS record and certificate, if the name is on a zone of the same account. Without one it answers only at
// workers.dev.
//
// That's the pieces in this folder, the web client's files (`clients/web`), and each plugin's edge part: a plugin
// that has one holds an ordinary Worker in the folder `edge` of its own (`plugins/<name>/edge`), with a
// `wrangler.jsonc` and a `package.json` that names the first part of each path it answers (`"paths": ["mcp"]`).
// The gateway's list of paths is made here from those. A part is built in `target/edge/<name>`, a copy as deep in
// the repo as the folder it's of, so nothing that building leaves behind is among its plugin's files, and what it
// reaches in the repo by a path from itself it still reaches. It's given three bindings beside its own:
//
// - `OWN` writes and removes blobs of its plugin's own artifact for the network, where its plugin's servers read.
// - `NETWORK` says who the network's servers are, and whether a key is one of the network's own.
// - `RELAY` passes a call to a server and gives its answer, or tells every server something.
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { $, Glob } from "bun";

const here = import.meta.dir;
const root = resolve(here, "../..");
const [what, ...rest] = process.argv.slice(2);
if (!["dev", "deploy"].includes(what)) throw new Error("bun edge.js dev, or bun edge.js deploy [https://<address>]");
const at = what === "deploy" && rest[0] ? new URL(rest[0]) : null;
if (at && (at.protocol !== "https:" || at.pathname !== "/")) throw new Error("A network's address is https://<name>, with no path");

/** The pieces every edge has, each by the first part of the paths it answers. The web client's files are the rest. */
const PIECES = {
  // iroh's own two paths: its relay protocol, and the probe that times how far away the relay is.
  relay: "RELAY",
  ping: "RELAY",
  signer: "SIGNER",
  link: "WAY_IN",
};
const services = [
  { binding: "RELAY", service: "network-relay" },
  { binding: "SIGNER", service: "network-signer" },
  { binding: "WAY_IN", service: "network-way-in" },
  { binding: "WEB", service: "network-web" },
];

// The web client is its files with the kit among them, as every client has it: a page here is built against it, as
// an app is (`ui/app.js`), and finds it at `/ui/client/`. The piece `web` serves what's built, `clients/web/dist`.
const web = `${root}/clients/web`;
rmSync(`${web}/dist`, { recursive: true, force: true });
for (const file of readdirSync(web).filter((file) => file !== "dist")) cpSync(`${web}/${file}`, `${web}/dist/${file}`, { recursive: true });
cpSync(`${root}/ui/dist/client`, `${web}/dist/ui/client`, { recursive: true });

const built = `${root}/target/edge`;
rmSync(built, { recursive: true, force: true });
mkdirSync(built, { recursive: true });
const paths = { ...PIECES };
const parts = [];
for (const config of new Glob("*/edge/wrangler.jsonc").scanSync(`${root}/plugins`)) {
  const plugin = config.split("/")[0];
  const part = `${built}/${plugin}`;
  cpSync(`${root}/plugins/${dirname(config)}`, part, { recursive: true });
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
    { binding: "OWN", service: "network-signer", entrypoint: "Own", props: { plugin } },
    { binding: "NETWORK", service: "network-signer", entrypoint: "Network" },
    { binding: "RELAY", service: "network-relay" },
  ];
  await Bun.write(`${part}/wrangler.jsonc`, JSON.stringify(worker, null, 2));
  parts.push(relative(here, `${part}/wrangler.jsonc`));
}
const gateway = { name: "network-gateway", main: relative(built, `${here}/gateway/gateway.js`), compatibility_date: "2026-11-01", services, vars: { PATHS: paths } };
if (at) gateway.routes = [{ pattern: at.hostname, custom_domain: true }];
await Bun.write(`${built}/gateway.jsonc`, JSON.stringify(gateway, null, 2));
const gatewayConfig = relative(here, `${built}/gateway.jsonc`);

const wrangler = (...args) => Bun.spawn(["bun", "x", "wrangler", ...args], { cwd: here, stdio: ["inherit", "inherit", "inherit"] }).exited;
const pieces = ["relay", "signer", "way-in", "web"].map((piece) => `${piece}/wrangler.jsonc`);
if (what === "dev") {
  process.exit(await wrangler("dev", ...[gatewayConfig, ...pieces, ...parts].flatMap((config) => ["-c", config]), ...rest));
}
// A Worker is deployed after the ones it's bound to, each with its secrets if it has any: a file `secrets` beside
// its `wrangler.jsonc`, which only it holds, such as the signer's and a plugin's part's keys to an outside service.
for (const config of ["web/wrangler.jsonc", "signer/wrangler.jsonc", "relay/wrangler.jsonc", ...parts, "way-in/wrangler.jsonc", gatewayConfig]) {
  const file = `${dirname(config)}/secrets`;
  const secrets = existsSync(`${here}/${file}`) ? ["--secrets-file", file] : [];
  const failed = await wrangler("deploy", "-c", config, ...secrets);
  if (failed) process.exit(failed);
}
