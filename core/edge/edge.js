// Runs a network's edge on this machine, with nothing deployed, or deploys it to Cloudflare:
//
//     bun edge.js dev [what `wrangler dev` takes]
//     bun edge.js deploy [https://<the network's address>]
//
// Deployed with the network's address, the edge answers there: Cloudflare attaches that name to it, making its DNS
// record and certificate, if the name is on a zone of the same account. Without one it answers only at workers.dev.
//
// That's the edge's Worker (`wrangler.jsonc`, `edge.ts`), with the web client's files (`clients/web`) as its assets,
// the relay's (`relay/wrangler.jsonc`, `relay/worker.ts`), and each app's edge function: an app that has one holds an
// ordinary Worker in the folder `edge` of its own (`apps/<name>/edge`), with a `wrangler.jsonc` and a `package.json` that names the first part of each path it
// answers (`"paths": ["mcp"]`). The edge's list of them is made here. A part is built in `target/edge-dev/<name>` or
// `target/edge-deploy/<name>`, a copy as deep in the repo as the folder it's of, so nothing that building leaves behind
// is among its app's files, and what it reaches in the repo by a path from itself it still reaches. A deploy never
// rewrites what a local network's edge runs from. It's given three bindings to the edge beside its own:
//
// - `OWN` writes and removes blobs of its app's own artifact for the network, where its app's servers read.
// - `NETWORK` says who the network's servers are, and whether a key is one of the network's own.
// - `RELAY` passes a call to a server and gives its answer, or tells every server something.
//
// A part whose `package.json` says `"who": true` is the one that signs people in: its Worker exports an entrypoint
// `Who`, which the network's MCP endpoint asks who a request is from (`mcp/mcp.ts`). One part at most says so.
//
// A part whose `package.json` says `"tail": true` is a Tail Worker: the edge and every other part are given it as a
// tail consumer, so it hears what each did.
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

/** The first part of each path a piece of the edge answers (`edge.ts`). The web client's files are the rest. */
const PIECES = await Bun.file(`${here}/paths.json`).json();

const built = `${root}/target/edge-${what}`;
rmSync(built, { recursive: true, force: true });
mkdirSync(built, { recursive: true });

// The web client is the app's own page, the files its window is given (`clients/app/tauri.conf.json`), with the
// web's files over them (`clients/web`), which answer what the page asks in a browser in place of the app's Rust half,
// and the UI library, which an app's page finds at `/ui/client/`. The edge serves them from `<built>/web`. A
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
// Built here, so a deploy never ships whatever build of the UI library was left on disk.
await $`bun install --frozen-lockfile && bun build.js`.cwd(`${root}/core/ui`).quiet();
place(`${root}/core/ui/dist/client`, `${web}/ui/client`);
// The renderer's frame, with the renderer built for it as WebAssembly, and its check page, built with the UI library.
await $`bun build.js ${web}/renderer`.cwd(`${root}/clients/web/renderer`).quiet();
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

const edge = (await import(`${here}/wrangler.jsonc`)).default;
const paths = { ...PIECES };
const parts = {};
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
    paths[path] = parts[path] = binding;
  }
  if (about.who) {
    if (who) throw new Error(`${app} says who a request is from, which ${who.app} says already`);
    who = { app, binding: "WHO", service: worker.name, entrypoint: "Who" };
  }
  worker.services = [
    ...(worker.services ?? []),
    { binding: "OWN", service: edge.name, entrypoint: "Own", props: { app } },
    { binding: "NETWORK", service: edge.name, entrypoint: "Network" },
    { binding: "RELAY", service: edge.name, entrypoint: "RelayEntrypoint" },
  ];
  workers.push({ app, binding, part, worker, tail: about.tail === true });
}
const tails = workers.filter(({ tail }) => tail).map(({ worker }) => ({ service: worker.name }));
const tailed = (worker) => (tails.length ? { ...worker, tail_consumers: tails } : worker);
// The tail parts go first, since the rest name them.
for (const { part, worker, tail } of workers.sort((a, b) => b.tail - a.tail)) {
  await Bun.write(`${part}/wrangler.jsonc`, JSON.stringify(tail ? worker : tailed(worker), null, 2));
}

// The edge, with the web client's files as its assets, which are sent without the Worker but for the paths it answers
// and the renderer's files, which it sends as they're kept compressed (`web/web.ts`); its build, by which the relay knows
// which servers talk with it (`relay/relay.ts`); and which part answers which path, and the part that says who a
// request is from, bound.
const own = {
  ...edge,
  main: relative(built, `${here}/${edge.main}`),
  assets: { directory: "web", binding: "ASSETS", run_worker_first: [...Object.keys(paths).flatMap((path) => [`/${path}`, `/${path}/*`]), "/renderer/*"] },
  vars: { VERSION: version, PARTS: parts },
};
const full = tailed({
  ...own,
  services: [...edge.services, ...workers.map(({ binding, worker }) => ({ binding, service: worker.name })), ...(who ? [who] : []).map(({ binding, service, entrypoint }) => ({ binding, service, entrypoint }))],
  ...(at && { routes: [{ pattern: at.hostname, custom_domain: true }] }),
});
await Bun.write(`${built}/edge.jsonc`, JSON.stringify(full, null, 2));
const edgeConfig = relative(here, `${built}/edge.jsonc`);
const partConfigs = workers.map(({ part }) => relative(here, `${part}/wrangler.jsonc`));
// The relay, a Worker of its own (`relay/worker.ts`), which the edge passes its paths to.
const relayWorker = (await import(`${here}/relay/wrangler.jsonc`)).default;
const relayed = tailed({ ...relayWorker, main: relative(built, `${here}/relay/${relayWorker.main}`) });
await Bun.write(`${built}/relay.jsonc`, JSON.stringify(relayed, null, 2));
const relayConfig = relative(here, `${built}/relay.jsonc`);

const wrangler = (...args) => Bun.spawn(["bun", "x", "wrangler", ...args], { cwd: here, env: process.env, stdio: ["inherit", "inherit", "inherit"] }).exited;
// A part's D1 databases are made what they should be by their migrations (`migrations_dir`), as Wrangler applies
// them, never as the part answers a request: the local ones before `dev` starts, where it keeps them, and Cloudflare's
// just after the part is deployed, since Wrangler makes a database a part names on its first deploy, and refuses
// migrations for one that isn't there yet.
const migrate = async (/** @type {{worker: any}} */ { worker }, /** @type {string} */ deployed, /** @type {string[]} */ where) => {
  for (const { binding, migrations_dir } of worker.d1_databases ?? []) {
    const failed = migrations_dir && (await wrangler("d1", "migrations", "apply", binding, "-c", deployed, ...where));
    if (failed) process.exit(failed);
  }
};
if (what === "dev") {
  const kept = rest.indexOf("--persist-to");
  for (const [at, part] of workers.entries()) await migrate(part, partConfigs[at], ["--local", ...(kept >= 0 ? rest.slice(kept, kept + 2) : [])]);
  process.exit(await wrangler("dev", ...[edgeConfig, relayConfig, ...partConfigs].flatMap((config) => ["-c", config]), ...rest));
}
// A Worker is deployed after the ones it's bound to, each with its secrets if it has any: a file `secrets` beside
// its `wrangler.jsonc`, which only it holds, such as the edge's keys to the bucket and an app's part's to an outside
// service. The edge, the relay and the parts are bound to each other, and Cloudflare refuses a binding to a Worker that
// isn't there yet, so on an account that has no edge yet it's deployed first without its bindings to the others and
// its address.
const deploy = async (/** @type {string} */ config, /** @type {string} */ secrets, /** @type {string[]} */ ...more) => {
  const failed = await wrangler("deploy", "-c", config, ...(existsSync(secrets) ? ["--secrets-file", relative(here, secrets)] : []), ...more);
  if (failed) process.exit(failed);
};
if ((await $`bun x wrangler deployments list --name ${edge.name}`.cwd(here).quiet().nothrow()).exitCode !== 0) {
  const bindings = edge.durable_objects.bindings.filter(({ script_name }) => !script_name);
  await Bun.write(`${built}/bare.jsonc`, JSON.stringify({ ...own, services: [], durable_objects: { ...edge.durable_objects, bindings } }, null, 2));
  await deploy(relative(here, `${built}/bare.jsonc`), `${here}/secrets`);
}
// A part is deployed, then its databases migrated. The tail parts go before the relay and the other parts, which name
// them as their tail consumers, as Cloudflare refuses one that isn't there yet.
const deployPart = async (/** @type {number} */ at) => {
  await deploy(partConfigs[at], `${root}/apps/${workers[at].app}/edge/secrets`);
  await migrate(workers[at], partConfigs[at], ["--remote"]);
};
for (const [at, { tail }] of workers.entries()) if (tail) await deployPart(at);
// The relay is deployed only when what it runs changed: a deploy ends every connection it holds. Its version is tagged
// with a hash of its bundle and config, as the build Wrangler would upload, and one with the same tag isn't deployed again.
{
  const out = `${built}/relay-bundle`;
  await $`bun x wrangler deploy --dry-run --outdir ${out} -c ${relayConfig}`.cwd(here).quiet();
  const hash = new Bun.CryptoHasher("sha256").update(JSON.stringify(relayed));
  // Its modules, not the source map or the README Wrangler writes beside them, which says when it was built.
  for (const file of [...new Glob("**/*").scanSync(out)].filter((file) => !/(\.map|README\.md)$/.test(file)).sort()) hash.update(file).update(await Bun.file(`${out}/${file}`).bytes());
  const tag = hash.digest("hex").slice(0, 24);
  const status = await $`bun x wrangler deployments status --name ${relayWorker.name} --json`.cwd(here).quiet().nothrow();
  const live = status.exitCode === 0 && JSON.parse(status.text()).versions?.[0]?.version_id;
  const deployed = live && JSON.parse((await $`bun x wrangler versions view ${live} --name ${relayWorker.name} --json`.cwd(here).quiet()).text()).annotations?.["workers/tag"];
  if (deployed === tag) console.log(`The relay is as deployed (${tag}), so it's left running`);
  else await deploy(relayConfig, `${here}/relay/secrets`, "--tag", tag);
}
for (const [at, { tail }] of workers.entries()) if (!tail) await deployPart(at);
await deploy(edgeConfig, `${here}/secrets`);
