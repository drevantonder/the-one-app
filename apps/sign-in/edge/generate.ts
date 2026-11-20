// Writes what Better Auth's tables need that `migrations/` doesn't make yet as the next migration there, as Better
// Auth's CLI's `generate` does: it works it out against SQLite with every migration applied, as D1 is. Run it after
// changing Better Auth's version or what it's set up with (`options` in `auth.ts`), and the edge's deploy applies it
// (`core/edge/edge.js`). Better Auth's own CLI can't load `auth.ts`, which runs only in a Worker.
import { Database } from "bun:sqlite";
import { readdirSync } from "node:fs";
import { join } from "node:path";
import { getMigrations } from "better-auth/db/migration";

// `auth.ts` is a Worker's, and reads its bindings only as it answers a request, which this never does.
Bun.plugin({
  setup(build) {
    build.module("cloudflare:workers", () => ({
      loader: "object",
      exports: { env: {}, waitUntil: () => {}, WorkerEntrypoint: class {}, DurableObject: class {} },
    }));
  },
});
const { options } = await import("./auth.ts");

const folder = join(import.meta.dir, "migrations");
const applied = readdirSync(folder).filter((name) => name.endsWith(".sql")).sort();
const database = new Database(":memory:");
for (const name of applied) database.exec(await Bun.file(join(folder, name)).text());

const { toBeCreated, toBeAdded, compileMigrations } = await getMigrations(options("http://localhost", "generating", database));
if (!toBeCreated.length && !toBeAdded.length) {
  console.log("Better Auth's tables are as the migrations make them: nothing to write.");
} else {
  const name = `${String(applied.length).padStart(4, "0")}_better_auth.sql`;
  await Bun.write(`${folder}/${name}`, `${(await compileMigrations()).trim()}\n`);
  console.log(`Wrote migrations/${name}`);
}
