import { negative, styles, tokens } from "ui"
import { sizes } from "./sizes.ts"

/// The Home Screen's own looks, by the design tokens' roles: Liquid Glass for its pills, + and Done, what's opened or
/// held over a frosted Home Screen, and the window's shadow on what floats.
export const looks = styles({
  cover: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0 },
  frosted: { backgroundColor: tokens.colour.veil, backdropFilter: tokens.material.thin },
  pill: { flexDirection: "row", alignItems: "center", borderRadius: tokens.radius.full },
  // The owner pill is its button, whose press and open fill it whole.
  ownerPill: { paddingInline: 0 },
  glass: { paddingInline: tokens.grid[1], pointerEvents: "auto", backgroundColor: tokens.colour.glass, boxShadow: tokens.shadow.glass, backdropFilter: tokens.material.glass },
  menu: { position: "absolute", overflow: "hidden", borderRadius: tokens.radius.large, backgroundColor: tokens.colour.glass, backdropFilter: tokens.material.glass, boxShadow: tokens.shadow.window, color: tokens.colour.label },
  overview: { position: "absolute", zIndex: 15, backgroundColor: tokens.colour.veil, backdropFilter: tokens.material.thin, transitionProperty: ["opacity"], transition: tokens.motion.settle },
  gone: { opacity: 0 },
  // Fades in as it's added, and out where `gone` says, at the settle pace.
  fadeIn: { opacity: { default: 1, entering: 0 }, transitionProperty: ["opacity"], transition: tokens.motion.settle },
  slots: { position: "relative" },
  slot: { position: "absolute", top: 0, start: 0, zIndex: 0, alignItems: "center", gap: tokens.grid[3] },
  carried: { zIndex: 1 },
  gliding: { transitionProperty: ["translateX", "translateY"], transition: tokens.motion.settle },
  miniature: { position: "relative", overflow: "hidden", borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.background, boxShadow: tokens.shadow.window },
  // An icon sinks under the finger, dimmed at once, for as long as it takes to hold, and comes back quickly.
  icon: { position: "relative", scale: { default: 1, pressed: 0.94 }, opacity: { default: 1, pressed: 0.85 }, transitionProperty: ["scale"], transition: tokens.motion.hold },
  tint: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, borderRadius: tokens.radius.large, backgroundColor: tokens.colour.tile },
  round: { borderRadius: tokens.radius.window },
  // The launcher's search is bare words, with no box, magnifier or ring, as Raycast's is.
  bare: { flexGrow: 1, height: tokens.length.control, font: tokens.type.large, color: tokens.colour.label },
  // The launcher's search and its card, each a surface of the system's glass floating over the desktop, with a
  // window's shadow and edge.
  // Held where nothing in them takes a press, they move the panel, as Raycast's is dragged.
  surface: { position: "absolute", overflow: "hidden", borderRadius: tokens.radius.window, backgroundColor: tokens.colour.panel, backdropFilter: tokens.material.window, boxShadow: tokens.shadow.window, appRegion: "drag" },
  // The card changes size by a spring, which a change on the way turns, and what's in it is carried with it.
  card: { transitionProperty: ["width", "height", "translateX"], transition: tokens.motion.snappy },
  // Home, the App Library and what was found cross-fade in the card, each drawn ahead as it's about to (`willChange`);
  // an owner's pages roll up and down.
  fading: { transitionProperty: ["opacity", "scale", "translateY"], transition: tokens.motion.smooth, willChange: ["opacity", "scale", "translateY"] },
  rolling: { transitionProperty: ["opacity", "translateY"], transition: tokens.motion.snappy },
  search: { flexDirection: "row", alignItems: "center", zIndex: 6 },
  owner: { position: "absolute", overflow: "hidden" },
  track: { position: "absolute", top: 0, start: 0, flexDirection: "row" },
  // What goes with the pages without being dragged with them: sliding with them by the same spring, and fading out of
  // their way.
  sliding: { transitionProperty: ["translateX", "opacity"], transition: tokens.motion.paging },
  layer: { position: "absolute", alignItems: "center" },
  row: { position: "absolute", start: 0, end: 0, alignItems: "center", justifyContent: "center" },
  inert: { pointerEvents: "none" },
  // What a finger carries floats over the pages on a soft shadow, which fades as it settles.
  lifted: { position: "absolute", boxShadow: tokens.shadow.lifted, opacity: 1, transitionProperty: ["opacity"], transition: tokens.motion.settle },
  liftedIcon: { borderRadius: tokens.radius.icon },
  liftedWidget: { borderRadius: tokens.radius.large },
  // What a page measures and moves as it runs: where it is, how big, how opaque, how far moved.
  at: (start: number, top: number, width: number, height: number) => ({ start, top, width, height }),
  fill: (top: number, start: number, end: number) => ({ top, start, end }),
  top: (top: number) => ({ top }),
  bottom: (bottom: number) => ({ bottom }),
  square: (size: number) => ({ width: size, height: size }),
  wide: (width: number) => ({ width }),
  tall: (height: number) => ({ height }),
  atLeast: (minHeight: number) => ({ minHeight }),
  scaled: (width: number, height: number, scale: number) => ({ width, height, scale }),
  opacity: (opacity: number) => ({ opacity }),
  faded: (opacity: number, scale: number) => ({ opacity, scale }),
  drifted: (opacity: number, translateY: number) => ({ opacity, translateY }),
  shifted: (translateX: number) => ({ translateX }),
  moved: (translateX: number, translateY: number) => ({ translateX, translateY }),
  popup: (start: number, width: number) => ({ start, width }),
  chosen: { position: "absolute", top: negative(tokens.grid[1]), bottom: negative(tokens.grid[1]), start: negative(tokens.grid[1]), end: negative(tokens.grid[1]), borderRadius: tokens.radius.large, backgroundColor: tokens.colour.selection },
  sized: (width: number, height: number) => ({ width, height }),
  padded: (paddingTop: number, paddingBottom: number) => ({ paddingTop, paddingBottom }),
  padBottom: (paddingBottom: number) => ({ paddingBottom }),
  lowered: (translateY: number) => ({ translateY }),
  turned: (rotate: number) => ({ rotate }),
  absolute: { position: "absolute" },
  rigid: { flexShrink: 0 },
  shrinks: { flexShrink: 1 },
  narrow: { maxWidth: "100%" },
  centred: { alignItems: "center" },
  live: { pointerEvents: "auto" },
  smaller: { scale: 0.75 },
  note: { font: tokens.type.caption, color: tokens.colour.secondaryLabel },
  destructive: { color: tokens.colour.red },
  // The whole of it: the window, its two sections side by side, each owner's pages over another's, and a screenful.
  root: { position: "relative", overflow: "hidden", color: tokens.colour.label },
  sections: { position: "absolute", top: 0, start: 0, flexDirection: "row" },
  // The pages and what goes with them, slid to where they're going by the host's spring, UIScrollView's paging's.
  paging: { transitionProperty: ["translateX"], transition: tokens.motion.paging },
  clipped: { position: "relative", overflow: "hidden" },
  owners: { position: "absolute", top: 0, start: 0 },
  screens: { position: "absolute", top: 0, start: 0, flexDirection: "row" },
  placedPage: (top: number, start: number, scale: number) => ({ top, start, scale }),
  topRow: { position: "absolute", zIndex: 6, start: 0, end: 0, alignItems: "center", justifyContent: "center", pointerEvents: "none" },
  bottomRow: { position: "absolute", zIndex: 6, start: 0, end: 0, alignItems: "center", pointerEvents: "none" },
  broken: { justifyContent: "center", padding: tokens.grid[5] },
  // A page's apps and widgets: each where it's laid, moved as it's carried and swung as it jiggles.
  widgetCell: { position: "relative", display: "grid", flexGrow: 1 },
  count: { position: "absolute", top: negative(tokens.grid[1.5]), end: negative(tokens.grid[1.5]) },
  tileBox: { alignItems: "center", gap: sizes.iconGap, minWidth: 0, width: "100%" },
  // An app's icon, `size` across: corners from `corner`, the 18 points a Home Screen icon of 60 has, so a small icon in an App Library category is the same shape.
  iconImage: (borderRadius: number) => ({ borderRadius }),
  name: { font: tokens.type.caption, lineClamp: 1, textAlign: "center", maxWidth: "100%" },
  smallName: { font: tokens.type.small },
  // An empty page's words, inside its margins and in its middle, as iOS's ContentUnavailableView is.
  empty: (top: number, start: number, end: number, bottom: number) => ({ position: "absolute", top, start, end, bottom, alignItems: "center", justifyContent: "center", textAlign: "center" }),
  placed: (start: number, top: number, width: number, height: number, translateX: number, translateY: number, rotate: number, opacity: number) => ({ position: "absolute", start, top, width, height, translateX, translateY, rotate, opacity }),
  installing: { position: "absolute", borderRadius: tokens.radius.full, backgroundColor: tokens.colour.overlay, color: tokens.colour.background, alignItems: "center", justifyContent: "center" },
  minus: { position: "absolute", zIndex: 2, borderRadius: tokens.radius.full, backgroundColor: tokens.colour.secondaryLabel, color: tokens.colour.background, alignItems: "center", justifyContent: "center" },
  // The App Library: its categories, each a square of its first apps, as many across as fit (`shelf`), with the gap
  // between them in points, as iOS's are, and what's found.
  categories: (gap: number) => ({ flexDirection: "row", flexWrap: "wrap", gap }),
  category: { alignItems: "center", gap: tokens.grid[1.5], minWidth: 0 },
  folder: { position: "relative", display: "grid", gridTemplateColumns: { repeat: 2 } },
  folderBox: (columnGap: number, rowGap: number, size: number, paddingTop: number, paddingInline: number, paddingBottom: number) => ({ columnGap, rowGap, width: size, height: size, paddingTop, paddingInline, paddingBottom }),
  more: { display: "grid", gridTemplateColumns: { repeat: 2 }, gap: sizes.folderGap, alignSelf: "center", justifyContent: "center", marginInline: "auto" },
  categoryName: { font: tokens.type.caption, color: tokens.colour.secondaryLabel, lineClamp: 1 },
  found: { display: "grid", gridTemplateColumns: { fill: sizes.found }, rowGap: tokens.grid[4], columnGap: tokens.grid[3] },
  library: { flexShrink: 0 },
  libraryScroll: { flexGrow: 1, flexBasis: 0 },
  libraryPadding: (paddingInline: number, paddingTop: number, paddingBottom: number) => ({ paddingInline, paddingTop, paddingBottom }),
  libraryColumn: { gap: tokens.grid[5], marginInline: "auto" },
  background: { gap: tokens.grid[3] },
  fold: { flexDirection: "row", alignItems: "center", alignSelf: "flex-start", gap: tokens.grid[1], marginBlock: tokens.grid[1], paddingBlock: tokens.grid[1], color: tokens.colour.secondaryLabel },
  foldName: { font: tokens.type.medium },
  chevron: { scale: 0.875 },
  entry: { alignItems: "center", gap: tokens.grid[1.5], minWidth: 0 },
  entrySmall: { gap: tokens.grid[1] },
  // An opened category, and what's held with its menu.
  dialog: { position: "absolute", top: 0, start: 0 },
  categoryLayer: { zIndex: 10 },
  heldLayer: { zIndex: 20 },
  // It grows out of its square and shrinks back into it, and the veil over what's behind blurs and dims with it, on
  // one spring, which a change on the way turns from where it is.
  veils: { opacity: { default: 1, entering: 0 }, transitionProperty: ["opacity"], transition: tokens.motion.smooth },
  grows: { position: "absolute", pointerEvents: "none", scale: { default: 1, entering: 0.3 }, opacity: { default: 1, entering: 0 }, transitionProperty: ["scale", "opacity"], transition: tokens.motion.smooth },
  // Closed, kept made in its square for the next time it opens.
  unveiled: { opacity: 0 },
  shrunk: { scale: 0.3, opacity: 0 },
  openedArea: { position: "absolute", overflow: "hidden", pointerEvents: "auto" },
  opened: { gap: tokens.grid[8] },
  categoryTitle: { font: tokens.type.largeTitle },
  // Its apps as Home's, spread across, four on a phone.
  openedApps: { display: "grid", gridTemplateColumns: { fill: sizes.found }, justifyContent: "space-between", rowGap: tokens.grid[4], columnGap: tokens.grid[3] },
  copy: { position: "absolute", display: "grid", pointerEvents: "none" },
  // What's held grows a little as it's lifted, and a menu opens from small, each with a spring.
  lifting: { display: "grid", scale: { default: 1.06, entering: 1 }, transitionProperty: ["scale"], transition: tokens.motion.snappy },
  // What a finger carries is raised from where it was lifted, and put back down as it settles.
  raised: { display: "grid", scale: { default: 1.12, entering: 1 }, transitionProperty: ["scale"], transition: tokens.motion.snappy },
  raisedWidget: { display: "grid", scale: { default: 1.05, entering: 1 }, transitionProperty: ["scale"], transition: tokens.motion.snappy },
  set: { scale: 1 },
  opens: { opacity: { default: 1, entering: 0 }, scale: { default: 1, entering: 0.25 }, transitionProperty: ["opacity", "scale"], transition: tokens.motion.snappy },
  copyAt: (start: number, top: number, width: number, height: number, scale: number) => ({ start, top, width, height, scale }),
  installedFor: { paddingInline: tokens.grid[4], paddingBlock: tokens.grid[2.5], borderBottomWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  holdItem: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: tokens.grid[4], minHeight: sizes.menuRow, paddingInline: tokens.grid[4], borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  holdWhy: { paddingBlock: tokens.grid[2.5], color: tokens.colour.secondaryLabel },
  // Adding a widget: its sizes, each a preview, the one chosen ringed.
  widgetIcon: { width: tokens.grid[8], height: tokens.grid[8], borderRadius: tokens.radius.medium },
  sizes: { gap: tokens.grid[4] },
  sizesRow: { flexDirection: "row", gap: tokens.grid[4], padding: tokens.grid[1] },
  sizeOption: { alignItems: "center", gap: tokens.grid[2], flexShrink: 0, color: tokens.colour.secondaryLabel },
  sizeChosen: { color: tokens.colour.label },
  preview: { display: "grid", padding: tokens.grid[0.5], borderRadius: tokens.radius.large, borderWidth: 0, borderColor: tokens.colour.secondaryLabel, pointerEvents: "none" },
  previewChosen: { borderWidth: tokens.length.hairline },
  sizeName: { font: tokens.type.caption },
  // The pages' overview: Done, and each page's miniature with its tick.
  overviewBar: { flexDirection: "row", justifyContent: "flex-end", alignItems: "center", paddingInline: tokens.grid[4] },
  topRowAt: (paddingTop: number, height: number) => ({ paddingTop, height }),
  overviewScroll: { flexGrow: 1, flexBasis: 0, paddingTop: tokens.grid[4], paddingInline: tokens.grid[6] },
  thumb: { position: "absolute", pointerEvents: "none" },
  thumbAt: (start: number, top: number, scale: number, opacity: number) => ({ start, top, scale, opacity }),
  check: { width: tokens.grid[7], height: tokens.grid[7], alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.full, borderWidth: tokens.length.hairline, borderColor: tokens.colour.label, backgroundColor: tokens.colour.label, color: tokens.colour.background },
  unchecked: { borderColor: tokens.colour.secondaryLabel, backgroundColor: tokens.colour.transparent },
  // The pills: the owner's name, rolling, and the page dots.
  roll: { position: "relative", overflow: "hidden", alignItems: "center" },
  rollSizer: { opacity: 0, justifyContent: "center" },
  dots: { flexDirection: "row", alignItems: "center", height: "100%" },
  dotButton: { height: "100%", justifyContent: "center", paddingInline: tokens.length.hairline },
  dot: { borderRadius: tokens.radius.full, backgroundColor: tokens.colour.secondaryLabel },
  dotLit: { backgroundColor: tokens.colour.label },
  dotAt: (size: number, marginInline: number, opacity: number) => ({ width: size, height: size, marginInline, opacity }),
  ownerRow: { flexDirection: "row", alignItems: "center", gap: sizes.ownerGap },
  ownerName: { font: tokens.type.medium, fontWeight: tokens.weight.medium, lineClamp: 1, maxWidth: sizes.ownerName },
  caret: { color: tokens.colour.secondaryLabel, scale: 0.75 },
  ownerItem: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2] },
  libraryGlyph: { height: "100%", justifyContent: "center", paddingStart: tokens.grid[0.5], paddingEnd: tokens.grid[1], color: tokens.colour.secondaryLabel },
  lit: { color: tokens.colour.label },
  editBar: { position: "absolute", zIndex: 7, flexDirection: "row", alignItems: "center", justifyContent: "space-between", pointerEvents: "none" },
  editBarPad: (paddingInline: number) => ({ paddingInline }),
  // The launcher's search and what it found.
  launcherSearch: { flexGrow: 1, flexDirection: "row", alignItems: "center", paddingInline: sizes.searchInline },
  heading: { font: tokens.type.caption, color: tokens.colour.secondaryLabel, paddingInline: tokens.grid[1] },
  results: { gap: tokens.grid[3] },
  nothingFound: { paddingBlock: tokens.grid[6] },
  apps: { flexDirection: "row", gap: tokens.grid[3], alignItems: "flex-start" },
  lead: { flexShrink: 0, display: "grid" },
  foundApps: { flexGrow: 1, flexDirection: "row", flexWrap: "wrap", rowGap: tokens.grid[3] },
  result: { position: "relative", width: sizes.found, height: sizes.foundHeight, paddingTop: tokens.grid[1.5] },
  widgets: { flexDirection: "row", flexWrap: "wrap", gap: tokens.grid[3] },
  widgetCard: { display: "grid" },
  // Installing an app for an owner, or being told you can't.
  ask: { alignItems: "center", gap: tokens.grid[3] },
  askIcons: { flexDirection: "row", flexWrap: "wrap", justifyContent: "center", gap: tokens.grid[3] },
  askIcon: { width: tokens.grid[16], height: tokens.grid[16], borderRadius: tokens.radius.large },
  askWhy: { color: tokens.colour.secondaryLabel, textAlign: "center" },
  askButtons: { alignSelf: "stretch", gap: tokens.grid[3], paddingTop: tokens.grid[2] },
  askButton: { alignSelf: "stretch", paddingTop: tokens.grid[2] },
  // What a finger carries, drawn from where it was lifted.
  carry: { position: "absolute", zIndex: 30, display: "grid", pointerEvents: "none" },
  carryAt: (start: number, top: number, width: number, height: number, translateX: number, translateY: number, scale: number) => ({ start, top, width, height, translateX, translateY, scale }),
})
