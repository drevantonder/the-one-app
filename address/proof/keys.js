// Two keys that are nobody's ask the network at `address` to be let in, each proving it holds its own key as a
// device does (`ask.js`), and the second tries to come in on the first's code. It prints what each is told.
import { hex } from "../ask.js";

const [address] = process.argv.slice(2);

/** A key of its own, and what it asks the address, signed with it unless `signed` is false. */
async function made() {
  const { privateKey, publicKey } = await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"]);
  const key = hex.encode(new Uint8Array(await crypto.subtle.exportKey("raw", publicKey)));
  const ask = async (path, body, signed = true) => {
    body = JSON.stringify(body);
    const at = String(Date.now());
    const message = new TextEncoder().encode(`one/ask/1\n${path}\n${at}\n${body}`);
    const signature = hex.encode(new Uint8Array(await crypto.subtle.sign("Ed25519", privateKey, message)));
    const headers = { "content-type": "application/json", ...(signed && { "one-key": key, "one-at": at, "one-signature": signature }) };
    const answer = await fetch(address + path, { method: "POST", headers, body });
    return { status: answer.status, said: await answer.json() };
  };
  return { key, ask };
}
const told = ({ status, said }) => `${status} ${said.error_description ?? said.error?.message ?? said.error ?? JSON.stringify(said)}`;
const grant = "urn:ietf:params:oauth:grant-type:device_code";

const [first, second] = [await made(), await made()];
const code = await first.ask("/sign-in/auth/device/code", { client_id: first.key, name: "first" });
console.log(`the first asks for a code for itself, signed with its key: ${code.status}, given ${code.said.user_code}`);
console.log(`the second asks for a code for the first's key, signed with its own: ${told(await second.ask("/sign-in/auth/device/code", { client_id: first.key }))}`);
console.log(`the second asks for one for the first's key, signing nothing: ${told(await second.ask("/sign-in/auth/device/code", { client_id: first.key }, false))}`);
const token = { grant_type: grant, device_code: code.said.device_code };
console.log(`the second asks to be let in with the first's code, as itself: ${told(await second.ask("/sign-in/auth/device/token", { ...token, client_id: second.key }))}`);
console.log(`the second asks to be let in with the first's code, as the first: ${told(await second.ask("/sign-in/auth/device/token", { ...token, client_id: first.key }))}`);
console.log(`the first asks with its own code, which nobody has answered: ${told(await first.ask("/sign-in/auth/device/token", { ...token, client_id: first.key }))}`);
for (const [which, { ask }] of [["first", first], ["second", second]]) {
  const pass = await ask("/mcp", { jsonrpc: "2.0", id: 1, method: "one/pass", params: {} });
  console.log(`the ${which} asks for its pass: ${told(pass)}`);
}
