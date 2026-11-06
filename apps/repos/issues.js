// Issues, as GitHub's: each repo's numbered from 1, open or closed (as completed or not planned), with labels from the
// repo's own set, assignees and comments. Everyone who reads a repo (everyone) opens issues and comments; an issue's
// author edits, closes and reopens it; those who push to the repo (its owners and collaborators) do that to any of its
// issues, and label and assign them and manage its labels; its owners delete an issue, or anyone's comment. What the
// tools give is GitHub's shape where it has one (number, state, state_reason, labels with their colour), with
// people by handle, so a GitHub repo's issues could come back the same.

/// GitHub's labels, which every new repo starts with: name, colour (hex, no #) and description.
export const LABELS = [
  ["bug", "d73a4a", "Something isn't working"],
  ["documentation", "0075ca", "Improvements or additions to documentation"],
  ["duplicate", "cfd3d7", "This issue already exists"],
  ["enhancement", "a2eeef", "New feature or request"],
  ["good first issue", "7057ff", "Good for newcomers"],
  ["help wanted", "008672", "Extra attention is needed"],
  ["invalid", "e4e669", "This doesn't seem right"],
  ["question", "d876e3", "Further information is requested"],
  ["wontfix", "ffffff", "This will not be worked on"],
];

const PAGE = 25;
const COLOR = /^#?([0-9a-f]{6})$/i;
const REASONS = ["completed", "not_planned"];

const NUMBER = { type: "integer", description: "the issue's number in its repo, like 12 for #12, from repos_issues" };
const LABEL_NAMES = { type: "array", items: { type: "string" }, description: "label names, from repos_labels, like [\"bug\"]: the issue's labels become exactly these ([] for none)" };
const ASSIGNEES = { type: "array", items: { type: "string" }, description: "people's handles, from groups_list, like [\"sam\"], at most 10: the issue's assignees become exactly these ([] for none)" };
const BODY = (what) => ({ type: "string", description: `${what}, in Markdown (GitHub's), up to 65536 characters` });

function number(n) {
  const x = Number(String(n ?? "").trim().replace(/^#/, ""));
  if (!Number.isInteger(x) || x < 1) throw new Error("give an issue by its number, like 12 for #12: repos_issues lists them");
  return x;
}

function title(t) {
  t = String(t ?? "").trim();
  if (!t) throw new Error("an issue needs a title");
  if (t.length > 256) throw new Error("a title is at most 256 characters: put the rest in its body");
  return t;
}

function body(b, what = "a body") {
  b = String(b ?? "");
  if (b.length > 65536) throw new Error(`${what} is at most 65536 characters`);
  return b;
}

const placeholders = (xs) => xs.map(() => "?").join(", ");
const like = (q) => `%${q.replace(/[\\%_]/g, (c) => "\\" + c)}%`;

export function tools(R) {
  const { global, named, object, rights, who, byHandle, handles, randomId, iso, REPO } = R;
  const person = (ctx, id) => who(ctx, `user:${id}`);
  const COMMENT_ID = { type: "string", description: "a comment's id, from repos_issue's comments" };
  const CONFIRM = { type: "boolean", description: "true once the person agreed: it can't be undone" };

  async function issueAt(g, r, n) {
    const i = (await g.query("select * from issues where repo = ? and number = ?", [r.id, number(n)]))[0];
    if (!i) throw new Error(`${r.path} has no issue #${number(n)}: repos_issues lists them`);
    return i;
  }

  async function shape(ctx, g, r, rows) {
    if (!rows.length) return [];
    const ns = rows.map((i) => i.number);
    const [labels, assignees, comments] = await Promise.all([
      g.query(`select il.number, l.name, l.color, l.description from issue_labels il join labels l on l.repo = il.repo and l.name = il.label
                where il.repo = ? and il.number in (${placeholders(ns)}) order by l.name`, [r.id, ...ns]),
      g.query(`select number, user from issue_assignees where repo = ? and number in (${placeholders(ns)})`, [r.id, ...ns]),
      g.query(`select number, count(*) as n from comments where repo = ? and number in (${placeholders(ns)}) group by number`, [r.id, ...ns]),
    ]);
    return rows.map((i) => ({
      number: i.number, title: i.title, state: i.state, state_reason: i.state_reason, author: person(ctx, i.user),
      labels: labels.filter((l) => l.number === i.number).map(({ name, color, description }) => ({ name, color, description })),
      assignees: assignees.filter((a) => a.number === i.number).map((a) => person(ctx, a.user)),
      comments: comments.find((c) => c.number === i.number)?.n ?? 0,
      created: i.created, updated: i.updated, closed: i.closed, closed_by: i.closed_by ? person(ctx, i.closed_by) : null,
    }));
  }

  async function labelsNamed(g, r, names) {
    if (!Array.isArray(names)) throw new Error('give labels as a list of names, like ["bug"]');
    const all = await g.query("select name, color, description from labels where repo = ? order by name", [r.id]);
    return [...new Set(names.map((n) => String(n).trim().toLowerCase()))].map((n) => {
      const l = all.find((x) => x.name.toLowerCase() === n);
      if (!l) throw new Error(`${r.path} has no label ${n}: ${all.length ? `it has ${all.map((x) => x.name).join(", ")}` : "it has none"}; repos_set_label makes one`);
      return l;
    });
  }

  function people(ctx, hs) {
    if (!Array.isArray(hs)) throw new Error('give assignees as a list of handles, like ["sam"]');
    const ids = [...new Set(hs.map((h) => {
      const p = byHandle(ctx, String(h).replace(/^@/, ""));
      if (!p || !p.id.startsWith("user:") || p.removed) throw new Error(`no one has the handle ${h}: groups_list has them`);
      return p.id.slice(5);
    }))];
    if (ids.length > 10) throw new Error("an issue has at most 10 assignees");
    return ids;
  }

  const may = (ok, why) => {
    if (!ok) throw new Error(why);
  };
  const pushers = (r) => `the owners and collaborators of ${r.path}`;

  function addComment(ctx, g, r, n, text) {
    const id = randomId();
    g.exec("insert into comments values (?, ?, ?, ?, ?, ?, ?)", [id, r.id, n, ctx.me.user, text, ctx.now(), ctx.now()]);
    return { id, author: person(ctx, ctx.me.user), body: text, created: ctx.now(), updated: ctx.now(), yours: true };
  }

  async function commentAt(g, r, id) {
    const c = (await g.query("select * from comments where id = ? and repo = ?", [String(id ?? ""), r.id]))[0];
    if (!c) throw new Error(`${r.path} has no comment ${id}: repos_issue gives an issue's comments`);
    return c;
  }

  return {
    issues: {
      reads: true,
      description: "A repo's issues, as GitHub lists them: newest first, 25 a page, open ones unless state says otherwise, each with its number, title, state (and state_reason once closed: completed or not_planned), author, labels, assignees, how many comments it has, and when it was opened, last updated and closed. open_issues and closed_issues count those that match the rest of the filters; more says whether there's another page; push, whether you label, assign and close any of them; yours, whether you own the repo. repos_issue reads one with its body and comments.",
      input: {
        type: "object", required: ["repo"],
        properties: {
          repo: REPO,
          state: { type: "string", enum: ["open", "closed", "all"], description: "open (the default), closed or all" },
          label: { type: "string", description: "only issues with this label, by name, from repos_labels" },
          assignee: { type: "string", description: "only issues assigned to this person, by handle, from groups_list; none for those assigned to no one" },
          author: { type: "string", description: "only issues this person opened, by handle, from groups_list" },
          q: { type: "string", description: "words to find in issues' titles, bodies and comments, or a number like 12 or #12" },
          page: { type: "integer", description: "which 25: 0 (the default) for the newest, 1 for the 25 before them, and on while more is true" },
        },
      },
      async run(ctx, { repo, state = "open", label, assignee, author, q, page = 0 }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        if (!["open", "closed", "all"].includes(state)) throw new Error("state is open, closed or all");
        const where = ["i.repo = ?"], params = [r.id];
        if (label) {
          const [l] = await labelsNamed(g, r, [label]);
          where.push("exists (select 1 from issue_labels il where il.repo = i.repo and il.number = i.number and il.label = ?)");
          params.push(l.name);
        }
        if (assignee === "none") where.push("not exists (select 1 from issue_assignees a where a.repo = i.repo and a.number = i.number)");
        else if (assignee) {
          where.push("exists (select 1 from issue_assignees a where a.repo = i.repo and a.number = i.number and a.user = ?)");
          params.push(people(ctx, [assignee])[0]);
        }
        if (author) {
          where.push("i.user = ?");
          params.push(people(ctx, [author])[0]);
        }
        const words = String(q || "").trim();
        if (words) {
          const n = /^#?\d+$/.test(words) ? Number(words.replace("#", "")) : -1;
          where.push(`(i.number = ? or i.title like ? escape '\\' or i.body like ? escape '\\'
                       or exists (select 1 from comments c where c.repo = i.repo and c.number = i.number and c.body like ? escape '\\'))`);
          params.push(n, like(words), like(words), like(words));
        }
        const all = where.join(" and ");
        const counts = await g.query(`select state, count(*) as n from issues i where ${all} group by state`, params);
        const shown = state === "all" ? all : `${all} and i.state = ?`;
        const p = Math.max(0, Math.floor(Number(page) || 0));
        const rows = await g.query(`select * from issues i where ${shown} order by i.number desc limit ? offset ?`, [...params, ...(state === "all" ? [] : [state]), PAGE + 1, p * PAGE]);
        return iso({
          repo: r.path,
          open_issues: counts.find((c) => c.state === "open")?.n ?? 0,
          closed_issues: counts.find((c) => c.state === "closed")?.n ?? 0,
          issues: await shape(ctx, g, r, rows.slice(0, PAGE)),
          more: rows.length > PAGE,
          ...(await rights(g, r)),
        });
      },
    },
    issue: {
      reads: true,
      description: "One issue: its title, body (Markdown), state and state_reason, author, labels, assignees, times, and its comments, oldest first, each with its id, author, body and times (yours says it's yours to edit and delete). can says what you may do to it: edit its title and body, close or reopen it, triage it (labels and assignees), and delete it.",
      input: { type: "object", required: ["repo", "number"], properties: { repo: REPO, number: NUMBER } },
      async run(ctx, { repo, number: n }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const i = await issueAt(g, r, n);
        const [[shaped], comments, { yours, push }] = await Promise.all([
          shape(ctx, g, r, [i]),
          g.query("select id, user, body, created, updated from comments where repo = ? and number = ? order by created, id", [r.id, i.number]),
          rights(g, r),
        ]);
        const author = i.user === ctx.me.user;
        return iso({
          repo: r.path, ...shaped, body: i.body,
          comments: comments.map((c) => ({ id: c.id, author: person(ctx, c.user), body: c.body, created: c.created, updated: c.updated, yours: c.user === ctx.me.user })),
          can: { edit: author || push, close: author || push, triage: push, delete: yours },
        });
      },
    },
    open_issue: {
      reads: false,
      description: "Open an issue in a repo, as anyone may: a title and a body, and if you push to the repo, its labels and assignees. Gives the issue, with its number.",
      input: {
        type: "object", required: ["repo", "title"],
        properties: { repo: REPO, title: { type: "string", description: "one line, up to 256 characters" }, body: BODY("what it's about"), labels: LABEL_NAMES, assignees: ASSIGNEES },
      },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        const t = title(input.title), b = body(input.body);
        const push = await g.may("push", object(r));
        may(push || !(input.labels?.length || input.assignees?.length), `only ${pushers(r)} label and assign its issues: open it without labels or assignees`);
        const labels = input.labels ? await labelsNamed(g, r, input.labels) : [];
        const assignees = input.assignees ? people(ctx, input.assignees) : [];
        const n = r.issues + 1, now = ctx.now();
        g.check("select issues = ? from repos where id = ?", [r.issues, r.id], "someone opened an issue meanwhile: try again");
        g.exec("update repos set issues = ? where id = ?", [n, r.id]);
        g.exec("insert into issues (repo, number, title, body, state, user, created, updated) values (?, ?, ?, ?, 'open', ?, ?, ?)", [r.id, n, t, b, ctx.me.user, now, now]);
        for (const l of labels) g.exec("insert into issue_labels values (?, ?, ?)", [r.id, n, l.name]);
        for (const a of assignees) g.exec("insert into issue_assignees values (?, ?, ?)", [r.id, n, a]);
        return iso({
          repo: r.path, number: n, title: t, body: b, state: "open", state_reason: null, author: person(ctx, ctx.me.user), labels,
          assignees: assignees.map((a) => person(ctx, a)), comments: 0, created: now, updated: now, closed: null, closed_by: null,
        });
      },
    },
    edit_issue: {
      reads: false,
      description: "Change an issue: its title and body (its author, and those who push to the repo), and its labels and assignees (those who push to the repo), each replaced by what you give; leave out what stays. repos_close_issue and repos_reopen_issue change its state.",
      input: {
        type: "object", required: ["repo", "number"],
        properties: { repo: REPO, number: NUMBER, title: { type: "string", description: "its new title, one line, up to 256 characters" }, body: BODY("its new body"), labels: LABEL_NAMES, assignees: ASSIGNEES },
      },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        const i = await issueAt(g, r, input.number);
        const push = await g.may("push", object(r));
        const text = input.title !== undefined || input.body !== undefined, triage = input.labels !== undefined || input.assignees !== undefined;
        if (!text && !triage) throw new Error("give what changes: title, body, labels or assignees");
        may(!text || push || i.user === ctx.me.user, `only its author and ${pushers(r)} edit #${i.number}`);
        may(!triage || push, `only ${pushers(r)} label and assign its issues`);
        const [shaped] = await shape(ctx, g, r, [i]);
        const now = ctx.now(), out = { ...shaped, body: i.body, updated: now };
        if (input.title !== undefined) g.exec("update issues set title = ? where repo = ? and number = ?", [(out.title = title(input.title)), r.id, i.number]);
        if (input.body !== undefined) g.exec("update issues set body = ? where repo = ? and number = ?", [(out.body = body(input.body)), r.id, i.number]);
        if (input.labels !== undefined) {
          out.labels = await labelsNamed(g, r, input.labels);
          g.exec("delete from issue_labels where repo = ? and number = ?", [r.id, i.number]);
          for (const l of out.labels) g.exec("insert into issue_labels values (?, ?, ?)", [r.id, i.number, l.name]);
        }
        if (input.assignees !== undefined) {
          const ids = people(ctx, input.assignees);
          out.assignees = ids.map((a) => person(ctx, a));
          g.exec("delete from issue_assignees where repo = ? and number = ?", [r.id, i.number]);
          for (const a of ids) g.exec("insert into issue_assignees values (?, ?, ?)", [r.id, i.number, a]);
        }
        g.exec("update issues set updated = ? where repo = ? and number = ?", [now, r.id, i.number]);
        return iso({ repo: r.path, ...out });
      },
    },
    close_issue: {
      reads: false,
      description: "Close an issue, as completed (the default) or not planned, with a comment if you give one: its author closes it, and those who push to the repo close any. Closing a closed one changes its reason.",
      input: {
        type: "object", required: ["repo", "number"],
        properties: {
          repo: REPO, number: NUMBER,
          state_reason: { type: "string", enum: REASONS, description: "completed (the default): it's done; not_planned: it won't be (a duplicate, or won't fix)" },
          comment: BODY("a comment to leave as it closes"),
        },
      },
      async run(ctx, { repo, number: n, state_reason = "completed", comment }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const i = await issueAt(g, r, n);
        if (!REASONS.includes(state_reason)) throw new Error("state_reason is completed or not_planned");
        may(i.user === ctx.me.user || (await g.may("push", object(r))), `only its author and ${pushers(r)} close #${i.number}`);
        if (i.state === "closed" && i.state_reason === state_reason) throw new Error(`#${i.number} is already closed as ${state_reason.replace("_", " ")}`);
        if (comment?.trim()) addComment(ctx, g, r, i.number, body(comment, "a comment"));
        g.exec("update issues set state = 'closed', state_reason = ?, closed = ?, closed_by = ?, updated = ? where repo = ? and number = ?", [state_reason, ctx.now(), ctx.me.user, ctx.now(), r.id, i.number]);
        return `Closed ${r.path}#${i.number} as ${state_reason.replace("_", " ")}.`;
      },
    },
    reopen_issue: {
      reads: false,
      description: "Reopen a closed issue, with a comment if you give one: its author reopens it, and those who push to the repo reopen any.",
      input: { type: "object", required: ["repo", "number"], properties: { repo: REPO, number: NUMBER, comment: BODY("a comment to leave as it reopens") } },
      async run(ctx, { repo, number: n, comment }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const i = await issueAt(g, r, n);
        may(i.user === ctx.me.user || (await g.may("push", object(r))), `only its author and ${pushers(r)} reopen #${i.number}`);
        if (i.state === "open") throw new Error(`#${i.number} is open`);
        if (comment?.trim()) addComment(ctx, g, r, i.number, body(comment, "a comment"));
        g.exec("update issues set state = 'open', state_reason = 'reopened', closed = null, closed_by = null, updated = ? where repo = ? and number = ?", [ctx.now(), r.id, i.number]);
        return `Reopened ${r.path}#${i.number}.`;
      },
    },
    delete_issue: {
      reads: false,
      confirm: true,
      description: "Delete an issue for good, with its comments, as only the repo's owners may. Its number isn't given again.",
      input: { type: "object", required: ["repo", "number"], properties: { repo: REPO, number: NUMBER, confirm: CONFIRM } },
      async run(ctx, { repo, number: n }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const i = await issueAt(g, r, n);
        may(await g.may("manage", object(r)), `only the owners of ${r.path} delete its issues: close it instead (repos_close_issue)`);
        for (const table of ["issues", "issue_labels", "issue_assignees", "comments"]) g.exec(`delete from ${table} where repo = ? and number = ?`, [r.id, i.number]);
        return `Deleted ${r.path}#${i.number}.`;
      },
    },
    comment: {
      reads: false,
      description: "Comment on an issue, as anyone may. Gives the comment, with its id.",
      input: { type: "object", required: ["repo", "number", "body"], properties: { repo: REPO, number: NUMBER, body: BODY("the comment") } },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        const i = await issueAt(g, r, input.number);
        const text = body(input.body, "a comment");
        if (!text.trim()) throw new Error("a comment needs some text");
        const c = addComment(ctx, g, r, i.number, text);
        g.exec("update issues set updated = ? where repo = ? and number = ?", [ctx.now(), r.id, i.number]);
        return iso({ repo: r.path, number: i.number, ...c });
      },
    },
    edit_comment: {
      reads: false,
      description: "Change the text of a comment of yours.",
      input: { type: "object", required: ["repo", "id", "body"], properties: { repo: REPO, id: COMMENT_ID, body: BODY("its new text") } },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        const c = await commentAt(g, r, input.id);
        may(c.user === ctx.me.user, "only its author edits a comment");
        const text = body(input.body, "a comment");
        if (!text.trim()) throw new Error("a comment needs some text: repos_delete_comment deletes one");
        g.exec("update comments set body = ?, updated = ? where id = ?", [text, ctx.now(), c.id]);
        return iso({ repo: r.path, number: c.number, id: c.id, author: person(ctx, c.user), body: text, created: c.created, updated: ctx.now(), yours: true });
      },
    },
    delete_comment: {
      reads: false,
      confirm: true,
      description: "Delete a comment for good: your own, or anyone's on a repo you own.",
      input: { type: "object", required: ["repo", "id"], properties: { repo: REPO, id: COMMENT_ID, confirm: CONFIRM } },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        const c = await commentAt(g, r, input.id);
        may(c.user === ctx.me.user || (await g.may("manage", object(r))), `only its author and the owners of ${r.path} delete a comment`);
        g.exec("delete from comments where id = ?", [c.id]);
        return `Deleted the comment on ${r.path}#${c.number}.`;
      },
    },
    labels: {
      reads: true,
      description: "A repo's labels, by name: each with its colour (hex, as GitHub's: d73a4a), description, and how many open issues have it. A new repo starts with GitHub's.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        return g.query(
          `select l.name, l.color, l.description,
                  (select count(*) from issue_labels il join issues i on i.repo = il.repo and i.number = il.number
                    where il.repo = l.repo and il.label = l.name and i.state = 'open') as open_issues
             from labels l where l.repo = ? order by l.name`,
          [r.id],
        );
      },
    },
    set_label: {
      reads: false,
      description: "Make a label in a repo, or change one (its name, colour or description), as those who push to it may; a renamed label stays on its issues.",
      input: {
        type: "object", required: ["repo", "name"],
        properties: {
          repo: REPO,
          name: { type: "string", description: "the label's name, up to 50 characters: one from repos_labels to change it, or a new one" },
          new_name: { type: "string", description: "its new name, to rename it" },
          color: { type: "string", description: "its colour as hex, like d73a4a; a new one's is ededed unless given" },
          description: { type: "string", description: "what it's for, up to 100 characters" },
        },
      },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        may(await g.may("push", object(r)), `only ${pushers(r)} manage its labels`);
        const name = String(input.name ?? "").trim();
        const to = input.new_name === undefined ? name : String(input.new_name).trim();
        if (!name || !to || to.length > 50) throw new Error("a label's name is 1 to 50 characters");
        if (input.color !== undefined && !COLOR.test(input.color)) throw new Error("give a colour as hex, like d73a4a");
        if ((input.description ?? "").length > 100) throw new Error("a label's description is at most 100 characters");
        const color = input.color === undefined ? undefined : COLOR.exec(input.color)[1].toLowerCase();
        const was = (await g.query("select * from labels where repo = ? and name = ?", [r.id, name]))[0];
        if (to.toLowerCase() !== (was?.name ?? name).toLowerCase() && (await g.query("select 1 from labels where repo = ? and name = ?", [r.id, to])).length) {
          throw new Error(`${r.path} has a label ${to} already`);
        }
        if (!was) {
          if (input.new_name !== undefined) throw new Error(`${r.path} has no label ${name} to rename: repos_labels lists them`);
          const l = { name, color: color ?? "ededed", description: input.description ?? "" };
          g.exec("insert into labels values (?, ?, ?, ?)", [r.id, l.name, l.color, l.description]);
          return { ...l, open_issues: 0 };
        }
        const l = { name: to, color: color ?? was.color, description: input.description ?? was.description };
        g.exec("update labels set name = ?, color = ?, description = ? where repo = ? and name = ?", [l.name, l.color, l.description, r.id, was.name]);
        if (l.name !== was.name) g.exec("update issue_labels set label = ? where repo = ? and label = ?", [l.name, r.id, was.name]);
        return l;
      },
    },
    delete_label: {
      reads: false,
      confirm: true,
      description: "Delete a label from a repo, and from every issue that has it, as those who push to it may.",
      input: { type: "object", required: ["repo", "name"], properties: { repo: REPO, name: { type: "string", description: "the label's name, from repos_labels" }, confirm: CONFIRM } },
      async run(ctx, input) {
        const g = global(ctx);
        const r = await named(ctx, input.repo);
        may(await g.may("push", object(r)), `only ${pushers(r)} manage its labels`);
        const [l] = await labelsNamed(g, r, [input.name]);
        g.exec("delete from labels where repo = ? and name = ?", [r.id, l.name]);
        g.exec("delete from issue_labels where repo = ? and label = ?", [r.id, l.name]);
        return `Deleted the label ${l.name} from ${r.path}.`;
      },
    },
    _people: {
      reads: true,
      description: "Everyone in the network, by handle and name, for the screens' assignee and author pickers; for a GitHub repo, those GitHub lets its issues be assigned to.",
      input: { type: "object", properties: { repo: REPO } },
      run: (ctx) => handles(ctx).filter((h) => h.id.startsWith("user:") && !h.removed).map((h) => ({ handle: h.handle, name: h.name })).sort((a, b) => a.name.localeCompare(b.name)),
    },
  };
}
