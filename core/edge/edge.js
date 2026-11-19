// Runs a network's edge on this machine, with nothing deployed, or deploys it to Cloudflare:
//
//     bun edge.js dev [what `wrangler dev` takes]
//     bun edge.js deploy [https://<the network's address>]
//
// Deployed with the network's address, the gateway answers there: Cloudflare attaches that name to it, making its
// DNS record and certificate, if the name is on a zone of the same account. Without one it answers only at
// workers.dev.
//
// That's the pieces in this folder, the web client's files (`clients/web`), and each app's edge function: an app
// that has one holds an ordinary Worker in the folder `edge` of its own (`apps/<name>/edge`), with a
// `wrangler.jsonc` and a `package.json` that names the first part of each path it answers (`"paths": ["mcp"]`).
// The gateway's list of paths is made here from those. A part is built in `target/edge-dev/<name>` or
// `target/edge-deploy/<name>`, a copy as deep in the repo as the folder it's of, so nothing that building leaves behind
// is among its app's files, and what it reaches in the repo by a path from itself it still reaches. A deploy never
// rewrites what a local network's edge runs from. It's given three bindings beside its own:
//
// - `OWN` writes and removes blobs of its app's own artifact for the network, where its app's servers read.
// - `NETWORK` says who the network's servers are, and whether a key is one of the network's own.
// - `RELAY` passes a call to a server and gives its answer, or tells every server something.
//
// A part whose `package.json` says `"who": true` is the one that signs people in: its Worker exports an entrypoint
// `Who`, which the network's MCP endpoint asks who a request is from (`mcp/mcp.ts`). One part at most says so.
//
// A part whose `package.json` says `"tail": true` is a Tail Worker: every other Worker here that handles requests
// for the network is given it as a tail consumer, so it hears what each did. The gateway only passes each on, and
// the web client's are files; the presigner isn't either, since such a part is bound to it and so is deployed after.
import { createPrivateKey, sign } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync, symlinkSync } from "node:fs";
import { basename, dirname, relative, resolve } from "node:path";
import { $, Glob } from "bun";

const here = import.meta.dir;
const root = resolve(here, "../..");
const [what, ...rest] = process.argv.slice(2);
if (!["dev", "deploy"].includes(what)) throw new Error("bun edge.js dev, or bun edge.js deploy [https://<address>]");
const at = what === "deploy" && rest[0] ? new URL(rest[0]) : null;
if (at && (at.protocol !== "https:" || at.pathname !== "/")) throw new Error("A network's address is https://<name>, with no path");
// Wrangler reads the Cloudflare token from the environment: CLOUDFLARE_API_TOKEN when it's set, else 1Password's, read
// into this process only.
if (what === "deploy" && !process.env.CLOUDFLARE_API_TOKEN) {
  const read = await $`op read ${"op://van-tonder-nosudo/CLOUDFLARE_API_TOKEN/credential"}`.quiet().nothrow();
  if (read.exitCode !== 0 || !read.text().trim()) throw new Error("No Cloudflare token: set CLOUDFLARE_API_TOKEN, or sign in to 1Password (`op`) for van-tonder-nosudo/CLOUDFLARE_API_TOKEN");
  process.env.CLOUDFLARE_API_TOKEN = read.text().trim();
}

/** The pieces every edge has, each by the first part of the paths it answers. The web client's files are the rest. */
const PIECES = {
  // iroh's own two paths: its relay protocol, and the probe that times how far away the relay is.
  relay: "RELAY",
  ping: "RELAY",
  presigner: "PRESIGNER",
  // The network's front door.
  mcp: "MCP",
  "llms.txt": "MCP",
  skills: "MCP",
  apps: "MCP",
  link: "MCP",
};
const services = [
  { binding: "RELAY", service: "network-relay" },
  { binding: "PRESIGNER", service: "network-presigner" },
  { binding: "MCP", service: "network-mcp" },
  { binding: "WEB", service: "network-web" },
];

const built = `${root}/target/edge-${what}`;
rmSync(built, { recursive: true, force: true });
mkdirSync(built, { recursive: true });

// The web client is the app's own page, the files its window is given (`clients/app/tauri.conf.json`), with the
// web's files over them (`clients/web`), which answer what the page asks in a browser in place of the app's Rust half,
// and the UI library, which an app's page finds at `/ui/client/`. The piece `web` serves them from `<built>/web`. A
// deploy's are copies; a local network's are links to where each is made (a `cp -s` farm), so it serves the page as
// it is now, and the UI library as its code reloader last built it, through a rebase and a deploy alike.
const web = `${built}/web`;
const app = `${root}/clients/app`;
const pageFiles = (await Bun.file(`${app}/tauri.conf.json`).json()).build.frontendDist;
const place = (/** @type {string} */ from, /** @type {string} */ to) => (what === "dev" ? symlinkSync(from, to) : cpSync(from, to, { recursive: true }));
mkdirSync(`${web}/ui`, { recursive: true });
for (const file of pageFiles) place(`${app}/${file}`, `${web}/${basename(file)}`);
for (const file of readdirSync(`${root}/clients/web`).filter((file) => !["dist", "node_modules", "renderer"].includes(file))) {
  rmSync(`${web}/${file}`, { force: true });
  place(`${root}/clients/web/${file}`, `${web}/${file}`);
}
// Its service worker is built, with what it imports, over its source.
rmSync(`${web}/sw.js`, { force: true });
await $`bun install --frozen-lockfile && bun build sw.js --format iife --minify --outfile ${web}/sw.js`.cwd(`${root}/clients/web`).quiet();
// The renderer's frame, with the renderer built for it as WebAssembly, built here as the UI library is.
await $`bun build.js ${web}/renderer`.cwd(`${root}/clients/web/renderer`).quiet();
// Built here, so a deploy never ships whatever build of the UI library was left on disk.
await $`bun install --frozen-lockfile && bun build.js`.cwd(`${root}/core/ui`).quiet();
place(`${root}/core/ui/dist/client`, `${web}/ui/client`);
// The app's own page, as its window has it, at `/page/`, which the app keeps and loads from its next start if it's
// newer than its own and it runs on it (`clients/app/src/page.rs`), with what's said of it in `update.json`: its
// version, the lowest build of the app it runs on (`clients/app/page/update.json`), and each file's SHA-256. As
// Tauri's updater and Sparkle do, the app takes it only with `update.json.sig`, the platform's Ed25519 signature of
// `update.json`, whose public half it has built in, so neither whoever runs the network's edge nor anything in front
// of it can give the app a page of their own. PAGE_SIGNING_KEY is its private half, a PKCS#8 DER in base64 (in
// 1Password, "The One App page signing key"), read from there when the variable isn't set; without a key the deploy
// refuses, as an unsigned page is one apps refuse.
const version = (await $`sh ${root}/tooling/version`.text()).trim();
{
  const files = {};
  for (const file of pageFiles) {
    cpSync(`${app}/${file}`, `${web}/page/${basename(file)}`);
    files[basename(file)] = new Bun.CryptoHasher("sha256").update(await Bun.file(`${app}/${file}`).bytes()).digest("hex");
  }
  const update = { version, ...(await Bun.file(`${app}/page/update.json`).json()), files };
  const said = Buffer.from(JSON.stringify(update));
  await Bun.write(`${web}/page/update.json`, said);
  const pageKey = process.env.PAGE_SIGNING_KEY || (await $`op read ${"op://van-tonder-nosudo/The One App page signing key/credential"}`.text()).trim();
  const key = createPrivateKey({ key: Buffer.from(pageKey, "base64"), format: "der", type: "pkcs8" });
  await Bun.write(`${web}/page/update.json.sig`, sign(null, said, key).toString("base64"));
}

const paths = { ...PIECES };
const parts = [];
const workers = [];
let who = null;
for (const config of new Glob("*/edge/wrangler.jsonc").scanSync(`${root}/apps`)) {
  const app = config.split("/")[0];
  const part = `${built}/${app}`;
  cpSync(`${root}/apps/${dirname(config)}`, part, { recursive: true });
  const about = await Bun.file(`${part}/package.json`).json();
  await $`bun install --frozen-lockfile`.cwd(part).quiet();
  if (about.scripts?.build) await $`bun run build`.cwd(part).quiet();

  const worker = (await import(`${part}/wrangler.jsonc`)).default;
  const binding = `PART_${app.toUpperCase().replace(/-/g, "_")}`;
  for (const path of about.paths ?? []) {
    if (paths[path]) throw new Error(`${app} answers /${path}, which ${paths[path]} answers already`);
    paths[path] = binding;
  }
  if (about.who) {
    if (who) throw new Error(`${app} says who a request is from, which ${who.app} says already`);
    who = { app, binding: "WHO", service: worker.name, entrypoint: "Who" };
  }
  services.push({ binding, service: worker.name });
  worker.services = [
    ...(worker.services ?? []),
    { binding: "OWN", service: "network-presigner", entrypoint: "Own", props: { app } },
    { binding: "NETWORK", service: "network-presigner", entrypoint: "Network" },
    { binding: "RELAY", service: "network-relay" },
  ];
  workers.push({ part, worker, tail: about.tail === true });
}
const tails = workers.filter(({ tail }) => tail).map(({ worker }) => ({ service: worker.name }));
const tailed = (worker) => (tails.length ? { ...worker, tail_consumers: tails } : worker);
// The tail parts go first, since the rest name them.
for (const { part, worker, tail } of workers.sort((a, b) => b.tail - a.tail)) {
  await Bun.write(`${part}/wrangler.jsonc`, JSON.stringify(tail ? worker : tailed(worker), null, 2));
  parts.push(relative(here, `${part}/wrangler.jsonc`));
}
// A request's signal is aborted when its client goes away, and passed on with it, so the part it's for hears so
// (`mcp/mcp.ts`).
const gateway = {
  name: "network-gateway",
  main: relative(built, `${here}/gateway/gateway.ts`),
  compatibility_date: "2026-11-01",
  compatibility_flags: ["enable_request_signal"],
  services,
  vars: { PATHS: paths },
};
if (at) gateway.routes = [{ pattern: at.hostname, custom_domain: true }];
await Bun.write(`${built}/gateway.jsonc`, JSON.stringify(gateway, null, 2));
const gatewayConfig = relative(here, `${built}/gateway.jsonc`);

const wrangler = (...args) => Bun.spawn(["bun", "x", "wrangler", ...args], { cwd: here, env: process.env, stdio: ["inherit", "inherit", "inherit"] }).exited;
const webWorker = (await import(`${here}/web/wrangler.jsonc`)).default;
await Bun.write(`${built}/web.jsonc`, JSON.stringify({ ...webWorker, assets: { directory: "web" } }, null, 2));
const config = {
  web: relative(here, `${built}/web.jsonc`),
  presigner: "presigner/wrangler.jsonc",
};
// The network's updater, which the relay is bound to, the relay and the MCP endpoint are told their tail parts in a
// copy of their config beside the gateway's; the updater and the relay the edge's build, by which the relay knows which
// servers talk with it (`relay/relay.ts`); and the MCP endpoint the part that says who a request is from.
for (const piece of ["updater", "relay", "mcp"]) {
  const worker = (await import(`${here}/${piece}/wrangler.jsonc`)).default;
  if (piece === "mcp" && who) worker.services = [...worker.services, { binding: who.binding, service: who.service, entrypoint: who.entrypoint }];
  if (piece !== "mcp") worker.vars = { ...worker.vars, VERSION: version };
  await Bun.write(`${built}/${piece}.jsonc`, JSON.stringify(tailed({ ...worker, main: relative(built, `${here}/${piece}/${worker.main}`) }), null, 2));
  config[piece] = relative(here, `${built}/${piece}.jsonc`);
}
const pieces = Object.values(config);
// A part's D1 databases are made what they should be by their migrations (`migrations_dir`), as Wrangler applies
// them, never as the part answers a request: the local ones before `dev` starts, where it keeps them, and Cloudflare's
// just after the part is deployed, since Wrangler makes a database a part names on its first deploy, and refuses
// migrations for one that isn't there yet.
const migrate = async (/** @type {string} */ deployed, /** @type {string[]} */ where) => {
  const { worker } = workers.find(({ part }) => relative(here, `${part}/wrangler.jsonc`) === deployed) ?? {};
  for (const { binding, migrations_dir } of worker?.d1_databases ?? []) {
    const failed = migrations_dir && (await wrangler("d1", "migrations", "apply", binding, "-c", deployed, ...where));
    if (failed) process.exit(failed);
  }
};
if (what === "dev") {
  const kept = rest.indexOf("--persist-to");
  for (const part of parts) await migrate(part, ["--local", ...(kept >= 0 ? rest.slice(kept, kept + 2) : [])]);
  process.exit(await wrangler("dev", ...[gatewayConfig, ...pieces, ...parts].flatMap((config) => ["-c", config]), ...rest));
}
// A Worker is deployed after the ones it's bound to, each with its secrets if it has any: a file `secrets` beside
// its `wrangler.jsonc`, which only it holds, such as the presigner's and an app's part's keys to an outside service.
// The tail parts come first among the parts, before whatever names them.
const [tailParts, otherParts] = [parts.slice(0, tails.length), parts.slice(tails.length)];
for (const deployed of [config.web, config.presigner, ...tailParts, config.updater, config.relay, ...otherParts, config.mcp, gatewayConfig]) {
  const file = `${dirname(deployed)}/secrets`;
  const secrets = existsSync(`${here}/${file}`) ? ["--secrets-file", file] : [];
  const failed = await wrangler("deploy", "-c", deployed, ...secrets);
  if (failed) process.exit(failed);
  await migrate(deployed, ["--remote"]);
}
