// What a person's device signs to change who can do what: `global`'s member entries' content, built here and never
// taken from a node, the same as src/database/content.rs builds it (tests/contracts.rs checks they agree).

const node = (kind, data) => ({ app: "node", kind, data });

export const members = {
  device: ({ key, seal, user, name, roles = ["client"], newUser, device, signIn, place }) =>
    node("device", { key, seal, user, name, roles, ...(newUser ? { user_name: newUser.name, role: newUser.role } : {}), ...(device ? { device } : {}), ...(signIn ? { sign_in: signIn } : {}), ...(place ? { place } : {}) }),
  passkey: ({ key, seal, user, name, credential }) => node("device", { key, seal, user, name, roles: ["client"], passkey: credential }),
  role: ({ user, role }) => node("role", { user, role }),
  removeUser: ({ user }) => node("remove", { user }),
  removeDevice: ({ key }) => node("remove", { key }),
  /// A device that asked to join as this person's, turned down ("Not Me") before it was listed.
  refuse: ({ key, seal, name }) => node("refuse", { key, seal, name }),
  /// What one of this person's devices signed in with, forgotten.
  forgetSignIn: ({ key }) => node("forget-sign-in", { key }),
  setting: ({ name, value = null }) => node("setting", { name, value }),
  group: ({ group, name }) => node("group", { group, name }),
  invite: ({ group, user, role }) => node("invite", { group, user, role }),
  accept: ({ group }) => node("accept", { group }),
  member: ({ group, user, role = null }) => node("member", { group, user, role }),
  rename: ({ what, id, name }) => node("rename", { [what]: id, name }),
  handle: ({ what, id, handle }) => node("rename", { [what]: id, handle }),
  epoch: ({ scope, n, to, hash }) => node("epoch", { scope, n, to, hash }),
  /// App schedules. `keys`: the hash of the app's key in each epoch sealed to the endpoint, by epoch (`epochHash`).
  appSchedules: ({ endpoint, scope, app, until, keys = {} }) => node("app-schedules", { endpoint, scope, app, until, keys }),
  runs: ({ on }) => node("runs", { on }),
  /// A part of a device's oned (a service, or `apps`, the app service) switched on or off there.
  service: ({ device, name, on = false, scope }) => node("service", { device, name, on: !!on, ...(scope ? { scope } : {}) }),
  retire({ app, owner, how }) {
    const change = { deleted: { deleted: true }, restored: { deleted: false }, erased: { erased: true } }[how];
    if (!change) throw new Error(`an app is deleted, restored or erased, not ${how}`);
    return node("app", { name: app, owner, ...change });
  },
  install: ({ app, installed }) => node(installed ? "install" : "uninstall", { app }),
};
