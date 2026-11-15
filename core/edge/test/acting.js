// A server's key, which is on no list until someone adds it, telling sign-in's edge part to let
// a device in as someone, as a server does (`SignIn`). `bun acting.js <address> <file>` makes the key, keeps it in
// the file and prints it; `bun acting.js <address> <file> signed|unsigned <who> <code>` asks with it, and prints
// what it's answered.
import { hex } from "../ask.js";

const [address, file, how, who, code] = process.argv.slice(2);
if (!how) {
  const { privateKey, publicKey } = await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"]);
  await Bun.write(file, JSON.stringify(await crypto.subtle.exportKey("jwk", privateKey)));
  console.log(hex.encode(new Uint8Array(await crypto.subtle.exportKey("raw", publicKey))));
  process.exit();
}
const jwk = await Bun.file(file).json();
const privateKey = await crypto.subtle.importKey("jwk", jwk, "Ed25519", false, ["sign"]);
const key = hex.encode(Buffer.from(jwk.x, "base64url"));

const path = "/sign-in/ask";
const body = JSON.stringify({ ask: "let_in", who, code });
const at = String(Date.now());
const message = new TextEncoder().encode(`one/ask/1\n${path}\n${at}\n${body}`);
const signature = hex.encode(new Uint8Array(await crypto.subtle.sign("Ed25519", privateKey, message)));
const headers = { "content-type": "application/json", ...(how === "signed" && { "one-key": key, "one-at": at, "one-signature": signature }) };
const answer = await fetch(address + path, { method: "POST", headers, body });
console.log(`${answer.status} ${await answer.text()}`);
