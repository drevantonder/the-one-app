# Design

What users see, as the project's owner decided it. Only their decisions are here: what an agent picked and they haven't corrected is not, and is free to change. Before changing how a screen looks or behaves, read its section; a rewrite carries every line of it over. A decision that replaces one here deletes it in the same change. Nothing here describes a screen: the code does that.

## Everywhere

- Follow Jakob's Law. It is the first rule and comes before every other one here: people spend most of their time in other apps and on other operating systems (ours has an OS feel to it), so ours works the way those already do.
- A word on a screen is the one iOS, Android, Google or GitHub uses for the same thing.
- What a device does itself still follows the device: scrolling, the keyboard, the back gesture, its pickers and its share sheet.
- Each platform's conventions are followed (17 Nov): an iPhone's text size, its bounce, a list passing under the bar and the home indicator, a status-bar tap to the top; a row's ⋯ menu also opens on a long press or a right-click; a swipe on a row only for a clear Delete or common action already in that menu; no pull to refresh, since lists update live. Cmd-, stays unbound for now.
- Three layers. Principles, the same everywhere: content first; order from type size and weight, not boxes, outlines or shadows; colour is for app icons and for warn or danger ("this needs you"), never for "tap this": a button's weight says how much it matters; fine is silent (no "Connected" or green); a problem shows on the thing itself, as one problem box (symbol, title, one sentence, the action that fixes it); an empty screen is one grey line, with a button only for an obvious first step; one component per job.
- Our look, the same on every client: shadcn's design principles, language and component names, with its neutral theme and no platform tint. One `default` button per view, then `secondary`, `outline` and `ghost`; red only for `destructive`. App icons as flat tiles, one solid colour each, external brands' real logos (never a generic glyph, in widgets too), red count badges, Phosphor icons. Layout by container queries: each part sizes itself, grids fill the room, lists stop at a readable width, and what a list opens shows beside it where there's room. Never a page width. Screens are the UI library's components, through `ui`, by choice, not by rule: a screen is written in Solid. Every screen in this repo is on it; one that doesn't opt in gets its theme and the bridge, and native UI goes once it's moved.
- The typeface is Allo at roundness 100, with the device's own monospace for code, the same on every client; a script it lacks falls back to the device's own.
- Text is one of a few type styles (`core/ui/src/tokens.css`): screens and plugins use them, never a weight or size of their own. Weights sit between Allo's named ones, a little lighter in dark; big titles and figures are tracked in; numbers that change or line up are tabular. Weight doesn't animate.
- The platform's own: only what a web view can't do (passkeys, keychain, notifications, the scanner, files, home-screen widgets, the Mac's menu bar item). The sidebar, sheets, menus and alerts are the web's, the same everywhere.
- A failure's trace id is for developers: the CLI and MCP show it, never a person's screen.
- Text is short: one idea per sentence, the fewest words that still say it, nothing the reader doesn't need now. That's screens, notifications, CLI output, MCP tool descriptions and skills.
- The real thing, not counts: a small set is listed ("on You and Work"), never "added to 2 pages". No version numbers or rows of figures added to look like a reference.
- One way to do each thing. No second version of a screen to save a step; fewer screens, modes and variants beat a shortcut.
- Something that takes a while shows what it's doing where its result will be, one state at a time: its steps while it starts (`Steps`), a spinner and a line while it's under way (`Status`), on what's there now when it's busy with that, and a problem box with Try Again once it's failed. A hold-up of a few seconds isn't a problem: only one that lasts is. (Andre, 16 Nov.)
- Screens are for people; the CLI and MCP are for agents.
- A device says whether it's on in its grey line, never with a dot or the word "Offline": "Online" while its socket is open, else "Seen 2 minutes ago" (screens, long form) or "seen 2 min ago" (CLI and MCP), and nothing for one never seen. This device says "This device" instead. Everywhere a user picks a device (Devices rows, Run Priority cards, Enabled Services columns, an app's Runs On, where schedules run) each one says it; a call to a device that's off says "mac was last seen 2 min ago", never "offline". Mockups skipped on purpose (Andre, 11-11): this is the best fit for the rules above, so change it freely later.
- A screen works fully within what MCP Apps allows (no reliance on notifications, a navigation stack, sheets outside its frame or native features), so it's the same in our clients and in someone else's.
- Motion: the browser's own transitions and animations inside a screen, the browser's View Transitions between screens, and neither when the user asked for reduced motion.

## Home Screen

- Each owner (You first, then Everyone, then your groups) has several pages side by side, then their own App Library last, as iOS's.
- A page has no title at its top, and keeps the gap a phone's own home screen has.
- Pages never scroll: a page is a fixed grid, and what doesn't fit goes on a new page. Widgets never scroll either: a fixed number of rows, and tapping one opens its app.
- Left and right move between an owner's pages. Up and down on a page that doesn't scroll moves between owners, sliding vertically; a page that scrolls, as the App Library does, keeps them. Each owner stays on the page it was left on.
- The owner switcher is fixed at the top in the middle, and its name rolls as the owner changes. Tapping it opens a list of every owner; picking one slides there.
- The page dots sit at the bottom in the middle, with nothing behind them, and belong to their owner's row: they slide with it. Sliding along them moves between pages too.
- The App Library has neither the dots nor the owner switcher. It holds only what's installed, only its owner's, by category as an iPhone's App Library: four small icons, one opening its app, anywhere else opening the category. An opened category has no owner above it, and grows from its place and back into it, as an iOS folder does.
- New apps land in the App Library, not on a page: pages hold only what's put there. An app about you (Colour Mode, Vault, Sign-In & Security, Devices, Notifications, Connect Your AI, AI Usage and the device apps) is in your App Library even when it's installed for everyone, and no app is in two.
- Desktop and tablet: no sidebar. As many whole pages side by side as fit, one to three, with the dots only when not every page fits. Up and down moves between owners, and the same owner switcher works. Nothing there the phone doesn't have.
- Editing Home pages follows iOS's own, in its gestures, states and look. Touch and hold an app or empty space, then Edit Home Screen (right-click on a computer). Icons jiggle, each with ⊖, which takes it off the Home Screen and leaves it in the App Library; uninstalling is the Plugins app's. Drag to reorder, and hold at a page's edge to move to the next page, a new one past the last. + opens the widget gallery: pick a widget and its size, then place it. Tapping the page dots opens the page overview, to reorder, hide or delete Home pages. Done, or a tap on empty space, leaves. From the App Library, touch and hold an app, then drag it onto a Home page. A computer has the same edit mode, as Launchpad does.
- Whoever may arrange an owner's pages edits them; anyone else gets only the app's own menu on touch and hold, with no Edit Home Screen. An owner's pages are one layout: editing everyone's or a group's changes them for everyone who sees them.
- Settings isn't a place: its apps are ordinary apps, each in its category.
- Widgets come in a phone's own sizes: 2×2, 4×2 and 4×4. A widget's title is low in its order.

## Vault

- The one place for everything a person has given access to, in sections: Asked for, Passwords and keys, Accounts, then one for each other kind of item a plugin keeps there (Chrome Profiles).
- Accounts are every plugin's, in one list, each with its plugin's logo; Add Account under them.
- An item is ready, busy or action_required, as its plugin says: ready shows nothing; busy shows a spinner and what's under way; action_required shows a dot, why and the one action that fixes it (Sign In, Try Again). Signing out is that, or deleting the item.
- Deleting an item is soft delete's, as for anything an app made: it's in Recently Deleted for 30 days.
- An item a plugin keeps opens beside the list on a desktop, drawn by that plugin itself (a Chrome profile by Chrome Profiles).

## Plugins (17 Nov)

- Two parts, Marketplace and Installed, in one app, and quiet.
- Marketplace: one row for each of the nine plugin categories, cut so the next app shows, with See All; apps the size they are on the Home Screen.
- A plugin's page has a row for each owner, with Install, Open and its state there; then Requires, each requirement's title and why, never "What you get"; then See All.
- Trusted and untrusted are said plainly. An update available, a failed install with why and Try Again, and Recently Deleted for what was uninstalled.
- Install installs what it requires too, saying why. Uninstalling one that others require warns first that they go too.
- Uninstalling puts a plugin and its data in Recently Deleted, as one item, whose Restore brings both back for 30 days.
- An owner you can't install for shows a plain line, "Only Andre can install for Everyone", with no button.

## Devices, Sign-In & Security and Connect Your AI

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

## Opening, the connection and updates (17 Nov)

- The window's states and the connection's are the owner's signed-off machines (`client.js`, `Connection` in core/client): a new state or move is theirs to sign off.
- Opening is the `Steps` look: "Connecting to your network" the first time, "Can't reach your network" while it's offline with nothing kept, then "Opening Home Screen". A launcher kept from before opens without the network.
- With the launcher showing, a network that doesn't answer is a thin banner at the top, as Slack and Gmail have, and everything else stays: "Reconnecting…" as it tries again, and nothing once it's back.
- No launcher: "There's no Home Screen yet." with Open Plugins, or "Only Andre can install a Home Screen for Everyone" where the person can't.
- A launcher that doesn't draw: "Home Screen didn't open." with Try Again and Show Details.
- Update is an icon top right on Home pages, beside the owner switcher, with the word beside it on desktop as Chrome's Update pill, shown only when an update for this device is ready.

## Desktop windows

- A Mac window has an overlay title bar with one fixed sidebar button. Windows and Linux keep the system's bar, tinted.

## Git Repos

- A repo's page follows GitHub's layout.
