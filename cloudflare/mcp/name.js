// What an outside program's device is called on the Devices screen and in its approval: where its sign-in really went
// first, so a name it chose for itself can't push that out of view, and the end of a long host, where its domain is.

const HOST_MOST = 40;
const NAME_MOST = 64;

export function deviceName(called, goes) {
  const name = String(called || "MCP client");
  if (!goes) return name.slice(0, NAME_MOST);
  const host = goes.length > HOST_MOST ? `…${goes.slice(1 - HOST_MOST)}` : goes;
  return `${host} · ${name}`.slice(0, NAME_MOST);
}
