// A small app with a screen, a widget, tools and data: copy this folder, rename it, and change it. Deploying it builds
// screens/main.jsx into dist/main.html, which this imports as text.
import main from "./dist/main.html" with { type: "text" };

const today = (ctx) => new Date(ctx.now()).toISOString().slice(0, 10);
const counted = async (ctx, day) => (await ctx.user.query("select id, at from marks where day = ? order by at desc", [day]));

export default {
  title: "Tally",
  icon: "hash",
  color: "#34c759",
  description: "Counts things, a day at a time",
  schema: {
    user: [`create table marks (id text primary key, day text not null, at integer not null); create index marks_day on marks (day);`],
  },
  tools: {
    today: {
      title: "A day's tally",
      reads: true,
      description: "A day's tally: how many, and when each was added. Gives { day, count, marks: [{ id, at }] }.",
      input: { type: "object", properties: { day: { type: "string", description: "the user's day, YYYY-MM-DD; today by default" } } },
      output: { type: "object", required: ["day", "count", "marks"], properties: {
        day: { type: "string" },
        count: { type: "integer" },
        marks: { type: "array", items: { type: "object", required: ["id", "at"], properties: { id: { type: "string" }, at: { type: "integer", description: "when, in milliseconds" } } } },
      } },
      run: async (ctx, { day = today(ctx) }) => {
        const marks = await counted(ctx, day);
        return { day, count: marks.length, marks };
      },
    },
    add: {
      title: "Add one",
      reads: false,
      description: "Adds one to a day's tally. Gives { id }, which remove takes.",
      input: { type: "object", properties: { day: { type: "string", description: "the user's day, YYYY-MM-DD; today by default" } } },
      output: { type: "object", required: ["id"], properties: { id: { type: "string" } } },
      run: async (ctx, { day = today(ctx) }) => {
        const id = crypto.randomUUID();
        ctx.user.exec("insert into marks (id, day, at) values (?, ?, ?)", [id, day, ctx.now()]);
        return { id };
      },
    },
    remove: {
      title: "Remove one",
      reads: false,
      description: "Takes one away. Gives { removed }.",
      input: { type: "object", required: ["id"], properties: { id: { type: "string", description: "a mark's id, from today" } } },
      output: { type: "object", required: ["removed"], properties: { removed: { type: "string" } } },
      run: async (ctx, { id }) => {
        const [mark] = await ctx.user.query("select id from marks where id = ?", [id]);
        if (!mark) throw new Error(`no mark ${id}: ids come from today`);
        ctx.user.exec("delete from marks where id = ?", [id]);
        return { removed: id };
      },
    },
  },
  screens: {
    main: { title: "Tally", html: main },
  },
  widgets: {
    today: {
      title: "Tally",
      description: "Today's tally",
      size: "small",
      sizes: ["small"],
      async render(ctx) {
        const marks = await counted(ctx, today(ctx));
        return { t: "col", link: { screen: "main" }, gap: 8, c: [
          { t: "stat", value: marks.length, label: "today" },
          { t: "button", label: "Add One", tool: "add" },
        ] };
      },
    },
  },
};
