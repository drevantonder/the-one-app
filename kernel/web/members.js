// What a person's device signs to change who can do what: `global`'s member entries' content, built here and never
// taken from a node, the same as src/members.rs builds it (tests/contracts.rs checks they agree).

const node = (kind, data) => ({ app: "node", kind, data });

export const members = {
  device: ({ key, seal, user, name, roles = ["client"], newUser, device }) => node("device", { key, seal, user, name, roles, ...(newUser ? { user_name: newUser.name, role: newUser.role } : {}), ...(device ? { device } : {}) }),
  passkey: ({ key, seal, user, name, credential }) => node("device", { key, seal, user, name, roles: ["client"], passkey: credential }),
  role: ({ user, role }) => node("role", { user, role }),
  /// `seqs`: the last of each of their devices' entries in `global` the remover holds (null for none), by key.
  removeUser: ({ user, seqs }) => node("remove", { user, seqs }),
  removeDevice: ({ key, seq = null }) => node("remove", { key, seq }),
  setting: ({ name, value = null }) => node("setting", { name, value }),
  group: ({ group, name }) => node("group", { group, name }),
  invite: ({ group, user, role }) => node("invite", { group, user, role }),
  accept: ({ group }) => node("accept", { group }),
  member: ({ group, user, role = null }) => node("member", { group, user, role }),
  rename: ({ what, id, name }) => node("rename", { [what]: id, name }),
  handle: ({ what, id, handle }) => node("rename", { [what]: id, handle }),
  epoch: ({ scope, n, to, hash }) => node("epoch", { scope, n, to, hash }),
  /// `keys`: the hash of the app's key in each epoch sealed to the endpoint, by epoch (`epochHash`).
  grant: ({ endpoint, scope, app, until, keys = {} }) => node("grant", { endpoint, scope, app, until, keys }),
  runs: ({ on }) => node("runs", { on }),
  /// A part of a device's oned (a service, or `apps`, the app service) switched on or off there.
  service: ({ device, name, on = false }) => node("service", { device, name, on: !!on }),
  retire({ app, owner, how }) {
    const change = { deleted: { deleted: true }, restored: { deleted: false }, erased: { erased: true } }[how];
    if (!change) throw new Error(`an app is deleted, restored or erased, not ${how}`);
    return node("app", { name: app, owner, ...change });
  },
  install: ({ app, installed }) => node(installed ? "install" : "uninstall", { app }),
};
