// The pictures plugins/insights/test/insights.sh takes with clients/shell/test/window.js: Insights in Settings, then a
// failure's trace beside it on a desktop, and a slow tool's chart.
export default async ({ page, home, shot, size }) => {
  await home.locator('button[aria-label="Settings"]').click()
  await home.locator(".tile", { hasText: "Insights" }).first().click()
  const app = page.frameLocator("iframe:not([inert])")
  await app.getByText("Slowest").first().waitFor({ timeout: 15000 })
  await shot("insights")
  await app.getByText("Count refused on purpose").first().click()
  await app.locator(".waterfall li").first().waitFor({ timeout: 15000 })
  await shot("insights-trace")
  if (size === "phone") await app.getByRole("button", { name: "Back" }).last().click()
  await app.getByText("probe › slow").first().click()
  await app.getByText("95th Percentile").first().waitFor({ timeout: 15000 })
  await shot("insights-slow")
}
