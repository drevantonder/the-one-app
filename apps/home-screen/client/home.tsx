// The Home Screen: two sections side by side, Home and the App Library, as an iPhone's. Home is every owner's pages:
// yours, everyone's, then each of your groups', one owner above another, each a row of screenfuls, each as many of
// their pages side by side as fit, a phone's page drawn larger to suit the window's height on a wide one, and centred
// where there are fewer (layout.ts). The App Library, every app you can reach, by category, is one page, for everyone,
// a screen wide, that scrolls up and down. Left and right moves a screenful, as a phone's pages do, through Home's
// screenfuls and on to the App Library; up and down on Home moves between owners. The pages move by hand, as a
// phone's do (pager.ts). On Home, a small pill at the top in the middle says whose pages they are and opens a list of
// every owner. At the bottom, just clear of the home indicator, the dots pill has the owner's page dots, then the App
// Library's glyph, lit there. The App Store is an app like any other, in the App Library. A page never scrolls: it's a
// fixed grid, and what doesn't fit goes on to the next. Whoever arranges an owner's pages edits them as a phone's Home
// Screen is edited: touch and hold an app or a widget (press and hold it, or right-click it, with a mouse), then Edit
// Home Screen, or hold it a while longer, or touch and hold a page's empty space. An app held in the App Library is
// carried onto any owner's pages. Where it is, what's open over it and the editing are one state machine (machine.ts),
// whose snapshot is one signal here. It's written on the UI library's primitives, so a native host draws it as the DOM does.
import { createEffect, createMemo, Errored, Loading, refresh, Show } from "solid-js"
import { Button, covers, haptic, Problem, render, View, viewport } from "ui"
import { looks } from "./looks.ts"
import { home, state, editing, launcher, windowWidth } from "./store.ts"
import { type Owner, owners } from "./owners.ts"
import { OpenCategory } from "./category.tsx"
import { Holding } from "./holding.tsx"
import { Overview } from "./overview.tsx"
import { Gallery, Asking, Refused, EditBar } from "./editing.tsx"
import { createLauncher, LauncherPanel, launcherKeys } from "./launcher.tsx"
import { Phone } from "./phone.tsx"
import { createScreens } from "./screens.ts"
import { createPaging } from "./paging.ts"
import { Carried, createCarry } from "./carry.tsx"

function HomeScreen(props: { owners: Owner[] }) {
  const layout = createScreens(props)
  const paging = createPaging(props, layout)
  const launching = createLauncher(props, layout, paging)
  const carrying = createCarry(props, layout, paging)
  const { editBar, editBarGoing, overview, overviewGoing, library } = layout
  const { cardRect, found, openTarget, move, toggleLibrary } = launching
  const { copy } = carrying
  // Editing begins with a tick, as an iPhone's does; lifting something into it plays its own.
  createEffect(
    createMemo(() => state().matches("editing")),
    (now) => void (now && home.getSnapshot().matches({ editing: "still" }) && haptic("selection")),
    { defer: true },
  )
  // While pages are edited, the device's own back ends editing, as Done does, and the window's rail keeps out of the
  // way, as iOS's chrome does while its icons are rearranged: something of the Home Screen's own is open over it.
  createEffect(
    createMemo(() => state().matches("editing")),
    (now) => (now ? covers(() => home.send({ type: "done" })) : undefined),
  )

  const opened = () => state().context.category
  const held = () => (state().matches("holding") ? state().context.held : undefined)
  const owner = () => props.owners[state().context.at.owner]
  return (
    <View
      onKeyDown={launcher() ? launcherKeys({ move, open: openTarget, toggleLibrary, library, found: () => found().length }) : undefined}
      style={[looks.root, looks.sized(windowWidth(), viewport().height)]}
    >
      <Show
        when={launcher()}
        fallback={<Phone owners={props.owners} layout={layout} paging={paging} />}
      >
        <LauncherPanel owners={props.owners} layout={layout} paging={paging} launching={launching} />
      </Show>
      <Show when={editBar()}>
        <EditBar going={editBarGoing()} within={launcher() ? cardRect() : undefined} />
      </Show>
      <Show when={copy()}>{(now) => <Carried now={now} owner={owner()} />}</Show>
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[0]} opened={opened()} within={launcher() ? cardRect() : undefined} />}</Show>
      <Show when={held()}>{(held) => <Holding owner={owner()} index={state().context.at.owner} held={held()} />}</Show>
      <Show when={overview()}>
        {(edited) => <Overview owner={props.owners[edited().owner]} index={edited().owner} going={overviewGoing()} />}
      </Show>
      <Show when={editing()}>{(edited) => <Gallery owner={props.owners[edited().owner]} />}</Show>
      <Show when={state().matches("editing") && !state().context.held?.item?.widget && state().context.held?.item && state().context.held}>
        {(held) => (
          <>
            <Asking owner={owner()} held={held()} />
            <Show when={owner().subject}>
              <Refused owner={owner()} held={held()} />
            </Show>
          </>
        )}
      </Show>
    </View>
  )
}

/// The Home Screen didn't draw: what DESIGN.md says of a launcher that doesn't, with Try Again, which asks for it
/// again, and what went wrong under it. Its client loads it again by itself once a newer version is out, as the older
/// one meeting what's new is the likeliest cause.
function Broken(props: { error: Error }) {
  return (
    <View style={[looks.broken, looks.tall(viewport().height)]}>
      <Problem
        heading="Home Screen didn't open."
        text={new Error(`Try again in a moment.\n\n${typeof props.error?.cause === "string" ? props.error.cause : (props.error?.message ?? props.error)}`)}
        action={<Button variant="secondary" onPress={() => refresh(owners)}>Try Again</Button>}
      />
    </View>
  )
}

render(() => (
  <Errored fallback={(error) => <Broken error={error() as Error} />}>
    <Loading>
      <HomeScreen owners={owners()} />
    </Loading>
  </Errored>
))
