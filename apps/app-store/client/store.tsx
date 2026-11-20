// App Store: every app the network can install, a row of each category. An app's page has Get, which installs it, and
// a row for each owner, with where its install is for them and what to do about it: managing what's installed is
// otherwise the App Library's. A link opens it at an app's page (`one:app-store#app=<its artifact>`), as the App
// Library does for an app with no screen of its own.
import { createEffect, createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch } from "solid-js"
import type { Element } from "solid-js"
import { Button, call, changed, confirm, Empty, go, Icon, Image, link, List, Menu, negative, open, Pressable, Problem, Row, Screen, ScrollView, Search, Section, Skeleton, Spinner, styles, Text, toast, tokens, View } from "ui"
import { sizes } from "./sizes.ts"

const look = styles({
  /// An app's tile: its icon and its name under it, the size it is on a Home Screen, or as wide as a grid's column.
  tile: { alignItems: "center", gap: tokens.grid[1.5], width: sizes.tile, flexShrink: 0, borderRadius: tokens.radius.medium },
  fills: { width: "100%" },
  icon: { width: sizes.icon, height: sizes.icon, borderRadius: tokens.radius.large },
  largeIcon: { width: sizes.largeIcon, height: sizes.largeIcon, borderRadius: tokens.radius.large },
  /// Where an app has no icon.
  blank: { backgroundColor: tokens.colour.quinaryLabel },
  name: { font: tokens.type.caption, textAlign: "center", lineClamp: 1, width: "100%" },
  shelf: { gap: tokens.grid[3] },
  shelfHeading: { flexDirection: "row", alignItems: "baseline", justifyContent: "space-between", minHeight: tokens.length.control },
  heading: { fontWeight: tokens.weight.medium },
  more: { font: tokens.type.caption },
  // A row that scrolls sideways leaves room round what's in it for the focus ring, which it would clip.
  shelfApps: { flexDirection: "row", gap: tokens.grid[3], padding: tokens.grid[1], margin: negative(tokens.grid[1]) },
  shelves: { display: "grid", gridTemplateColumns: { fill: sizes.shelf }, rowGap: tokens.grid[7], columnGap: tokens.grid[10] },
  grid: { display: "grid", gridTemplateColumns: { fill: tokens.grid[20] }, rowGap: tokens.grid[6], columnGap: tokens.grid[3] },
  chips: { flexDirection: "row", gap: tokens.grid[2], padding: tokens.grid[1], margin: negative(tokens.grid[1]) },
  /// A category to choose, a pill: the one chosen is dark.
  chip: { flexShrink: 0, height: tokens.length.control, paddingInline: tokens.grid[4], alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.full, backgroundColor: tokens.colour.quinaryLabel, color: tokens.colour.label },
  chosen: { backgroundColor: tokens.colour.label, color: tokens.colour.background },
  quiet: { color: tokens.colour.secondaryLabel },
  busy: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2] },
  page: { gap: tokens.grid[6], maxWidth: tokens.length.readable },
  top: { flexDirection: "row", gap: tokens.grid[4] },
  about: { flexGrow: 1, flexShrink: 1, minWidth: 0, justifyContent: "space-between", gap: tokens.grid[1] },
  titles: { gap: tokens.grid[1] },
  title: { font: tokens.type.title, lineClamp: 2 },
  row: { flexDirection: "row" },
  requires: { display: "grid", gridTemplateColumns: { fill: tokens.grid[20] }, rowGap: tokens.grid[4], columnGap: tokens.grid[3] },
  tools: { flexDirection: "row", flexWrap: "wrap", columnGap: tokens.grid[4], rowGap: tokens.grid[1] },
  grows: { flexGrow: 1 },
  hidden: { display: "none" },
})

/** An app the network can install, as `app-store_marketplace` lists it. */
interface Listed {
  name: string
  app: string | null
  source: string | null
  title: string
  icon: string | null
  subtitle: string | null
  about: string | null
  category: string | null
  entrypoints: { name: string; icon: string | null }[]
  widgets: { name: string; sizes: string[] }[]
  tools: string[]
  trusted: boolean | null
  requires: { name: string; title: string; why: string | null }[]
}

/** An install, as `apps` lists it. */
interface Install {
  name: string
  for: string
  app: string | null
  state: "installing" | "installed" | "not_installed" | "recently_deleted" | null
  stage: string | null
  deleted: number | null
  trusted: boolean | null
  version: number | null
  update: { state: "available" | "updating" | "failed" } | null
  requires: Record<string, string>
  required_by: string[]
}

/** Whoever apps are installed for that this person sees: themselves, everyone and their groups, by the name a
 * person reads, with what's installed for them and who may install for them. */
interface Owner {
  subject?: string
  name: string
  who: string
  installs: Install[]
  installers: string[]
}

const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

const market = createMemo(() => json<{ categories: string[]; apps: Listed[] }>("app-store_marketplace"))

const owners = createMemo(async () => {
  const [groups, people] = await Promise.all([
    json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => []),
    json<{ people: { who: string; called?: string }[] }>("sign-in_everyone").then(({ people }) => people, (): { who: string; called?: string }[] => []),
  ])
  const called = (who: string) => people.find((person) => person.who === who)?.called ?? who.replace(/^users\//, "").replace(/^./, (first) => first.toUpperCase())
  const whose: { subject?: string; name: string }[] = [{ name: "You" }, { subject: "network", name: "Everyone" }, ...groups.map(({ group, called }) => ({ subject: group, name: called }))]
  const all = await Promise.all(
    whose.map(async (owner): Promise<Owner> => {
      const listed = await json<{ for: string; apps: Install[] }>("apps", {}, owner.subject)
      // Anyone installs for themselves; for a group or everyone, whoever may.
      const installers = owner.subject ? await json<{ users: string[] }>("permissions_users", { artifact: owner.subject, relation: "installer" }).then(({ users }) => users, (): string[] => []) : [listed.for]
      return { ...owner, who: listed.for, installs: listed.apps.filter((install) => install.for === listed.for), installers }
    }),
  )
  return { owners: all, me: all[0].who, called }
})
changed(() => (refresh(market), refresh(owners)))

type Owners = Awaited<ReturnType<typeof owners>>
type Market = Awaited<ReturnType<typeof market>>

/// Where the app is: every app, a category's every app, an app's page and its See All. Each step in is a step the Back
/// button and the device's back take out again. An app's page a link opened (`linked`) is where the App Store starts,
/// so it has no Back, as the Marketplace hasn't: the window's rail and the device's back leave it.
interface View {
  category?: string
  app?: string
  details?: boolean
  linked?: boolean
}
const [view, setView] = createSignal<View>({})
const to = (next: View) => go(() => setView({ ...next, linked: undefined }))
/// One step out: See All to the app's page, an app's page to the Marketplace as it was (its category stays chosen: a category is a filter of it, which changes it in place).
const out = () => go(() => setView((now) => (now.details ? { category: now.category, app: now.app } : now.app ? { category: now.category } : {})), { back: true })
/// The app a page is of: its artifact, or for one only the directory has, its source and name.
const key = (listed: Listed) => listed.app ?? `${listed.source}#${listed.name}`

/// What an install's app is, where the Marketplace doesn't list it: its name alone.
const listed = (market: Market, install: Install): Listed =>
  market.apps.find((listed) => listed.app === install.app) ?? { name: install.name, app: install.app, source: null, title: install.name, icon: null, subtitle: null, about: null, category: null, entrypoints: [], widgets: [], tools: [], trusted: install.trusted, requires: [] }

const and = (words: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(words)

/// Who may install for an owner, if not this person: "Only Andre can install for Everyone".
function only(all: Owners, owner: Owner) {
  if (owner.installers.includes(all.me) || owner.installers.includes("everyone")) return undefined
  const who = new Intl.ListFormat("en", { type: "disjunction" }).format(owner.installers.map(all.called))
  return owner.installers.length ? `Only ${who} can install for ${owner.name}` : `Nobody can install for ${owner.name}`
}

/// What's being done, by owner and app, while its call runs.
const [working, setWorking] = createSignal<string>()

/// Runs a call about an app for an owner, says what else it did (`also`: "Gmail uses Google Account to reach your
/// mail, so it's installed too."), and what went wrong, unless the install shows it itself.
async function act(owner: Owner, app: string | null, what: string, run: () => Promise<string>) {
  setWorking(`${owner.who}|${app}|${what}`)
  try {
    const done = JSON.parse(await run())
    if (done.also?.length) toast(done.also.join(" "))
  } catch (error) {
    const now = await json<{ apps: Install[] }>("apps", {}, owner.subject).catch(() => ({ apps: [] as Install[] }))
    const shown = now.apps.find((install) => install.for === owner.who && install.app === app)
    if (!(shown?.state === "not_installed" || shown?.update?.state === "failed")) toast(error as Error)
  } finally {
    setWorking()
    refresh(owners)
  }
}

const install = (owner: Owner, listed: Listed) => act(owner, listed.app, "install", () => call("install", { app: listed.app }, { subject: owner.subject }))
const retry = (owner: Owner, done: Install) => act(owner, done.app, "retry", () => call("apps_retry", { name: done.name }, { subject: owner.subject }))
const update = (owner: Owner, done: Install) => act(owner, done.app, "update", () => call("apps_update", { name: done.name }, { subject: owner.subject }))

/// Uninstalls, first warning where apps installed for them require it: they go too.
async function uninstall(market: Market, owner: Owner, done: Install) {
  const title = listed(market, done).title
  if (done.required_by.length) {
    const by = done.required_by.map((name) => owner.installs.find((install) => install.name === name)).filter((install) => !!install)
    const reasons = [...new Set(by.map((install) => install.requires[done.name]).filter(Boolean))]
    const reason = reasons.length === 1 ? ` ${reasons[0]}` : ""
    const titles = by.map((install) => listed(market, install).title)
    const text = titles.length === 1 ? `${titles[0]} uses ${title}${reason}. It can't work without it, so it'll be uninstalled too.` : `${and(titles)} use ${title}${reason}. They can't work without it, so they'll be uninstalled too.`
    if (!(await confirm({ heading: `Uninstall ${title}?`, text, action: "Uninstall", destructive: true }))) return
  }
  act(owner, done.app, "uninstall", () => call("uninstall", { name: done.name, confirm: done.required_by.length > 0 }, { subject: owner.subject }))
}

async function trust(market: Market, owner: Owner, done: Install) {
  const title = listed(market, done).title
  if (!done.trusted && !(await confirm({ heading: `Trust ${title}?`, text: "It runs on your servers, with everything they can reach, for everyone who has it.", action: "Make Trusted" }))) return
  act(owner, done.app, "trust", () => call("apps_trust", { name: done.name, trusted: !done.trusted }, { subject: owner.subject }))
}

/// An app's icon and its name under it, the size it is on a Home Screen.
function Tile(props: { listed: { title?: string; name?: string; icon: string | null }; onPress?: () => void; fills?: boolean }) {
  const name = () => props.listed.title ?? props.listed.name
  return (
    <Pressable role={props.onPress ? "button" : undefined} aria-label={props.onPress ? name() : undefined} onPress={props.onPress} style={[look.tile, props.fills && look.fills]}>
      <Show when={props.listed.icon} fallback={<View style={[look.icon, look.blank]} />}>
        <Image src={props.listed.icon!} alt="" style={look.icon} />
      </Show>
      <Text aria-hidden={props.onPress ? "true" : undefined} style={look.name}>{name()}</Text>
    </Pressable>
  )
}

/// A category's apps in one row, sideways, with See All once more are there than a phone shows.
function Shelf(props: { category: string; apps: Listed[] }) {
  return (
    <View style={look.shelf}>
      <View style={look.shelfHeading}>
        <Text role="heading" aria-level={2} style={look.heading}>{props.category}</Text>
        <Show when={props.apps.length > 4}>
          <Pressable role="button" onPress={() => to({ category: props.category })}>
            <Text style={look.more}>See All</Text>
          </Pressable>
        </Show>
      </View>
      <ScrollView style={look.shelfApps}>
        <For each={props.apps} keyed={key}>
          {(listed) => <Tile listed={listed()} onPress={() => to({ category: view().category, app: key(listed()) })} />}
        </For>
      </ScrollView>
    </View>
  )
}

/// Apps as a grid of tiles: a category's every one, or what a search found.
function Grid(props: { apps: Listed[] }) {
  return (
    <View style={look.grid}>
      <For each={props.apps} keyed={key}>
        {(listed) => <Tile fills listed={listed()} onPress={() => to({ category: view().category, app: key(listed()) })} />}
      </For>
    </View>
  )
}

function Marketplace(props: { market: Market }) {
  const [query, setQuery] = createSignal("")
  const found = () => {
    const words = query().trim().toLowerCase()
    return props.market.apps.filter((listed) => `${listed.title} ${listed.subtitle ?? ""} ${listed.category ?? ""}`.toLowerCase().includes(words))
  }
  const categories = () => props.market.categories.map((category) => ({ category, apps: props.market.apps.filter((listed) => listed.category === category) })).filter(({ apps }) => apps.length)
  const chosen = () => view().category
  const choose = (category?: string) => setView({ category })
  return (
    <>
      <Search placeholder="Search" aria-label="Search apps" value={query()} onChangeText={setQuery} />
      <Show when={!query()}>
        <ScrollView role="toolbar" aria-label="Categories" style={look.chips}>
          <Chip chosen={!chosen()} onPress={() => choose()}>All</Chip>
          <For each={categories()} keyed={({ category }) => category}>
            {(shelf) => <Chip chosen={chosen() === shelf().category} onPress={() => choose(shelf().category)}>{shelf().category}</Chip>}
          </For>
        </ScrollView>
      </Show>
      <Show
        when={query()}
        fallback={
          <Show when={chosen()} fallback={<View style={look.shelves}><For each={categories()} keyed={({ category }) => category}>{(shelf) => <Shelf category={shelf().category} apps={shelf().apps} />}</For></View>}>
            <Grid apps={props.market.apps.filter((listed) => listed.category === chosen())} />
          </Show>
        }
      >
        <Show when={found().length} fallback={<Empty>No apps match.</Empty>}>
          <Grid apps={found()} />
        </Show>
      </Show>
    </>
  )
}

/// A category to choose, a pill: the one chosen is dark.
function Chip(props: { chosen: boolean; onPress: () => void; children: Element }) {
  return (
    <Pressable role="button" aria-pressed={props.chosen ? "true" : "false"} onPress={props.onPress} style={[look.chip, props.chosen && look.chosen]}>
      <Text style={look.heading}>{props.children}</Text>
    </Pressable>
  )
}

/// Get, on an app's page: it installs the app for whoever it can still be installed for, and where that's more than one
/// owner, asks who, in a menu under it. Once there's nobody left you may install it for, it's Open where the app has
/// a screen and is installed, as an iPhone's App Store has it: for you first, then the owners after you.
function Get(props: { all: Owners; listed: Listed }) {
  const left = () => props.all.owners.filter((owner) => !only(props.all, owner) && !owner.installs.some((install) => install.app === props.listed.app))
  const busy = () => left().some((owner) => working() === `${owner.who}|${props.listed.app}|install`)
  const opens = () =>
    props.listed.entrypoints.length
      ? props.all.owners.flatMap((owner) => owner.installs.filter((done) => done.app === props.listed.app && done.state === "installed").map((done) => ({ owner, done })))[0]
      : undefined
  return (
    <Show when={left().length || busy()} fallback={<Show when={opens()}>{(it) => <Button onPress={() => open(it().done.name, { subject: it().owner.subject })}>Open</Button>}</Show>}>
      <Show
        when={left().length > 1 && !busy()}
        fallback={
          <Button loading={busy()} onPress={() => install(left()[0], props.listed)}>
            Get
          </Button>
        }
      >
        <Menu.Root onSelect={(value) => install(left().find((owner) => owner.who === value)!, props.listed)}>
          <Menu.Trigger variant="default">Get</Menu.Trigger>
          <Menu.Positioner>
            <Menu.Content>
              <Menu.Group label="Install for">
                <For each={left()} keyed={(owner) => owner.who}>
                  {(owner) => <Menu.Item value={owner().who}>{owner().name}</Menu.Item>}
                </For>
              </Menu.Group>
            </Menu.Content>
          </Menu.Positioner>
        </Menu.Root>
      </Show>
    </Show>
  )
}

/// Words that are only so, in grey.
const Quiet = (props: { children: Element }) => <Text style={look.quiet}>{props.children}</Text>

/// What's under way, with its spinner.
const Busy = (props: { children: Element }) => (
  <View style={look.busy}>
    <Spinner />
    <Quiet>{props.children}</Quiet>
  </View>
)

/// An app's place for one owner: where its install is for them, and what to do about it.
function Where(props: { market: Market; all: Owners; owner: Owner; listed: Listed }) {
  const done = () => props.owner.installs.find((install) => install.app === props.listed.app)
  const busy = (what: string) => working() === `${props.owner.who}|${props.listed.app}|${what}`
  const state = () => done()?.state ?? null
  // Each state's controls stay as they are while what's listed is asked for again, so an open menu stays open.
  const now = () => done()!
  const may = () => !only(props.all, props.owner)
  // Drawn where the row asks for it, so a menu in it is the row's (a right-click, a swipe, the action panel).
  const end = () => (
    <Switch>
      <Match when={state() === "installing"}>
        <Busy>Installing</Busy>
      </Match>
      {/* Restoring is Recently Deleted's, as for everything deleted: one place, as iOS has it. */}
      <Match when={state() === "recently_deleted" && may()}>
        <Button variant="secondary" onPress={() => open("settings#soft-delete/deleted", { subject: props.owner.subject })}>Recently Deleted</Button>
      </Match>
      <Match when={state() === "installed"}>
        <View style={look.busy}>
          <Show when={now().trusted === false}>
            <Quiet>Untrusted</Quiet>
          </Show>
          <Switch fallback={<Quiet>Ready for your AI</Quiet>}>
            <Match when={now().update?.state === "updating"}>
              <Busy>Updating</Busy>
            </Match>
            <Match when={now().update?.state === "available" && may()}>
              <Button variant="secondary" loading={busy("update")} onPress={() => update(props.owner, now())}>Update</Button>
            </Match>
            <Match when={props.listed.entrypoints.length}>
              <Button variant="secondary" onPress={() => open(now().name, { subject: props.owner.subject })}>Open</Button>
            </Match>
          </Switch>
          <Show when={may()}>
          <Menu.Root onSelect={(value) => (value === "uninstall" ? uninstall(props.market, props.owner, now()) : trust(props.market, props.owner, now()))}>
            <Menu.Trigger />
            <Menu.Positioner>
              <Menu.Content>
                <Menu.Item value="trust">{now().trusted === false ? "Make Trusted" : "Make Untrusted"}</Menu.Item>
                <Menu.Separator />
                <Menu.Item value="uninstall" destructive>Uninstall</Menu.Item>
              </Menu.Content>
            </Menu.Positioner>
          </Menu.Root>
          </Show>
        </View>
      </Match>
    </Switch>
  )
  // Version numbers are the App Store's alone (DESIGN.md).
  const text = () => (state() === "not_installed" ? "Couldn't install" : state() === "recently_deleted" ? "Uninstalled" : state() === "installed" && now().version ? `Version ${now().version}` : !state() ? (only(props.all, props.owner) ?? "Not installed") : undefined)
  return <Row heading={props.owner.name} text={text()} end={end()} />
}

/// What's failed of an app for an owner, on the app's page: installing it, or its update. Why is for Telemetry and the agents (`apps`), not a person.
function Failed(props: { all: Owners; owner: Owner; listed: Listed }) {
  const done = () => props.owner.installs.find((install) => install.app === props.listed.app)
  const busy = () => working() === `${props.owner.who}|${props.listed.app}|retry`
  const heading = () => (done()?.state === "not_installed" ? `Couldn't install for ${props.owner.name}` : `Couldn't update for ${props.owner.name}`)
  return (
    <Show when={done()?.state === "not_installed" || done()?.update?.state === "failed"}>
      <Problem
        heading={heading()}
        text={done()!.state === "not_installed" ? "This can't be installed yet" : "This update can't be installed yet"}
        action={<Show when={!only(props.all, props.owner)}><Button variant="outline" loading={busy()} onPress={() => retry(props.owner, done()!)}>Try Again</Button></Show>}
      />
    </Show>
  )
}

/// An app's page: what it is, a row for each owner, then what it gives, what it requires and why, and See All.
function Page(props: { market: Market; all: Owners; listed: Listed }) {
  // The apps that need it, wherever it's installed, by their names.
  const requiredBy = () => [
    ...new Set(
      props.all.owners.flatMap((owner) => {
        const done = owner.installs.find((install) => install.app === props.listed.app)
        return (done?.required_by ?? []).map((name) => listed(props.market, owner.installs.find((install) => install.name === name) ?? { ...done!, name, app: null }).title)
      }),
    ),
  ]
  return (
    <View style={look.page}>
      <View style={look.top}>
        <Show when={props.listed.icon} fallback={<View style={[look.largeIcon, look.blank]} />}>
          <Image src={props.listed.icon!} alt="" style={look.largeIcon} />
        </Show>
        <View style={look.about}>
          <View style={look.titles}>
            <Text role="heading" aria-level={2} style={look.title}>{props.listed.title}</Text>
            <Show when={props.listed.subtitle}>
              <Quiet>{props.listed.subtitle!}</Quiet>
            </Show>
          </View>
          <Show when={props.listed.app}>
            <View style={look.row}>
              <Get all={props.all} listed={props.listed} />
            </View>
          </Show>
        </View>
      </View>
      <Show when={props.listed.app} fallback={<Quiet>Ask your AI to check it and install it.</Quiet>}>
        <List each={props.all.owners} keyed={(owner) => owner.who}>
          {(owner) => <Where market={props.market} all={props.all} owner={owner()} listed={props.listed} />}
        </List>
        <For each={props.all.owners} keyed={(owner) => owner.who}>
          {(owner) => <Failed all={props.all} owner={owner()} listed={props.listed} />}
        </For>
      </Show>
      <Show when={props.listed.about}>
        <Text>{props.listed.about}</Text>
      </Show>
      <Show when={props.listed.entrypoints.length}>
        <Section heading="On the Home Screen">
          <View style={look.requires}>
            <For each={props.listed.entrypoints}>{(entrypoint) => <Tile fills listed={entrypoint} />}</For>
          </View>
        </Section>
      </Show>
      <Show when={props.listed.widgets.length}>
        <Section heading="Widgets">
          <Text>{and(props.listed.widgets.map((widget) => widget.name))}</Text>
        </Section>
      </Show>
      <Show when={props.listed.requires.length}>
        <Section heading="Requires">
          <For each={props.listed.requires}>
            {(requirement) => (
              <Text>
                {requirement.title}
                <Show when={requirement.why}>
                  <Text style={look.quiet}> {requirement.why}</Text>
                </Show>
              </Text>
            )}
          </For>
        </Section>
      </Show>
      <Show when={requiredBy().length}>
        <Section heading="Required by">
          <Text>{and(requiredBy())}</Text>
        </Section>
      </Show>
      <Show when={props.listed.source}>
        <Section heading="Source">
          <Text>{props.listed.source!}</Text>
        </Section>
      </Show>
      <Show when={props.listed.tools.length}>
        <List>
          <Row heading="See All" onPress={() => to({ ...view(), app: key(props.listed), details: true })} end={<Icon name="caret-right" along />} />
        </List>
      </Show>
    </View>
  )
}

/// See All: everything else an app is made of.
function Details(props: { listed: Listed }) {
  return (
    <Screen heading={props.listed.title} back onBack={out}>
      <Section heading="Tools">
        <Text>{String(props.listed.tools.length)}</Text>
        <View style={look.tools}>
          <For each={props.listed.tools}>{(tool) => <Text style={look.quiet}>{tool}</Text>}</For>
        </View>
      </Section>
      <Show when={props.listed.trusted !== null}>
        <Section heading="Runs">
          <Text>{props.listed.trusted ? "Trusted, on your servers" : "Untrusted, in a sandbox"}</Text>
        </Section>
      </Show>
    </Screen>
  )
}

function AppStore(props: { market: Market; all: Owners }) {
  // An app the Marketplace doesn't list, as an installed one may not be, still has its page, as a link opens it.
  const opened = () => props.market.apps.find((listed) => key(listed) === view().app) ?? (view().app ? installed(props.market, props.all, view().app!) : undefined)
  const covered = () => !!(view().app && opened())
  // The Marketplace stays mounted, hidden, under an app's page, so going back shows it as it was, scroll and all, as
  // a navigation stack keeps the screen before.
  return (
    <>
      <View style={[look.grows, covered() && look.hidden]}>
        <Screen heading="App Store">
          <Marketplace market={props.market} />
        </Screen>
      </View>
      <Show when={covered()}>
        <Show when={view().details} fallback={<Screen back={!view().linked} onBack={out} heading={opened()!.title} largeTitle><Page market={props.market} all={props.all} listed={opened()!} /></Screen>}>
          <Details listed={opened()!} />
        </Show>
      </Show>
    </>
  )
}

/// An app that's installed as the Marketplace doesn't list it, by its artifact.
const installed = (market: Market, all: Owners, app: string) => {
  const install = all.owners.flatMap((owner) => owner.installs).find((one) => one.app === app)
  return install && listed(market, install)
}

/// The App Store.
export default function Store() {
  // A link to an app's page, `one:app-store#app=<its artifact>`, as an App Library's app with no screen of its own opens
  // it: its page, where the App Store starts.
  createEffect(link, (fragment) => {
    if (fragment?.startsWith("app=")) setView({ app: decodeURIComponent(fragment.slice(4)), linked: true })
  })
  return (
    <Errored fallback={(error) => <Screen heading="App Store"><Problem heading="Can't show apps" text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading="App Store"><Skeleton rows={4} /></Screen>}>
        <AppStore market={market()} all={owners()} />
      </Loading>
    </Errored>
  )
}
