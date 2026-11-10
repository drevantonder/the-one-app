// A tiny app for the tests that need an app with a database in `global` that they write to and read back: each post is a
// line of text, numbered in the order it was written.

const where = (ctx) => ctx.global;

export default {
  name: "notes",
  title: "Notes",
  description: "Lines of text, for tests",
  schema: { global: [`create table posts (n integer primary key, text text not null)`] },
  // Runs only where a user chose a device to run notes' schedules.
  schedules: { tick: { tool: "post", input: { text: "on schedule" }, every: "1m", description: "Adds a line" } },
  tools: {
    post: {
      reads: false,
      description: "Add a line.",
      input: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
      async run(ctx, { text }) {
        where(ctx).exec("insert into posts (n, text) values ((select coalesce(max(n), 0) + 1 from posts), ?)", [text]);
        return "Added.";
      },
    },
    fail: {
      reads: true,
      description: "Throws what it's given, as an app's error would carry it.",
      input: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
      async run(ctx, { text }) {
        throw new Error(text);
      },
    },
    refuse: {
      reads: true,
      description: "Refuses what it's asked, saying what it's given.",
      input: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
      async run(ctx, { text }) {
        throw Object.assign(new Error(text), { code: "refused" });
      },
    },
    read: {
      reads: true,
      description: "Every line, oldest first.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        return { posts: await where(ctx).query("select n, text from posts order by n") };
      },
    },
  },
};
