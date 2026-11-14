// A machine that's on no list asks the relay and the signer at `address`, as iroh's endpoint and a server would,
// and prints what each tells it. Its key is real and it proves it: it's refused for whose it isn't. `list` is the
// list of servers it says it's on.
import { blake3 } from "@noble/hashes/blake3.js";
import { hex } from "../ask.js";

const [address, list] = process.argv.slice(2);
const { privateKey, publicKey } = await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"]);
const key = hex.encode(new Uint8Array(await crypto.subtle.exportKey("raw", publicKey)));
const sign = async (message) => new Uint8Array(await crypto.subtle.sign("Ed25519", privateKey, message));
// It says it's on a list of servers, which it isn't.
const pass = btoa(JSON.stringify({ list, first: "a guess" }));

const relay = new WebSocket(address.replace(/^http/, "ws") + "/relay", {
  protocols: ["iroh-relay-v2"],
  headers: { authorization: `Bearer ${pass}` },
});
relay.binaryType = "arraybuffer";
const told = await new Promise((resolve) => {
  relay.onmessage = async ({ data }) => {
    const frame = new Uint8Array(data);
    if (frame[0] !== 0) return resolve(frame);
    const challenge = blake3(frame.subarray(1), { context: new TextEncoder().encode("iroh-relay handshake v1 challenge signature") });
    relay.send(Uint8Array.of(1, ...hex.decode(key), 64, ...(await sign(challenge))));
  };
});
console.log(`the relay, once it proved its key: ${told[0] === 3 ? "denied" : "admitted"}: ${new TextDecoder().decode(told.subarray(2))}`);
relay.close();

async function ask(method, path, body) {
  const at = String(Date.now());
  const signature = hex.encode(await sign(new TextEncoder().encode(`one/ask/1\n${path.split("?")[0]}\n${at}\n${body ?? ""}`)));
  const headers = { "one-key": key, "one-at": at, "one-signature": signature, "one-pass": pass };
  const answer = await fetch(address + path, { method, headers, body });
  if (!answer.ok) return console.log(`the signer, asked to ${method} ${path} ${body ?? ""}: ${answer.status} ${await answer.text()}`);
  const signed = await answer.json();
  const put = await fetch(signed.url, { method: signed.method, headers: signed.headers, body: JSON.stringify([key]) });
  const code = /<Code>(.*?)<\/Code>/.exec(await put.text())?.[1];
  console.log(`the signer, asked to ${method} ${path} ${body}: signs it, and the bucket answers ${put.status} ${code}`);
}
await ask("POST", "/signer/sign", JSON.stringify({ read: `${list}servers` }));
await ask("POST", "/signer/sign", JSON.stringify({ write: `${list}own/tracker/stolen` }));
await ask("POST", "/signer/sign", JSON.stringify({ remove: `${list}added` }));
await ask("POST", "/signer/sign", JSON.stringify({ write: `servers/${key}/address` }));
await ask("POST", "/signer/sign", JSON.stringify({ write: "network/servers" }));
await ask("GET", `/signer/servers?of=${list}`);
