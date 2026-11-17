// The SQL that lists the apps of a repo's apps/ folder: what each one's SKILL.md says, and what its code says
// as a network's Marketplace lists it (`apps marketplace`), where it lists it:
//     one apps marketplace | bun seed.js <owner>/<repo> <its apps/ folder> > seed.sql
import { Glob } from "bun";
import { frontMatter, listing } from "./worker.ts";

const [source, folder] = process.argv.slice(2);
const [owner, repo] = source.toLowerCase().split("/");
// The network's own over a copy someone in it deployed, where there are both.
const apps = (await Bun.stdin.json()).apps.filter((app) => app.app).sort((a, b) => a.app.startsWith("network/") - b.app.startsWith("network/"));
const store = new Map(apps.map((app) => [app.app.split("/").pop(), app]));
const quote = (text) => (text == null ? "NULL" : `'${text.replaceAll("'", "''")}'`);
const now = new Date().toISOString();

for (const skill of [...new Glob("*/SKILL.md").scanSync(folder)].sort()) {
  const { name, description, subtitle, about, category } = frontMatter(await Bun.file(`${folder}/${skill}`).text());
  const listed = listing(store.get(name));
  const values = [owner, repo, name, description, subtitle, about, category, listed && JSON.stringify(listed), now].map(quote).join(", ");
  console.log(
    `INSERT INTO apps (owner, repo, name, description, subtitle, about, category, listing, checked) VALUES (${values})` +
      ` ON CONFLICT DO UPDATE SET description = excluded.description, subtitle = excluded.subtitle, about = excluded.about, category = excluded.category, listing = COALESCE(excluded.listing, listing), checked = excluded.checked;`,
  );
}
