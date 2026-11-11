// Every one of auth's pages (pages.tsx's), as the auth Worker serves them (pages.js) but with no script of their own (only what moves them), in a custom
// build's look (its config.toml's name and logo), each an HTML file that opens as it is in a browser, to look at:
//
//   bun onboarding/render.js [config.toml] [folder]
//
// This repo's config.toml and a folder in /tmp unless given. It draws what @one/ui's build last drew: after changing
// pages.tsx or @one/ui, `bun run build` in src/screen/ui first.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { pages } from "./pages.js";

const repo = path.join(import.meta.dirname, "..");
const [config = path.join(repo, "config.toml"), out] = process.argv.slice(2);
const built = path.join(repo, "src", "screen", "ui", "dist");
const drawn = JSON.parse(readFileSync(path.join(built, "auth.json"), "utf8"));
const { page } = pages({ css: readFileSync(path.join(built, "auth.css"), "utf8"), ...drawn });

const { name, logo, join_domain = "" } = Bun.TOML.parse(readFileSync(config, "utf8"));
// What its users see their network called, as build.rs names it (config::YOURS).
const yours = join_domain ? `${name.replace("The ", "the ")} network` : `your ${name.replace(/^The /, "")} network`;
const look = { name, yours, logo: pathToFileURL(path.resolve(path.dirname(config), logo)).href, provider: "Google", providers: [{ id: "google", name: "Google" }], user: "Sam" };

const folder = out || path.join(tmpdir(), `auth-pages-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`);
mkdirSync(folder, { recursive: true });
for (const variant of Object.keys(drawn.pages)) writeFileSync(path.join(folder, `${variant}.html`), page(variant, look));
console.log(`${Object.keys(drawn.pages).length} pages in ${folder}`);
