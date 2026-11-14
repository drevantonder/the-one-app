// The gateway: sends each request at the network's address where its path says, and does nothing else. Which
// piece answers which path is made where the address is deployed (`address.js`): the pieces every address has, and
// each plugin's part at the address.

export default {
  fetch(request, env) {
    const first = new URL(request.url).pathname.split("/")[1];
    return env[env.PATHS[first] ?? "WEB"].fetch(request);
  },
};
