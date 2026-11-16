# Design

What users see, as the project's owner decided it. Only their decisions are here: what an agent picked and they haven't corrected is not, and is free to change. Before changing how a screen looks or behaves, read its section; a rewrite carries every line of it over. A decision that replaces one here deletes it in the same change. Nothing here describes a screen: the code does that.

## Everywhere

- Follow Jakob's Law. It is the first rule and comes before every other one here: people spend most of their time in other apps and on other operating systems (ours has an OS feel to it), so ours works the way those already do.
- A word on a screen is the one iOS, Android, Google or GitHub uses for the same thing.
- What a device does itself still follows the device: scrolling, the keyboard, the back gesture, its pickers and its share sheet.
- Three layers. Principles, the same everywhere: content first; order from type size and weight, not boxes, outlines or shadows; colour is for app icons and for warn or danger ("this needs you"), never for "tap this": a button's weight says how much it matters; fine is silent (no "Connected" or green); a problem shows on the thing itself, as one problem box (symbol, title, one sentence, the action that fixes it); an empty screen is one grey line, with a button only for an obvious first step; one component per job.
- Our look, the same on every client: shadcn's design principles, language and component names, with its neutral theme and no platform tint. One `default` button per view, then `secondary`, `outline` and `ghost`; red only for `destructive`. App icons as flat tiles, one solid colour each, external brands' real logos (never a generic glyph, in widgets too), red count badges, Phosphor icons. Layout by container queries: each part sizes itself, grids fill the room, lists stop at a readable width, and what a list opens shows beside it where there's room. Never a page width. Screens are the UI library's components, through `ui`, by choice, not by rule: a screen is written in Solid. Every screen in this repo is on it; one that doesn't opt in gets its theme and the bridge, and native UI goes once it's moved.
- The typeface is Allo at roundness 100, with the device's own monospace for code, the same on every client; a script it lacks falls back to the device's own.
- The platform's own: only what a web view can't do (passkeys, keychain, notifications, the scanner, files, home-screen widgets, the Mac's menu bar item). The sidebar, sheets, menus and alerts are the web's, the same everywhere.
- A failure's trace id is for developers: the CLI and MCP show it, never a person's screen.
- Text is short: one idea per sentence, the fewest words that still say it, nothing the reader doesn't need now. That's screens, notifications, CLI output, MCP tool descriptions and skills.
- The real thing, not counts: a small set is listed ("on You and Work"), never "added to 2 pages". No version numbers or rows of figures added to look like a reference.
- One way to do each thing. No second version of a screen to save a step; fewer screens, modes and variants beat a shortcut.
- Something that takes a while shows what it's doing where its result will be, one state at a time: its steps while it starts (`Steps`), a spinner and a line while it's under way (`Status`), on what's there now when it's busy with that, and a problem box with Try Again once it's failed. A hold-up of a few seconds isn't a problem: only one that lasts is. (Andre, 16 Nov.)
- Screens are for people; the CLI and MCP are for agents.
- A device says whether it's on in its grey line, never with a dot or the word "Offline": "Online" while its socket is open, else "Seen 2 minutes ago" (screens, long form) or "seen 2 min ago" (CLI and MCP), and nothing for one never seen. This device says "This device" instead. Everywhere a user picks a device (Settings › Devices rows, Run Priority cards, Enabled Services columns, an app's Runs On, where schedules run) each one says it; a call to a device that's off says "mac was last seen 2 min ago", never "offline". Mockups skipped on purpose (Andre, 11-11): this is the best fit for the rules above, so change it freely later.
- A screen works fully within what MCP Apps allows (no reliance on notifications, a navigation stack, sheets outside its frame or native features), so it's the same in our clients and in someone else's.
- Motion: the browser's own transitions and animations inside a screen, the browser's View Transitions between screens, and neither when the user asked for reduced motion.

## Home Screen

- A page has no title at its top. Its name is small above the dots.
- On a phone the dots are a centred capsule; a wide window has the Sidebar instead, and no dots or page name. Swiping moves between pages, and so does sliding along the dots.
- The top of a page keeps the gap a phone's own home screen has.
- Settings is a cog at the end of the dots, with no Back.
- Widgets come in a phone's own sizes: 2×2, 4×2 and 4×4. A widget's title is low in its order.
- Accounts is a folder with no count, and opens as a page, not a dialog.

## Sidebar

- It shows every page's apps and all of Settings, each a section you can collapse.

## App Store

- A tile's button is "Add to", then "Manage" once the app is on at least one page. Both change which pages it's on.
- Taking an app off a page keeps its data, and adding it back brings the data back.

## Settings

- Passkeys are managed inside the app.
- Connect Your AI is one short prompt pointing the AI at the network's llms.txt.
- Devices: a device is approved with no code typed, by one of the user's own clients; browsers are in the one list of devices; Priority is cards; removing a device offers "Also erase its data".
- A device that's behind says "Update ready", with Update and Update All, and never its build, which is for agents (`builds`); and Cloudflare is a row of its own in that list, which only an admin updates, last in Update All. There is no automatic-update switch. A desktop app's Update button runs `one update`.
- A device an outside program approved says "Approved by <program>", and removing a program offers to remove what it approved.

## Signing in and joining

- No consent screen: signing in with the passkey is the only question.
- The one exception is a new device (15 Nov): it shows a short code, and where you sign in asks whether it matches, with nothing typed.
- "It should be possible for you to let in new devices without my tap using the cli and/or MCP" (15 Nov, to the agents).
- The sign-in screen and the pages the auth Worker serves are one component.
- The logo is the mark alone, with no tile; the tile is for the OS's icons.
- Words are bare: a title and a button. Warnings about who will act as you stay.
- Try a Demo is in a ? menu in the start screen's corner.

## Desktop windows

- A Mac window has an overlay title bar with one fixed sidebar button. Windows and Linux keep the system's bar, tinted.

## Git Repos

- A repo's page follows GitHub's layout.
