# Journeys

Notes on what people go through in The One App, written as we go.

## Getting in

What someone sees on a device before they're in the network: the first screen, signing in, waiting for another of
their devices to let this one in, and starting.

2026-11-09, found by reading the code, not by walking it:

- The first screen is `Welcome` (`src/screen/ui/src/components/welcome.tsx`). It shows the logo, the name and the way
  in: sign in, an invite link, an address to type, a pairing code, or "Open The One App" back from another site.
- The page where you use a passkey or Google is a separate hand-written copy of the same look
  (`cloudflare/auth/pages.js`). It should be the same component as the first screen.
- A custom build's branding is only its name and logo, from `config.toml`.
- Thinking about calling the screens `WelcomeScreen`, `WaitingScreen`, `StartingScreen` and `UnavailableScreen`, since
  they're whole screens and not parts. Not decided.
