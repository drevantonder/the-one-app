// Notifications' edge part. A server acting for the network asks it to wake phones and browsers
// (`/notifications/send`) with what it encrypted to each, which this can't read either, and it does, through the push services whose keys only it holds (`senders.js`). A browser
// asks it for the key it subscribes with (`/notifications/key`).
import { send } from "./senders.js";

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";

export default {
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    if (pathname === "/notifications/key" && request.method === "GET") return Response.json({ vapid: env.VAPID_PUBLIC_KEY ?? null });
    if (pathname !== "/notifications/send" || request.method !== "POST") {
      return Response.json({ error: "Notifications are sent at /notifications/send" }, { status: 404 });
    }
    const body = await request.text();
    const server = await env.NETWORK.verifiedKey(request.url, Object.fromEntries(request.headers), body);
    if (!server || !(await env.NETWORK.servers(NETWORK)).includes(server)) {
      return Response.json({ error: "Only a server acting for the network sends notifications" }, { status: 403 });
    }
    const { devices } = JSON.parse(body);
    return Response.json(await send(fetch, env, devices));
  },
};
