---
name: look-through-screens
description: Looking through what a user sees in The One App, step by step, such as signing in or joining.
---

# Look through a journey's screens

Before anyone's signed in: `onboarding/` has the script that draws every auth page as a file, in any custom build's look.

Signed in: the see-a-ui-change skill's `one-dev throwaway client <kind>`, as a new user would meet it, with `--as sam` for someone invited. Drive it through each step.

A picture of any page, at a width, light or dark: Playwright itself, `bunx playwright@1.58.0 screenshot --color-scheme=dark --viewport-size=390,844 <url> out.png` (`--channel chrome` to use this machine's Chrome). Look at every picture you take.
