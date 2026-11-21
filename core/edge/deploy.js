// Deploys a built edge (`bun edge.js build <folder>`, which copies this in beside it) to a network's address, checks
// it answers as that build, and rolls each Worker back to the version it ran before if it doesn't:
//
//     bun install --frozen-lockfile && bun deploy.js https://<the network's address>
//
// with the network's Cloudflare token in CLOUDFLARE_API_TOKEN. It needs nothing of the repo: each Worker is bundled
// already (`workers/<name>/`), as Wrangler bundles it, and is deployed as it is, with the web client's files
// (`web/`). A network's servers each have the edge of the build they run (`server-code`'s `edge/`), and one deploys
// it as the network's update asks (`core/edge/updater/updater.ts`).
import { existsSync } from "node:fs";
import { $ } from "bun";

const here = import.meta.dir;
const at = new URL(process.argv[2] ?? "");
if (at.protocol !== "https:" || at.pathname !== "/") throw new Error("bun deploy.js https://<address>: a network's address, with no path");
if (!process.env.CLOUDFLARE_API_TOKEN) throw new Error("No Cloudflare token: CLOUDFLARE_API_TOKEN isn't set");

// Wrangler deploys to the account of the zone the address is on, as Cloudflare says to the token, and nothing is
// deployed where the token can't see the zone: a token taken for another network's, as Andre's was the only one read,
// once deployed one network's Workers in place of another's (20 Nov).
const labels = at.hostname.split(".");
let zone;
for (let from = 0; !zone && from < labels.length - 1; from++) {
  const asked = await fetch(`https://api.cloudflare.com/client/v4/zones?name=${labels.slice(from).join(".")}`, { headers: { authorization: `Bearer ${process.env.CLOUDFLARE_API_TOKEN}` } });
  zone = asked.ok ? (await asked.json()).result?.[0] : undefined;
}
if (!zone) throw new Error(`The Cloudflare token for ${at.hostname} reaches no zone it's on, so nothing was deployed: it's another network's token, or it can't read the zone`);
process.env.CLOUDFLARE_ACCOUNT_ID = zone.account.id;
console.log(`Deploying to ${zone.account.name}, the account of ${zone.name}`);

/**
 * The Workers, in the order they're deployed, as `edge.js` built them: a Worker after the ones it's bound to, so the
 * tail parts first, which the rest name as their tail consumers, then the relay, the other parts and the edge.
 * @type {{version: string, workers: {name: string, kind: "tail" | "relay" | "part" | "edge", tag: string, d1: string[]}[]}}
 */
const built = await Bun.file(`${here}/workers.json`).json();
const wrangler = (/** @type {string[]} */ ...args) => Bun.spawn(["bun", "x", "wrangler", ...args], { cwd: here, env: process.env, stdio: ["inherit", "inherit", "inherit"] }).exited;
const config = (/** @type {string} */ name, file = "wrangler.jsonc") => `workers/${name}/${file}`;

/** The version of `name` that's live, with the tag it was deployed with, or `null` where there's none. */
async function live(/** @type {string} */ name) {
  const status = await $`bun x wrangler deployments status --name ${name} --json`.cwd(here).quiet().nothrow();
  const version = status.exitCode === 0 ? JSON.parse(status.text()).versions?.[0]?.version_id : null;
  if (!version) return null;
  const viewed = JSON.parse((await $`bun x wrangler versions view ${version} --name ${name} --json`.cwd(here).quiet()).text());
  return { version, tag: viewed.annotations?.["workers/tag"] };
}

// The edge answers at the network's address, which Cloudflare attaches to it, making its DNS record and certificate.
const edge = built.workers.find(({ kind }) => kind === "edge");
const edgeConfig = await Bun.file(`${here}/${config(edge.name)}`).json();
await Bun.write(`${here}/${config(edge.name, "deployed.jsonc")}`, JSON.stringify({ ...edgeConfig, routes: [{ pattern: at.hostname, custom_domain: true }] }, null, 2));

const before = Object.fromEntries(await Promise.all(built.workers.map(async ({ name }) => [name, await live(name)])));
/** The Workers deployed here, which are rolled back if the edge doesn't answer as its build. */
const deployed = [];
const deploy = async (/** @type {string} */ name, /** @type {string} */ file, /** @type {string[]} */ ...more) => {
  // A Worker's secrets are kept between deploys; a network's first deploy is given them by a file `secrets` beside it.
  const secrets = `workers/${name}/secrets`;
  const failed = await wrangler("deploy", "-c", config(name, file), ...(existsSync(`${here}/${secrets}`) ? ["--secrets-file", secrets] : []), ...more);
  if (failed) await back(`${name} didn't deploy`);
  deployed.push(name);
};

// The edge, the relay and the parts are bound to each other, and Cloudflare refuses a binding to a Worker that isn't
// there yet, so on an account that has no edge yet it's deployed first without its bindings to the others or its address.
if (!before[edge.name]) await deploy(edge.name, "bare.jsonc");
for (const { name, kind, tag, d1 } of built.workers) {
  if (kind === "edge") await deploy(name, "deployed.jsonc");
  // The relay is deployed only when what it runs changed: a deploy ends every connection it holds.
  else if (kind === "relay" && before[name]?.tag === tag) console.log(`The relay is as deployed (${tag}), so it's left running`);
  else await deploy(name, "wrangler.jsonc", "--tag", tag);
  // A part's D1 databases are made what they should be by their migrations, as Wrangler applies them, just after it's
  // deployed, since Wrangler makes a database a part names on its first deploy, and refuses migrations for one that
  // isn't there yet.
  for (const binding of d1) if (await wrangler("d1", "migrations", "apply", binding, "-c", config(name), "--remote")) await back(`${name}'s ${binding} didn't migrate`);
}

// It's checked as a person's app finds it: the window page's `update.json` says the build it was deployed from, and the
// web client's page loads. Cloudflare takes a few seconds to answer with a new version everywhere, so it's asked again
// for up to a minute.
let why = "";
for (const until = Date.now() + 60_000; Date.now() < until; await Bun.sleep(5_000)) {
  const said = await fetch(new URL("/page/update.json", at), { cache: "no-store" }).then((answer) => (answer.ok ? answer.json() : null), () => null);
  const page = await fetch(at, { redirect: "follow", cache: "no-store" }).then((answer) => answer.status, () => 0);
  why = said?.version !== built.version ? `it says it's ${said?.version ?? "nothing"}, not ${built.version}` : page >= 400 || page === 0 ? `its page answered ${page}` : "";
  if (!why) break;
}
if (why) await back(`The edge at ${at.host} didn't answer as ${built.version}: ${why}`);
console.log(`The edge at ${at.host} is ${built.version}`);

/** Rolls each Worker deployed here back to the version it ran before, and ends saying `failed`. */
async function back(/** @type {string} */ failed) {
  console.error(failed);
  for (const name of deployed.reverse()) {
    if (!before[name]) continue;
    console.error(`Rolling ${name} back to ${before[name].version}`);
    await wrangler("rollback", before[name].version, "--name", name, "--message", failed.slice(0, 100), "--yes");
  }
  process.exit(1);
}
