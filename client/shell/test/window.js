// Stands in for the app's window with a browser, to look at an app as it's drawn: the window's own page
// (`client/shell/page`), the home pages and each app's page as a server serves them, and the client's Rust half answering both, as
// `one mcp`. Only the window's glue is this file's: what the page asks the window (`window.__TAURI__`) is answered
// here from that client. A proof or a person's eye then sees what a device would show.
//
//     ONE_HOME=<client> ONE_AS=<user> bun client/shell/test/window.js <folder> <steps.js>
//
// `steps.js` exports `default async ({ page, home, app, shot })`: `page` is Playwright's, `home` is the home pages'
// frame, `app(name)` opens an app from them and gives its frame, and `shot(name)` takes a picture into <folder>, of each size and scheme:
// a phone and a desktop, light and dark. It runs from `address/`, which has Playwright.
import { readFileSync } from "node:fs";
import { chromium } from "playwright";

const [folder, steps] = process.argv.slice(2);
const root = `${import.meta.dir}/../../..`;
const one = Bun.spawn([`${root}/target/release/one`, "mcp"], { stdin: "pipe", stdout: "pipe", stderr: "inherit" });
const waiting = new Map();
let asked = 0;
(async () => {
  let read = "";
  for await (const part of one.stdout.pipeThrough(new TextDecoderStream())) {
    const lines = (read + part).split("\n");
    read = lines.pop();
    for (const answer of lines.map((line) => JSON.parse(line))) {
      waiting.get(answer.id)?.(answer);
      waiting.delete(answer.id);
    }
  }
})();
/** Asks the client what a view asks, in MCP's words, and gives its result. */
const ask = (method, params = {}) =>
  new Promise((answered, refused) => {
    const id = ++asked;
    waiting.set(id, ({ result, error }) => (error ? refused(new Error(error.message)) : answered(result)));
    one.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    one.stdin.flush();
  });

let reads;
const sizes = { phone: { width: 390, height: 844 }, desktop: { width: 1280, height: 800 } };
const page_files = { "index.html": "text/html", "client.js": "text/javascript", "client.css": "text/css" };
/** What a request says of who it's for: a group or the network, or nothing for the client's own user. */
const said = (who) => (who ? { _meta: { for: who } } : {});

const browser = await chromium.launch();
for (const [size, viewport] of Object.entries(sizes)) {
  for (const scheme of ["light", "dark"]) {
    // A picture is of where a move ends, so nothing is asked to move.
    const phone = size === "phone";
    const context = await browser.newContext({ viewport, colorScheme: scheme, deviceScaleFactor: 2, hasTouch: phone, isMobile: phone, reducedMotion: "reduce" });
    const page = await context.newPage();
    // What goes wrong in a page is said here, as the app's own log would say it.
    page.on("pageerror", (error) => console.error(`${size} ${scheme}: ${error.message}`));
    page.on("console", (said) => said.type() === "error" && console.error(`${size} ${scheme}: ${said.text()}`));
    // What the page asks its window. A recovery key is the one thing asked of the device that this can't give:
    // the one shown is made up, and typing one lets nothing in.
    await page.exposeFunction("asksWindow", async (command, args) => {
      if (command === "apps") return { ...(await ask("resources/list", said(args.who))), outdated: [] };
      if (command === "client") return { device: "this-device", network: null };
      if (command === "ask") return ask(args.method, args.params);
      // Whether a tool only reads, as the server lists it: one that doesn't may have changed what a view was answered.
      if (command === "reads") return (reads ??= (await ask("tools/list")).tools.filter((tool) => tool.annotations.readOnlyHint).map((tool) => tool.name));
      if (command === "recovery_key") return "MADE-UPKE-YFOR-APIC-TURE-ONLY-NOTA-REAL-ONEA-BCDE-FGHI-JKLM-NOPQ";
      if (command === "recover") throw new Error("That isn't your recovery key. Check it against what you wrote down.");
      return null;
    });
    await page.addInitScript(() => {
      const heard = { changed: [] };
      const invoke = async (command, args) => {
        const answer = await window.asksWindow(command, args);
        // What a view was answered may have changed once it changed something, and it's told to ask again.
        if (command === "ask" && args.method === "tools/call" && !(await window.asksWindow("reads")).includes(args.params.name))
          for (const listener of heard.changed) listener({ payload: args.view });
        return answer;
      };
      window.__TAURI__ = { core: { invoke }, event: { listen: (what, listener) => (heard[what] ??= []).push(listener) } };
    });
    await page.route("http://tauri.localhost/**", (route) => {
      const file = new URL(route.request().url()).pathname.slice(1);
      const from = file === "tokens.css" ? `${root}/ui/src/tokens.css` : `${root}/client/shell/page/${file}`;
      route.fulfill({ contentType: page_files[file] ?? "text/css", body: readFileSync(from) });
    });
    // An app's file, as the app's window finds it: among the newest files of its plugin that a server lists for
    // whoever the app is for, which follows its name in its address, last part first.
    await page.route(/^http:\/\/oneapp\.[^/]+\.localhost\//, async (route) => {
      const address = new URL(route.request().url());
      const [plugin, ...path] = address.pathname.slice(1).split("/");
      const who = address.hostname.split(".").slice(2, -1).reverse().join("/");
      const { resources } = await ask("resources/list", said(who));
      const version = resources.find(({ uri }) => uri.startsWith(`ui://${plugin}/`))?.uri.split("/")[3];
      try {
        const { contents } = await ask("resources/read", { uri: `ui://${plugin}/${version}/${path.join("/")}`, ...said(who) });
        route.fulfill({ contentType: contents[0].mimeType.replace(";profile=mcp-app", ""), body: contents[0].text });
      } catch {
        route.fulfill({ status: 404, body: "" });
      }
    });
    await page.goto("http://tauri.localhost/index.html");
    const home = page.frameLocator('iframe[src$="/home/app/index.html"]');
    const app = async (name) => {
      await home.locator(".tile", { hasText: name }).first().click();
      return page.frameLocator("iframe:not([inert])");
    };
    // A picture is taken once what moves has stopped.
    const shot = async (name) => {
      await page.waitForTimeout(600);
      await page.screenshot({ path: `${folder}/${name}-${size}-${scheme}.png` });
    };
    await (await import(steps)).default({ page, home, app, shot, size, scheme });
    await context.close();
  }
}
await browser.close();
one.stdin.end();
