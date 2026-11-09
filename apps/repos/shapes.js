// What Repos' tools answer, as JSON Schema: each tool's `output`. A repo on GitHub answers in the same shapes as one here
// (github.js), and a time is an ISO 8601 text in both.

const text = (description) => ({ type: "string", description });
const nullable = (schema) => ({ ...schema, type: [].concat(schema.type, "null") });

/// Someone in the network, or on GitHub.
export const PERSON = { type: "object", properties: { handle: { type: ["string", "null"] }, name: { type: "string" } } };

export const LABEL = {
  type: "object",
  required: ["name"],
  properties: { name: text("its name"), color: text("hex, like d73a4a"), description: { type: "string" }, open_issues: { type: ["integer", "null"], description: "how many open issues have it" } },
};

export const ISSUE = {
  type: "object",
  required: ["number", "title", "state"],
  properties: {
    repo: text("<owner>/<name>"),
    number: { type: "integer" },
    title: { type: "string" },
    state: { type: "string", enum: ["open", "closed"] },
    state_reason: { type: ["string", "null"], description: "completed, not_planned or reopened, once it has one" },
    author: nullable(PERSON),
    labels: { type: "array", items: LABEL },
    assignees: { type: "array", items: PERSON },
    comments: { type: "integer", description: "how many it has" },
    created: text("ISO 8601"),
    updated: text("ISO 8601"),
    closed: text("ISO 8601, once closed"),
    closed_by: nullable(PERSON),
    body: text("Markdown, in the issue itself"),
    url: text("on GitHub"),
  },
};

export const COMMENT = {
  type: "object",
  required: ["id", "body"],
  properties: {
    repo: text("<owner>/<name>"),
    number: { type: "integer", description: "its issue's number" },
    id: { type: "string" },
    author: nullable(PERSON),
    body: { type: "string" },
    created: text("ISO 8601"),
    updated: text("ISO 8601"),
    yours: { type: "boolean", description: "whether you may edit and delete it" },
  },
};

/// A commit as a list shows it.
export const COMMIT = {
  type: "object",
  required: ["sha", "subject", "author"],
  properties: {
    sha: { type: "string" },
    subject: text("its message's first line"),
    body: text("the rest of its message"),
    author: { type: "string" },
    email: { type: "string" },
    time: text("when it was written, ISO 8601"),
    committed: text("when it was committed, ISO 8601"),
    parents: { type: "array", items: { type: "string" } },
  },
};

export const FILE_CHANGE = {
  type: "object",
  required: ["path", "status"],
  properties: {
    path: { type: "string" },
    old_path: { type: ["string", "null"], description: "before a rename" },
    status: text("added, modified, deleted or renamed"),
    old_mode: { type: ["string", "null"] },
    new_mode: { type: ["string", "null"] },
    additions: { type: "integer" },
    deletions: { type: "integer" },
    hunks: { type: "array", items: { type: "object" }, description: "the diff, left out of a big file's in repos_commit" },
    binary: { type: "boolean" },
    too_big: { type: "boolean" },
    large: { type: "boolean", description: "its diff is left for repos_diff" },
    later: { type: "boolean" },
  },
};

export const REPO_LINE = {
  type: "object",
  required: ["repo", "name"],
  properties: {
    repo: text("<owner>/<name>, or github.com/<owner>/<name>"),
    name: { type: "string" },
    owner: PERSON,
    from: { type: ["string", "null"], description: "the repo it was forked from" },
    branch: { type: ["string", "null"], description: "its default branch" },
    branches: { type: ["integer", "null"], description: "how many it has" },
    created: text("ISO 8601"),
    updated: text("when it last moved, ISO 8601"),
    open_issues: { type: "integer" },
    deleted: { type: ["object", "null"], description: "{ time, by } once deleted", additionalProperties: true },
    yours: { type: "boolean", description: "whether you own it" },
    push: { type: "boolean", description: "whether you may push to it" },
    github: { type: "object", description: "{ url, private, fork }, for one on GitHub", additionalProperties: true },
  },
};

export const BRANCH_HEAD = {
  type: "object",
  required: ["branch", "head"],
  properties: {
    branch: { type: "string" },
    head: { type: "string", description: "its head commit's sha" },
    pushed_by: { type: ["string", "null"] },
    time: text("when it was pushed, ISO 8601"),
    subject: { type: ["string", "null"] },
    author: { type: ["string", "null"] },
    committed: text("ISO 8601"),
  },
};

/// A sign-in or connection step: where the user goes, and what then.
export const STEP = { type: "object", required: ["url", "then"], properties: { url: text("for the user to open"), then: text("what happens next, in words") } };

export const TEXT = (description) => text(description);
