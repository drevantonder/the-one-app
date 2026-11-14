// Times calls on one open connection: starts the client as MCP (`one mcp`), asks `method` fifty times one after
// another, after once to connect, and prints the middle time and the slowest.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const [one, method, name] = process.argv.slice(2);
const client = spawn(one, ["mcp"], { stdio: ["pipe", "pipe", "inherit"] });
const answers = createInterface({ input: client.stdout })[Symbol.asyncIterator]();
const ask = async (id) => {
  const params = name ? { name, arguments: {} } : {};
  client.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  const answer = JSON.parse((await answers.next()).value);
  if (answer.error || answer.result.isError) throw new Error(JSON.stringify(answer));
};
await ask(0);
const times = [];
for (let id = 1; id <= 50; id++) {
  const began = performance.now();
  await ask(id);
  times.push(performance.now() - began);
}
client.stdin.end();
times.sort((a, b) => a - b);
console.log(`${method}${name ? " " + name : ""}: middle ${times[25].toFixed(1)} ms, slowest ${times[49].toFixed(1)} ms, of 50 on one connection`);
