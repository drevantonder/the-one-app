// People, each in a browser of their own with a passkey of their own, for a proof to send somewhere: a line read
// is `<person> <address> [answer]`, and that person opens the address and does what the sign-in page asks: the
// passkey if it asks for it, and for a new device's code, the answer given. That's `matches` or `stop`, `look` to
// answer nothing, or `force` to ask the edge to let the device in whatever the page shows, as someone going
// round it would. `<person> signs-out` has their browser forget they signed in. What each screen said is printed
// on one line, with what was done between them, or where it sent them. The passkey is a virtual authenticator's,
// so nobody touches anything, and it's there for as long as this runs. Each person comes from an address of their
// own, as people do, which is what the network's edge counts tries by.
import { createInterface } from "node:readline";
import { chromium } from "playwright";

const browser = await chromium.launch();
const people = new Map();

async function person(name) {
  if (!people.has(name)) {
    const context = await browser.newContext({ extraHTTPHeaders: { "cf-connecting-ip": `198.51.100.${people.size + 1}` } });
    const page = await context.newPage();
    const devtools = await context.newCDPSession(page);
    await devtools.send("WebAuthn.enable");
    const passkeys = { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true };
    await devtools.send("WebAuthn.addVirtualAuthenticator", { options: passkeys });
    people.set(name, page);
  }
  return people.get(name);
}

/** What the screen says now: its heading, the code and what's said of it, a warning or a problem, and its buttons. */
const read = async (screen) => `"${(await screen.locator(":is(h1, .code p, .warning, .problem, .done, .ui-button)").allTextContents()).join(" / ")}"`;

for await (const line of createInterface({ input: process.stdin })) {
  const [name, address, answer] = line.split(" ");
  const page = await person(name);
  // Their browser forgets they signed in, as one they haven't signed in on yet would be.
  if (address === "signs-out") {
    await page.context().clearCookies();
    console.log("is signed in nowhere now");
    continue;
  }
  const screen = page.locator(".ui-sign-in");
  const left = () => new URL(page.url()).pathname !== "/sign-in/";
  // Waits for the page to have something to press or something to say, or to have sent them somewhere else.
  const settled = async () => {
    await Promise.race([screen.locator(":is(.ui-button, .problem, .done)").first().waitFor(), page.waitForURL(left)]);
    if (!left()) await page.waitForLoadState("networkidle");
  };
  const said = [];
  try {
    // The page is opened afresh, as a link is: the same page with another fragment would only scroll.
    await page.goto("about:blank");
    await page.goto(address);
    await settled();
    said.push(await read(screen));
    const signIn = screen.locator(".ui-button", { hasText: /^(Sign in|Create a passkey)$/ });
    if (await signIn.count()) {
      // Someone joining says the name they go by first: theirs, as a person writes it.
      const called = screen.locator("input[name=name]");
      if (await called.count()) {
        await called.fill(name[0].toUpperCase() + name.slice(1));
        said.push(`types ${name[0].toUpperCase() + name.slice(1)}`);
      }
      await signIn.click();
      await Promise.race([screen.locator(":is(.done, .problem, .code)").first().waitFor(), page.waitForURL(left)]);
      await settled();
      said.push("passkey", left() ? `sent on to ${page.url().replace(/([?&](code|state)=)[^&]*/g, "$1…")}` : await read(screen));
    }
    const matches = screen.locator(".ui-button", { hasText: "It matches" });
    if (answer === "force") {
      const code = new URL(page.url()).searchParams.get("user_code");
      const forced = await page.evaluate((userCode) => fetch("/sign-in/auth/device/approve", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ userCode }) }).then((answer) => answer.json()), code);
      said.push("asks the edge to let it in anyway", `"${forced.message ?? forced.error_description ?? JSON.stringify(forced)}"`);
    } else if (["matches", "stop"].includes(answer) && (await matches.count())) {
      await screen.locator(".ui-button", { hasText: answer === "matches" ? "It matches" : "Stop" }).click();
      await screen.locator(":is(.done, .problem)").first().waitFor();
      said.push(answer === "matches" ? "It matches" : "Stop", await read(screen));
    }
    console.log(said.join(" -> "));
  } catch (error) {
    console.log(`failed: ${error.message.split("\n")[0]} (after ${said.join(" -> ")})`);
  }
}
await browser.close();
