// The picture core/test/insights.sh takes of AI Gateway's Spending row, with client/shell/test/window.js.
export default async ({ page, home, shot }) => {
  await home.locator(".tile", { hasText: "AI Gateway" }).first().click()
  const app = page.frameLocator("iframe:not([inert])")
  await app.getByText("tokens this week").first().waitFor({ timeout: 15000 })
  await shot("ai-gateway")
}
