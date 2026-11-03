// What a person's device signs to change who can do what: `global`'s member entries' content, built here and never
// taken from a node, the same as src/members.rs builds it (tests/contracts.rs checks they agree).

const node = (kind, data) => ({ app: "node", kind, data });

export const members = {
  device: ({ key, seal, user, name, roles = ["client"], newUser }) => node("device", { key, seal, user, name, roles, ...(newUser ? { user_name: newUser.name, role: newUser.role } : {}) }),
  passkey: ({ key, seal, user, name, credential }) => node("device", { key, seal, user, name, roles: ["client"], passkey: credential }),
  role: ({ user, role }) => node("role", { user, role }),
  removeUser: ({ user }) => node("remove", { user }),
  removeDevice: ({ key, seq = null }) => node("remove", { key, seq }),
  setting: ({ name, value = null }) => node("setting", { name, value }),
  group: ({ group, name }) => node("group", { group, name }),
  invite: ({ group, user, role }) => node("invite", { group, user, role }),
  accept: ({ group }) => node("accept", { group }),
  member: ({ group, user, role = null }) => node("member", { group, user, role }),
  rename: ({ what, id, name }) => node("rename", { [what]: id, name }),
  epoch: ({ scope, n, to, hash }) => node("epoch", { scope, n, to, hash }),
  /// `keys`: the hash of the app's key in each epoch sealed to the endpoint, by epoch (`epochHash`).
  grant: ({ endpoint, scope, app, until, keys = {} }) => node("grant", { endpoint, scope, app, until, keys }),
  runs: ({ on }) => node("runs", { on }),
  retire({ app, owner, how }) {
    const change = { deleted: { deleted: true }, restored: { deleted: false }, erased: { erased: true } }[how];
    if (!change) throw new Error(`an app is deleted, restored or erased, not ${how}`);
    return node("app", { name: app, owner, ...change });
  },
  install: ({ app, installed }) => node(installed ? "install" : "uninstall", { app }),
};
