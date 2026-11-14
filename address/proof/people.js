// People, each in a browser of their own with a passkey of their own, for a proof to send somewhere: a line read
// is `<person> <address>`, and that person opens the address, answers the one question the sign-in page asks, and
// what the page then says is printed, or where it sent them. The passkey is a virtual authenticator's, so nobody
// touches anything, and it's there for as long as this runs.
import { createInterface } from "node:readline";
import { chromium } from "playwright";

const browser = await chromium.launch();
const people = new Map();

async function person(name) {
  if (!people.has(name)) {
    const context = await browser.newContext();
    const page = await context.newPage();
    const devtools = await context.newCDPSession(page);
    await devtools.send("WebAuthn.enable");
    const passkeys = { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true };
    await devtools.send("WebAuthn.addVirtualAuthenticator", { options: passkeys });
    people.set(name, page);
  }
  return people.get(name);
}

for await (const line of createInterface({ input: process.stdin })) {
  const [name, address] = line.split(" ");
  const page = await person(name);
  try {
    // The page is opened afresh, as a link is: the same page with another fragment would only scroll.
    await page.goto("about:blank");
    await page.goto(address);
    const asks = page.locator(".ui-sign-in .ui-button");
    await asks.waitFor();
    // Who will act as them is asked for as the page opens, and there once nothing more is being fetched.
    await page.waitForLoadState("networkidle");
    const asked = (await page.locator(".ui-sign-in :is(h1, .warning, .ui-button)").allTextContents()).join(" / ");
    await asks.click();
    const said = page.locator(".ui-sign-in :is(.done, .problem)");
    await Promise.race([said.waitFor(), page.waitForURL((now) => now.pathname !== "/sign-in/")]);
    const answer = new URL(page.url()).pathname === "/sign-in/" ? await said.textContent() : `sent on to ${page.url().replace(/([?&](code|state)=)[^&]*/g, "$1…")}`;
    console.log(`asked "${asked}", and after the passkey: ${answer}`);
  } catch (error) {
    console.log(`failed: ${error.message.split("\n")[0]}`);
  }
}
await browser.close();
