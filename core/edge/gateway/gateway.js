// The gateway: sends each request at the network's address where its path says, and does nothing else. Which
// piece answers which path is made where the edge is deployed (`edge.js`): the pieces every edge has, and
// each plugin's edge part.

export default {
  fetch(request, env) {
    const first = new URL(request.url).pathname.split("/")[1];
    return env[env.PATHS[first] ?? "WEB"].fetch(request);
  },
};
