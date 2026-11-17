// The page's Colour Mode from its first frame, as the client that shows it chose it: its address's `theme`, light
// or dark, or none for as the device is. It runs before the page's styles, as a classic script first in its head
// (app.js puts it there), so a dark choice on a light device never shows white while the page loads. The bridge keeps
// it as it changes (`dress` in bridge.ts).
{
  const theme = new URLSearchParams(location.search).get("theme")
  if (theme === "light" || theme === "dark") {
    document.documentElement.style.colorScheme = theme
    document.documentElement.classList.add(theme)
  }
}
