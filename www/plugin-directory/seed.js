// The SQL that lists the plugins in this repo's own plugins/ folder that a network may choose to install: every one
// but those whose server says it's essential, which every network has already.
//     bun seed.js > seed.sql
import { Glob } from "bun";
import { dirname } from "node:path";
import { frontMatter } from "./worker.ts";

const plugins = new URL("../../plugins/", import.meta.url).pathname;
const quote = (text) => `'${text.replaceAll("'", "''")}'`;
const now = new Date().toISOString();

for (const skill of [...new Glob("*/SKILL.md").scanSync(plugins)].sort()) {
  const folder = `${plugins}${dirname(skill)}`;
  const servers = [...new Glob("server/*.ex").scanSync(folder)].map((file) => Bun.file(`${folder}/${file}`).text());
  if ((await Promise.all(servers)).some((source) => /def essential, do: true/.test(source))) continue;
  const { name, description, subtitle, about } = frontMatter(await Bun.file(`${plugins}${skill}`).text());
  console.log(
    `INSERT INTO plugins (owner, repo, name, description, subtitle, about, checked) VALUES ('drevantonder', 'the-one-app', ${quote(name)}, ${quote(description)}, ${quote(subtitle)}, ${quote(about)}, ${quote(now)})` +
      ` ON CONFLICT DO UPDATE SET description = excluded.description, subtitle = excluded.subtitle, about = excluded.about, checked = excluded.checked;`,
  );
}
