// What this browser keeps in IndexedDB's `one` database, its one store: its keys by person (`device:<user>`), the
// invites it made (`invite:<user>`), the nodes it asks (`nodes:<user>`), and what its keyring keeps (`<name>:<user>`,
// one-client's `keyring::Keep`). A module of its own, for the worker that reaches the nodes (net-worker.js) to read
// with nothing else.

function kept(mode, act) {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open("one", 1);
    open.onupgradeneeded = () => open.result.createObjectStore("kept");
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const tx = open.result.transaction("kept", mode);
      const req = act(tx.objectStore("kept"));
      tx.oncomplete = () => (open.result.close(), resolve(req?.result));
      tx.onerror = () => (open.result.close(), reject(tx.error));
    };
  });
}

export const get = (id) => kept("readonly", (s) => s.get(id));
export const getAll = (ids) =>
  kept("readonly", (s) => {
    const asked = ids.map((id) => s.get(id));
    return { get result() { return asked.map((a) => a.result); } };
  });
/// Each `[id, value]` whose id starts with `prefix`.
export const named = (prefix) =>
  kept("readonly", (s) => {
    const range = IDBKeyRange.bound(prefix, `${prefix}\uffff`);
    const ids = s.getAllKeys(range), values = s.getAll(range);
    return { get result() { return ids.result.map((id, i) => [id, values.result[i]]); } };
  });
export const put = (id, v) => kept("readwrite", (s) => s.put(v, id));
export const del = (id) => kept("readwrite", (s) => s.delete(id));
