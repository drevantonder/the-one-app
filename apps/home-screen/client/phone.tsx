// A phone's Home Screen: every owner's pages one above another, each a row of screenfuls, the App Library after them,
// and the owner pill and dots over them.
import { For } from "solid-js"
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
  const { downY, xOf, opacityOf, sectionsX, swipes, wheel } = props.paging
  return (
    <>
        <View
          {...swipes}
          onWheel={wheel}
          style={[looks.sections, looks.sized(room().width * 2, room().height), looks.shifted(-sectionsX())]}
        >
          <View role="main" aria-label="Home" style={[looks.clipped, looks.rigid, looks.sized(room().width, room().height)]}>
            <View style={[looks.owners, looks.wide(room().width), looks.lowered(-downY())]}>
              <For each={props.owners} keyed={(owner) => owner.key}>
                {(owner, index) => (
                  <View role="region" aria-label={owner().name} style={[looks.clipped, looks.sized(room().width, room().height)]}>
                    <View style={[looks.screens, looks.slid(-Math.min(xOf(index())[0](), Math.max(0, homes(index()).length - 1) * room().width), opacityOf(index())[0]())]}>
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
        <View inert={library() || undefined} style={[looks.topRow, looks.top(room().safe.top), looks.tall(TOP_ROW), looks.shifted(-sectionsX())]}>
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
