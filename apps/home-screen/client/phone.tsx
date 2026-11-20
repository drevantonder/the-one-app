// A phone's Home Screen: every owner's pages one above another, each a row of screenfuls, the App Library after them,
// and the owner pill and dots over them.
import { For, untrack } from "solid-js"
import { View } from "ui"
import { lefts, TOP_ROW } from "./layout.ts"
import { looks } from "./looks.ts"
import { state, room, laidPage } from "./store.ts"
import { type Owner } from "./owners.ts"
import { HomePage } from "./pages.tsx"
import { Library } from "./library.tsx"
import { OwnerPill, DotsPill } from "./pills.tsx"
import { type Screens } from "./screens.ts"
import { type Paging } from "./paging.ts"

export function Phone(props: { owners: Owner[]; layout: Screens; paging: Paging }) {
  const { homes, pages, laid, library } = props.layout
  const { downY, xOf, opacityOf, sectionsX, swipes, along, wheel } = props.paging
  // The owner shown, how far along their pages and the App Library are, and how wide their Home is.
  const shown = () => state().context.at.owner
  const across = () => xOf(shown())[0]()
  const homeWidth = (owner: number) => Math.max(1, homes(owner).length) * room().width
  // Each other owner's pages are moved so the page they were last on is where the shown owner's is, ready for a swipe
  // up or down to them.
  const lined = (owner: number) => (owner === shown() ? 0 : across() - Math.min(xOf(owner)[0](), homeWidth(owner) - room().width))
  // The strip slides by its spring as it goes along, but not as another owner comes to be shown in its place, whose
  // pages are lined up where they go already.
  let was = untrack(shown)
  const slides = () => {
    const now = shown()
    const same = now === was
    was = now
    return same
  }
  return (
    <>
        {/* Home's pages and the App Library after them, in one strip the host drags along as the finger goes (`along`)
            and slides to where they're going by its spring. */}
        <View
          {...swipes}
          {...along}
          onWheel={wheel}
          style={[looks.sections, slides() && looks.paging, looks.tall(room().height), looks.shifted(-across())]}
        >
          <View role="main" aria-label="Home" style={[looks.clipped, looks.rigid, looks.sized(homeWidth(shown()), room().height)]}>
            <View style={[looks.owners, looks.lowered(-downY())]}>
              <For each={props.owners} keyed={(owner) => owner.key}>
                {(owner, index) => (
                  <View role="region" aria-label={owner().name} style={[looks.sized(homeWidth(index()), room().height)]}>
                    <View style={[looks.screens, looks.slid(lined(index()), opacityOf(index())[0]())]}>
                      <For each={homes(index())} keyed={false}>
                        {(screen) => (
                          <View style={[looks.slots, looks.sized(room().width, room().height)]}>
                            <For each={(pages()[index()] ?? []).slice(screen().from, screen().from + screen().count)} keyed={false}>
                              {(placed, at) => (
                                <View
                                  style={[looks.absolute, looks.placedPage((laidPage().height * (room().scale - 1)) / 2, lefts(room(), laidPage(), screen().count)[at] + (laidPage().width * (room().scale - 1)) / 2, room().scale)]}
                                >
                                  <HomePage owner={owner()} index={index()} page={screen().from + at} placed={placed()} laid={laidPage()} scale={room().scale} />
                                </View>
                              )}
                            </For>
                          </View>
                        )}
                      </For>
                    </View>
                  </View>
                )}
              </For>
            </View>
          </View>
          <Library owner={props.owners[0]} />
        </View>
        {/* Whose pages they are, over Home's, which goes with them as they slide off to the App Library. */}
        <View inert={library() || undefined} style={[looks.topRow, looks.paging, looks.top(room().safe.top), looks.tall(TOP_ROW), looks.shifted(-sectionsX())]}>
          <View style={[looks.live, looks.opacity(state().matches("editing") ? 0.6 : 1)]}>
            <OwnerPill owners={props.owners} />
          </View>
        </View>
        <View style={[looks.bottomRow, looks.bottom(room().safe.bottom + 16)]}>
          <View style={looks.live}>
            <DotsPill owners={props.owners} laid={laid().screens} />
          </View>
        </View>
    </>
  )
}
