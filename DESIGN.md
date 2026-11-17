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
- Text is one of a few type styles (`core/ui/src/tokens.css`): screens and apps use them, never a weight or size of their own. Weights sit between Allo's named ones, a little lighter in dark; big titles and figures are tracked in; numbers that change or line up are tabular. Weight doesn't animate.
- The platform's own: only what a web view can't do (passkeys, keychain, notifications, the scanner, files, home-screen widgets, the Mac's menu bar item). The sidebar, sheets, menus and alerts are the web's, the same everywhere.
- A failure's trace id is for developers: the CLI and MCP show it, never a person's screen.
- Text is short: one idea per sentence, the fewest words that still say it, nothing the reader doesn't need now. That's screens, notifications, CLI output, MCP tool descriptions and skills.
- The real thing, not counts: a small set is listed ("on You and Work"), never "added to 2 pages". No version numbers or rows of figures added to look like a reference. The one exception is an app's version, in the App Store only (17 Nov).
- One way to do each thing. No second version of a screen to save a step; fewer screens, modes and variants beat a shortcut.
- Something that takes a while shows what it's doing where its result will be, one state at a time: its steps while it starts (`Steps`), a spinner and a line while it's under way (`Status`), on what's there now when it's busy with that, and a problem box with Try Again once it's failed. A hold-up of a few seconds isn't a problem: only one that lasts is. (Andre, 16 Nov.)
- Screens are for people; the CLI and MCP are for agents.
- A device says whether it's on in its grey line, never with a dot or the word "Offline": "Online" while its socket is open, else "Seen 2 minutes ago" (screens, long form) or "seen 2 min ago" (CLI and MCP), and nothing for one never seen. This device says "This device" instead. Everywhere a user picks a device (Devices rows, Run Priority cards, Enabled Services columns, an app's Runs On, where schedules run) each one says it; a call to a device that's off says "mac was last seen 2 min ago", never "offline". Mockups skipped on purpose (Andre, 11-11): this is the best fit for the rules above, so change it freely later.
- A screen works fully within what MCP Apps allows (no reliance on notifications, a navigation stack, sheets outside its frame or native features), so it's the same in our clients and in someone else's.
- Motion: the browser's own transitions and animations inside a screen, the browser's View Transitions between screens. When the user asked for reduced motion, it's Apple's way (17 Nov): what would slide, grow or zoom is a short cross-fade instead, never switched off, and a page dragged by the finger finishes with a fade rather than jumping.
- Names people read are specific and title-cased (17 Nov): "Apple Calendar" next to "Google Calendar", never just "Calendar"; "Encryption", never an id like `encryption`. A long name is cut with "…"; there are no separate short names.
- People see apps, never "plugin" (17 Nov). An app with several launcher icons (app entrypoints, in code) is still just an app to them.
- Version numbers show only in the App Store (17 Nov).
- Reuse a screen before building one (17 Nov): an app with no screen of its own opens its App Store page.
- When something can't be done, it says why where the person tries it (17 Nov): Remove App greyed, with "This is Family's, and you're not one of its owners." Never a missing button with no reason.
- Removing anything asks first, in an alert that names what goes with it (17 Nov), as iOS's Remove App does.
- Admin things (infrastructure apps, managing users) show only to the people who can act on them, the network's owners (17 Nov). Everyone else never sees them.
- A section header's See All sits on the title's baseline, smaller than the title, ending at the content's edge (17 Nov), as iOS's App Store does.

## Home Screen

- Only Home is per owner (17 Nov): each owner (You first, then Everyone, then your groups) has several Home pages side by side. After them come one App Library and one App Store, for everyone you can reach: the App Store simply the next page, with a big "App Store" title. Home, the App Library and the App Store are its three sections, each at least a whole screen (17 Nov): a swipe moves one screenful, through Home's and on to the App Library, then the App Store.
- A page has no title at its top, and keeps the gap a phone's own home screen has.
- Pages never scroll: a page is a fixed grid, and what doesn't fit goes on a new page. Widgets never scroll either: a fixed number of rows, and tapping one opens its app.
- Left and right move between an owner's pages. Up and down on Home moves between owners, sliding vertically; in the App Library and the App Store it scrolls them. Each owner stays on the page it was left on.
- The bar is one small floating element, fixed at the bottom in the middle, in iOS 26's floating-bar style (17 Nov), with no accessory and no words: Home's house, ⊞ and the App Store's store, always there, the section shown in a pill of its own. On Home, the owner switcher (the owner's name and its chevron) and the page dots sit inside Home's pill, right after the house, so the App Library reads as what's to the right: `[ ⌂ You ⌃ ●○○ ] [⊞] [🏪]`, `[ ⌂ You ⌃ ] [⊞] [🏪]` with one page; in the App Library or the App Store Home's pill folds back to the house: `[⌂] [ ⊞ ] [🏪]`. It sits above the home indicator, and the top is given back to the pages. Each owner has its own number of dots, none with one page. As the owner changes, the same motion that rolls the name rolls the dots, so the next owner's dots swap in with their name. Long names are cut short. Tapping the switcher opens a list of every owner; picking one slides there. Sliding along the dots moves between pages too.
- The App Library holds every app you can reach, once, whether it's installed for you, a group or everyone (17 Nov), by category as an iPhone's App Library: four small icons, one opening its app, anywhere else opening the category. It's one full-width page that scrolls up and down, as iOS's (17 Nov): its search at the top, then its categories in a grid in the categories' order, two across on a phone and more on a wide screen; no pages and no dots. In the bar it's ⊞. Every installed app is there (17 Nov), an app with no entrypoint of its own too, which opens its App Store page. An app's hold menu says who it's installed for. What only the network's owners manage (Database, Encryption, Sandbox, Permissions) is in Admin, which only they see. An opened category has no owner above it, and grows from its place and back into it, as an iOS folder does.
- New apps land in the App Library, not on a page: pages hold only what's put there. An app about you (Colour Mode, Vault, Sign-In & Security, Devices, Notifications, Connect Your AI, AI Usage and the device apps) is in your App Library even when it's installed for everyone, and no app is in two.
- Desktop and tablet: no sidebar. As many whole pages side by side as fit, every one of them on an ultrawide. A screenful with fewer pages has them in its middle. Each is exactly the phone's page, nothing on it moved, drawn larger to suit the window's height, at most twice, so a 4K screen shows another page rather than bigger icons (17 Nov). Up and down moves between owners, and the same owner switcher works. Nothing there the phone doesn't have.
- Editing Home pages follows iOS's own, in its gestures, states and look. Touch and hold an app or empty space, then Edit Home Screen (right-click on a computer). Icons jiggle, each with ⊖, which takes it off the Home Screen and leaves it in the App Library. Drag to reorder, and hold at a page's edge to move to the next page, a new one past the last. + opens the widget gallery: pick a widget and its size, then place it. Tapping the page dots opens the page overview, to reorder, hide or delete Home pages. + is at the top left and Done at the top right, as iOS's. Done, or a tap on empty space, leaves. From the App Library, touch and hold an app, then drag it onto any owner's Home pages (17 Nov): Home slides in on the owner you were last on, holding at a side edge turns the page and at the top or bottom edge the owner, for an app already on a page too. Dropped where it's installed, it's placed; where it isn't but you may install it, a sheet asks "Install <App> for <Owner>?", and Install installs it there, its progress on its icon where it was dropped, or puts it back in the App Library saying why it couldn't; where you may not, a sheet says you can't add apps for them and who can, and it goes back. Moving an icon never uninstalls anything, and Done keeps every owner's pages as edited. A computer has the same edit mode, as Launchpad does.
- Uninstalling is in the App Library (17 Nov): touch and hold an app, then Remove App, which asks in an alert naming what goes with it ("Remove Sign-In? Devices, Users & Groups and Connect Your AI are part of it and will be removed too.") and puts it in Recently Deleted. Where you may not, Remove App is greyed, with why under it: "This is Family's, and you're not one of its owners."
- Whoever may arrange an owner's pages edits them; anyone else gets only the app's own menu on touch and hold, with no Edit Home Screen. An owner's pages are one layout: editing everyone's or a group's changes them for everyone who sees them.
- Home Screen behaviour copies iOS's unless there's a reason not to (17 Nov): paging, long press into edit mode, the hold menu, drag with the other icons reflowing, haptics, and an alert before removing.
- An app opened from its icon grows out of it and closes back into it, as iOS's (17 Nov); one opened any other way slides in. For whoever asked for reduced motion, either fades in place.
- The page dots are all one size, the current one brighter, as iOS's (17 Nov).
- In the bar each section is an icon (17 Nov): Home a house, the App Library ⊞, the App Store a store. Tapping an icon goes to the start of its section, a dot to its screenful.
- Editing moves no icon (17 Nov): on a phone the status bar hides and + and Done take its place, as iOS's; where there's no status bar, the strip above the first row is tall enough for them, so they never cover an icon or its ⊖.
- Big app icons on pages have their name under them (17 Nov), cut with "…" when long. The cards stay square: the icon shrinks to make room. Why: our icons aren't as well known as iOS's, so the name is needed.
- Nothing is hidden (17 Nov): the App Library always shows everything installed, and Remove from Home Screen only takes an app off a page.
- Settings isn't a place: its apps are ordinary apps, each in its category.
- Widgets come in a phone's own sizes: 2×2, 4×2 and 4×4. A widget's title is low in its order.

## Vault

- The one place for everything a person has given access to, in sections: Asked for, Passwords and keys, Accounts, then one for each other kind of item an app keeps there (Chrome Profiles).
- Accounts are every app's, in one list, each with its app's logo; Add Account under them.
- An item is ready, busy or action_required, as its app says: ready shows nothing; busy shows a spinner and what's under way; action_required shows a dot, why and the one action that fixes it (Sign In, Try Again). Signing out is that, or deleting the item.
- Deleting an item is soft delete's, as for anything an app made: it's in Recently Deleted for 30 days.
- An item an app keeps opens beside the list on a desktop, drawn by that app itself (a Chrome profile by Chrome Profiles).

## App Store (17 Nov)

- Browsing and getting apps, quiet: managing what's installed is the App Library's (17 Nov), so there's no Installed.
- Marketplace: one row for each of the nine app categories, cut so the next app shows, with See All; apps the size they are on the Home Screen.
- An app's page has a row for each owner, with Install, Open and its state there, its version where it's installed; then Requires, each requirement's title and why, never "What you get", and Required by (17 Nov); then See All.
- Trusted and untrusted are said plainly. An update available, a failed install with why and Try Again, and Recently Deleted for what was uninstalled.
- Install installs what it requires too, saying why. Uninstalling one that others require warns first that they go too.
- Uninstalling puts an app and its data in Recently Deleted, as one item, whose Restore brings both back for 30 days.
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
- No launcher: "There's no Home Screen yet." with Open App Store, or "Only Andre can install a Home Screen for Everyone" where the person can't.
- A launcher that doesn't draw: "Home Screen didn't open." with Try Again and Show Details.
- Update is an icon top right on Home pages, with the word beside it on desktop as Chrome's Update pill, shown only when an update for this device is ready.

## Desktop windows

- A Mac window has an overlay title bar with one fixed sidebar button. Windows and Linux keep the system's bar, tinted.

## Git Repos

- A repo's page follows GitHub's layout.
