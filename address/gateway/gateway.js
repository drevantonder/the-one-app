// The gateway: sends each request at the network's address where its path says, and does nothing else.

const PIECES = {
  // iroh's own two paths: its relay protocol, and the probe that times how far away the relay is.
  relay: "RELAY",
  ping: "RELAY",
  signer: "SIGNER",
  mcp: "WAY_IN",
  link: "WAY_IN",
};

export default {
  fetch(request, env) {
    const first = new URL(request.url).pathname.split("/")[1];
    return env[PIECES[first] ?? "WEB"].fetch(request);
  },
};
