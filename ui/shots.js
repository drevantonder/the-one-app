// Pictures of the example in a real browser, to look at after a change: a phone's width under a
// finger and a desktop's with a mouse, light and dark. `bun run shots <folder>`.
import { mkdirSync } from "node:fs"
import { chromium } from "playwright"
import { serve } from "./serve.js"

const folder = process.argv[2] ?? "shots"
mkdirSync(folder, { recursive: true })
const server = serve(0)
const browser = await chromium.launch()
const sizes = {
  phone: { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  desktop: { viewport: { width: 1280, height: 800 } },
}

for (const [size, options] of Object.entries(sizes)) {
  for (const colorScheme of ["light", "dark"]) {
    const page = await (await browser.newContext({ ...options, colorScheme, locale: "en-AU" })).newPage()
    // A picture of a page that threw is a picture of nothing.
    page.on("pageerror", (error) => (console.error(error), (process.exitCode = 1)))
    page.on("console", (message) => message.type() === "error" && (console.error(message.text()), (process.exitCode = 1)))
    const shot = (name) => page.screenshot({ path: `${folder}/${name}-${size}-${colorScheme}.png` })
    const press = (selector) => (options.hasTouch ? page.tap(selector) : page.click(selector))
    // What moves has to have arrived before its picture: everything but what turns for ever, in every shadow root.
    const settled = () => page.evaluate(async () => {
      await new Promise((frame) => requestAnimationFrame(() => requestAnimationFrame(frame)))
      const moving = (root) => [...root.getAnimations(), ...[...root.querySelectorAll("*")].flatMap((element) => (element.shadowRoot ? moving(element.shadowRoot) : []))]
      await Promise.all(moving(document).filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity).map((animation) => animation.finished.catch(() => {})))
    })

    await page.goto(`${server.url}example/`)
    await page.waitForFunction(() => !document.querySelector("#empty").hidden)
    await shot("example-empty")
    for (const [what, amount, category] of [["Coffee with Sam", "8.5", "meals"], ["Taxi from the airport", "64", "travel"], ["Hotel", "212", "stay"]]) {
      await press("#add")
      await page.locator("ui-input[name=what] input").fill(what)
      await page.locator("ui-number[name=amount] input").fill(amount)
      await page.evaluate((category) => (document.querySelector("ui-select[name=category]").value = category), category)
      if (what === "Hotel") {
        await settled()
        await shot("example-add")
      }
      await press("#save")
      await settled()
    }
    await shot("example-list")
    await press("ui-row:nth-child(2)")
    await settled()
    await shot("example-edit")
    await press("#remove")
    await settled()
    await shot("example-deleted")
  }
}

await browser.close()
server.stop()
