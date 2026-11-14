// An assistant somewhere else: an MCP client that knows only a network's address, and is let in at `/mcp` the way
// MCP's authorization says. This is the official SDK's client, which finds the authorization server from what the
// address answers, registers itself, and asks a person to let it in: the address it sends them to is printed as
// `authorize <address>`, for a proof to hand to someone's browser. Once they have, it lists the tools and calls
// each tool named after the address, and prints what it's answered.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { UnauthorizedError } from "@modelcontextprotocol/sdk/client/auth.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const [address, ...tools] = process.argv.slice(2);

// Where the person's browser is sent back to with the code, on this machine.
let came_back;
const code = new Promise((resolve) => (came_back = resolve));
const back = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  fetch(request) {
    came_back(new URL(request.url).searchParams.get("code"));
    return new Response("Go back to your assistant.");
  },
});
const redirect = `http://127.0.0.1:${back.port}/callback`;

// What the SDK keeps between steps, which a real assistant keeps on disk.
const kept = {};
const provider = {
  redirectUrl: redirect,
  clientMetadata: {
    client_name: "Proof Assistant",
    redirect_uris: [redirect],
    grant_types: ["authorization_code", "refresh_token"],
    response_types: ["code"],
    token_endpoint_auth_method: "none",
  },
  clientInformation: () => kept.client,
  saveClientInformation: (client) => void (kept.client = client),
  tokens: () => kept.tokens,
  saveTokens: (tokens) => void (kept.tokens = tokens),
  codeVerifier: () => kept.verifier,
  saveCodeVerifier: (verifier) => void (kept.verifier = verifier),
  redirectToAuthorization: (authorize) => console.log(`authorize ${authorize}`),
};

const reach = () => new StreamableHTTPClientTransport(new URL(`${address}/mcp`), { authProvider: provider });
const assistant = new Client({ name: "proof-assistant", version: "1.0.0" });
let way = reach();
try {
  await assistant.connect(way);
} catch (error) {
  if (!(error instanceof UnauthorizedError)) throw error;
  console.log("refused before anyone let it in, and told where to ask");
  await way.finishAuth(await code);
  way = reach();
  await assistant.connect(way);
}
const listed = await assistant.listTools();
console.log(`listed ${listed.tools.length} tools, among them ${listed.tools.slice(0, 4).map((tool) => tool.name).join(", ")}`);
for (const name of tools) {
  const answer = await assistant.callTool({ name, arguments: {} });
  console.log(`${name}: ${answer.content.map((part) => part.text).join(" ")}`);
}
// It stays to be asked again: each line read is a tool to call, until its input closes.
for await (const line of console) {
  try {
    const answer = await assistant.callTool({ name: line.trim(), arguments: {} });
    console.log(`${line.trim()}: ${answer.content.map((part) => part.text).join(" ")}`);
  } catch (error) {
    console.log(`${line.trim()}: refused, ${error.message}`);
  }
}
await assistant.close();
back.stop();
