// What the app's window gives its client (net-worker.js's `host`): the network's address, what it is (`client`, as its
// reads of the bucket are counted), the app's key for its user, which
// it signs and opens with itself (never handing it over), and what its keyring keeps for that user, in the app's folder.
// This code never reads the key: the app does (native-apps/src/lib.rs), and it's only ever kept in the OS's secret store.
export async function appHost(native) {
  const about = await native.about();
  const text = new TextEncoder();
  return {
    address: about.address,
    client: "app",
    device: async () => {
      const keys = await native.keys();
      return { key: keys.key, sealKey: keys.seal, sign: (bytes) => native.sign(bytes), seal: (share) => native.shared(share) };
    },
    keep: () => ({
      get: (name) => native.keepGet(name),
      put: (name, bytes) => native.keepPut(name, typeof bytes === "string" ? text.encode(bytes) : bytes),
      del: (name) => native.keepDelete(name),
    }),
  };
}
