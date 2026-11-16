// Notifications' edge function. A server acting for the network asks it to wake phones and browsers
// (`/notifications/send`) with what it encrypted to each, which this can't read either, and it does, through the push services whose keys only it holds (`senders.ts`). A browser
// asks it for the key it subscribes with (`/notifications/key`).
import { send, type Device, type Keys } from "./senders.ts";

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";

/** Its bindings: who the network's own are (the presigner's `Network`, which the edge gives every plugin's part), and its keys. */
interface NotificationsEnv extends Keys {
  NETWORK: {
    verifiedKey(url: string, headers: Record<string, string>, body: string): Promise<string | null>;
    servers(of: string): Promise<string[]>;
  };
}

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
    const { devices }: { devices: Device[] } = JSON.parse(body);
    return Response.json(await send(fetch, env, devices));
  },
} satisfies ExportedHandler<NotificationsEnv>;
