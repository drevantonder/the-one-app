// App Store: every app the network can install, a row of each category. An app's page has Get, which installs it, and
// a row for each owner, with where its install is for them and what to do about it: managing what's installed is
// otherwise the App Library's. It's this app's page, and the
// launcher's page after the App Library, which imports it (package.json's `exports`): its look comes with it, under
// `.app-store`, so the launcher's and its own don't meet.
import { createEffect, createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch } from "solid-js"
import styles from "./app.css?inline"
import { Button, call, changed, confirm, covers, Empty, go, Icon, List, Menu, open, Problem, Row, Screen, Search, Skeleton, Spinner, toast } from "ui"

if (!document.getElementById("app-store")) document.head.append(Object.assign(document.createElement("style"), { id: "app-store", textContent: styles }))

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
  why: string | null
  deleted: number | null
  trusted: boolean | null
  version: number | null
  update: { state: "available" | "updating" | "failed"; why: string | null } | null
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

/// Where the app is: every app, a category's every app, an app's page and its See All. Each step in is a step in the
/// history, so the device's back goes back out.
interface View {
  category?: string
  app?: string
  details?: boolean
}
const [view, setView] = createSignal<View>(history.state?.apps ?? {})
const to = (next: View) => go(() => (history.pushState({ ...history.state, apps: next }, ""), setView(next)))
addEventListener("popstate", () => {
  const now: View = history.state?.apps ?? {}
  if (JSON.stringify(now) !== JSON.stringify(view())) go(() => setView(now), { back: true })
})

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
const restore = (owner: Owner, done: Install) => act(owner, done.app, "restore", () => call("apps_restore", { name: done.name }, { subject: owner.subject }))

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
function Tile(props: { listed: { title?: string; name?: string; icon: string | null }; onClick?: () => void }) {
  const inside = () => (
    <>
      <Show when={props.listed.icon} fallback={<span class="blank" />}>
        <img src={props.listed.icon!} alt="" />
      </Show>
      <span>{props.listed.title ?? props.listed.name}</span>
    </>
  )
  return (
    <Show when={props.onClick} fallback={<div class="app-tile">{inside()}</div>}>
      <button type="button" class="app-tile" onClick={() => props.onClick?.()}>
        {inside()}
      </button>
    </Show>
  )
}

/// A category's apps in one row, sideways, cut so the next shows, with See All once more are there than fit.
function Shelf(props: { category: string; apps: Listed[] }) {
  const [more, setMore] = createSignal(false)
  const watch = (row: HTMLElement) => new ResizeObserver(() => setMore(row.scrollWidth > row.clientWidth + 1)).observe(row)
  return (
    <section class="shelf">
      <header>
        <h2>{props.category}</h2>
        <Show when={more()}>
          <Button variant="ghost" onClick={() => to({ category: props.category })}>
            See All
          </Button>
        </Show>
      </header>
      <div class="shelf-row" ref={watch}>
        <For each={props.apps} keyed={key}>
          {(listed) => <Tile listed={listed()} onClick={() => to({ category: view().category, app: key(listed()) })} />}
        </For>
      </div>
    </section>
  )
}

/// Apps as a grid of tiles: a category's every one, or what a search found.
function Grid(props: { apps: Listed[] }) {
  return (
    <div class="app-grid">
      <For each={props.apps} keyed={key}>
        {(listed) => <Tile listed={listed()} onClick={() => to({ category: view().category, app: key(listed()) })} />}
      </For>
    </div>
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
  const choose = (category?: string) => (history.replaceState({ ...history.state, apps: { category } }, ""), setView({ category }))
  return (
    <>
      <Search placeholder="Search" aria-label="Search apps" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} />
      <Show when={!query()}>
        <nav class="chips" aria-label="Categories">
          <Button variant={chosen() ? "secondary" : "default"} aria-pressed={String(!chosen()) as "true" | "false"} onClick={() => choose()}>
            All
          </Button>
          <For each={categories()} keyed={({ category }) => category}>
            {(shelf) => (
              <Button variant={chosen() === shelf().category ? "default" : "secondary"} aria-pressed={String(chosen() === shelf().category) as "true" | "false"} onClick={() => choose(shelf().category)}>
                {shelf().category}
              </Button>
            )}
          </For>
        </nav>
      </Show>
      <Show
        when={query()}
        fallback={
          <Show when={chosen()} fallback={<div class="shelves"><For each={categories()} keyed={({ category }) => category}>{(shelf) => <Shelf category={shelf().category} apps={shelf().apps} />}</For></div>}>
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

/// Get, on an app's page: it installs the app for whoever it can still be installed for, and where that's more than one
/// owner, asks who, in a menu under it. It's gone once there's nobody left you may install it for.
function Get(props: { all: Owners; listed: Listed }) {
  const left = () => props.all.owners.filter((owner) => !only(props.all, owner) && !owner.installs.some((install) => install.app === props.listed.app))
  const busy = () => left().some((owner) => working() === `${owner.who}|${props.listed.app}|install`)
  const [asking, setAsking] = createSignal(false)
  let anchor!: HTMLSpanElement
  const get = () => (left().length === 1 ? install(left()[0], props.listed) : setAsking(true))
  return (
    <Show when={left().length || busy()}>
      <span class="get" ref={anchor}>
        <Button loading={busy()} onClick={get}>
          Get
        </Button>
      </span>
      <Menu.Root
        open={asking()}
        onOpenChange={({ open }) => !open && setAsking(false)}
        onSelect={({ value }) => (setAsking(false), install(left().find((owner) => owner.who === value)!, props.listed))}
        positioning={{ getAnchorRect: () => anchor.getBoundingClientRect() }}
      >
        <Menu.Positioner>
          <Menu.Content>
            <Menu.ItemGroup id="get">
              <Menu.ItemGroupLabel htmlFor="get">Install for</Menu.ItemGroupLabel>
              <For each={left()} keyed={(owner) => owner.who}>
                {(owner) => <Menu.Item value={owner().who}>{owner().name}</Menu.Item>}
              </For>
            </Menu.ItemGroup>
          </Menu.Content>
        </Menu.Positioner>
      </Menu.Root>
    </Show>
  )
}

/// An app's place for one owner: where its install is for them, and what to do about it.
function Where(props: { market: Market; all: Owners; owner: Owner; listed: Listed }) {
  const done = () => props.owner.installs.find((install) => install.app === props.listed.app)
  const busy = (what: string) => working() === `${props.owner.who}|${props.listed.app}|${what}`
  const state = () => done()?.state ?? null
  // Each state's controls stay as they are while what's listed is asked for again, so an open menu stays open.
  const now = () => done()!
  const may = () => !only(props.all, props.owner)
  const end = (
    <Switch>
      <Match when={state() === "installing"}>
        <span class="busy"><Spinner /> Installing</span>
      </Match>
      <Match when={state() === "recently_deleted" && may()}>
        <Button variant="secondary" loading={busy("restore")} onClick={() => restore(props.owner, now())}>Restore</Button>
      </Match>
      <Match when={state() === "installed"}>
        <span class="owner-end">
          <Show when={now().trusted === false}>
            <span class="quiet">Untrusted</span>
          </Show>
          <Switch fallback={<span class="quiet">Ready for your AI</span>}>
            <Match when={now().update?.state === "updating"}>
              <span class="busy"><Spinner /> Updating</span>
            </Match>
            <Match when={now().update?.state === "available" && may()}>
              <Button variant="secondary" loading={busy("update")} onClick={() => update(props.owner, now())}>Update</Button>
            </Match>
            <Match when={props.listed.entrypoints.length}>
              <Button variant="secondary" onClick={() => open(now().name, { subject: props.owner.subject })}>Open</Button>
            </Match>
          </Switch>
          <Show when={may()}>
          <Menu.Root onSelect={({ value }) => (value === "uninstall" ? uninstall(props.market, props.owner, now()) : trust(props.market, props.owner, now()))}>
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
        </span>
      </Match>
    </Switch>
  )
  // Version numbers are the App Store's alone (DESIGN.md).
  const text = () => (state() === "not_installed" ? "Couldn't install" : state() === "recently_deleted" ? "Recently Deleted" : state() === "installed" && now().version ? `Version ${now().version}` : !state() ? (only(props.all, props.owner) ?? "Not installed") : undefined)
  return <Row heading={props.owner.name} text={text()} end={end} />
}

/// What's failed of an app for an owner, on the app's page: installing it, or its update.
function Failed(props: { all: Owners; owner: Owner; listed: Listed }) {
  const done = () => props.owner.installs.find((install) => install.app === props.listed.app)
  const busy = () => working() === `${props.owner.who}|${props.listed.app}|retry`
  const heading = () => (done()?.state === "not_installed" ? `Couldn't install for ${props.owner.name}` : `Couldn't update for ${props.owner.name}`)
  return (
    <Show when={done()?.state === "not_installed" || done()?.update?.state === "failed"}>
      <Problem
        heading={heading()}
        text={(done()!.state === "not_installed" ? done()!.why : done()!.update?.why) ?? undefined}
        action={<Show when={!only(props.all, props.owner)}><Button variant="outline" loading={busy()} onClick={() => retry(props.owner, done()!)}>Try Again</Button></Show>}
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
    <div class="app-page">
      <div class="head">
        <Show when={props.listed.icon} fallback={<span class="icon blank" />}>
          <img class="icon" src={props.listed.icon!} alt="" />
        </Show>
        <div>
          <h2>{props.listed.title}</h2>
          <Show when={props.listed.subtitle}>
            <p>{props.listed.subtitle}</p>
          </Show>
        </div>
        <Show when={props.listed.app}>
          <Get all={props.all} listed={props.listed} />
        </Show>
      </div>
      <Show when={props.listed.app} fallback={<p class="quiet">Ask your AI to check it and install it.</p>}>
        <List class="installs">
          <For each={props.all.owners} keyed={(owner) => owner.who}>
            {(owner) => <Where market={props.market} all={props.all} owner={owner()} listed={props.listed} />}
          </For>
        </List>
        <For each={props.all.owners} keyed={(owner) => owner.who}>
          {(owner) => <Failed all={props.all} owner={owner()} listed={props.listed} />}
        </For>
      </Show>
      <Show when={props.listed.about}>
        <p>{props.listed.about}</p>
      </Show>
      <dl class="facts">
        <Show when={props.listed.entrypoints.length}>
          <dt>On the Home Screen</dt>
          <dd class="entrypoints">
            <For each={props.listed.entrypoints}>{(entrypoint) => <Tile listed={entrypoint} />}</For>
          </dd>
        </Show>
        <Show when={props.listed.widgets.length}>
          <dt>Widgets</dt>
          <dd>{and(props.listed.widgets.map((widget) => widget.name))}</dd>
        </Show>
        <Show when={props.listed.requires.length}>
          <dt>Requires</dt>
          <dd class="requires">
            <For each={props.listed.requires}>
              {(requirement) => (
                <span>
                  {requirement.title}
                  <Show when={requirement.why}>
                    <span class="quiet"> {requirement.why}</span>
                  </Show>
                </span>
              )}
            </For>
          </dd>
        </Show>
        <Show when={requiredBy().length}>
          <dt>Required by</dt>
          <dd>{and(requiredBy())}</dd>
        </Show>
        <Show when={props.listed.source}>
          <dt>Source</dt>
          <dd>{props.listed.source}</dd>
        </Show>
      </dl>
      <Show when={props.listed.tools.length}>
        <List>
          <Row heading="See All" onClick={() => to({ ...view(), app: key(props.listed), details: true })} end={<Icon name="caret-right" along />} />
        </List>
      </Show>
    </div>
  )
}

/// See All: everything else an app is made of.
function Details(props: { listed: Listed }) {
  return (
    <Screen heading={props.listed.title} back>
      <dl class="facts">
        <dt>Tools</dt>
        <dd>
          {props.listed.tools.length}
          <span class="code">
            <For each={props.listed.tools}>{(tool) => <span>{tool}</span>}</For>
          </span>
        </dd>
        <Show when={props.listed.trusted !== null}>
          <dt>Runs</dt>
          <dd>{props.listed.trusted ? "Trusted, on your servers" : "Untrusted, in a sandbox"}</dd>
        </Show>
      </dl>
    </Screen>
  )
}

function AppStore(props: { market: Market; all: Owners; onBack?: () => void }) {
  const opened = () => props.market.apps.find((listed) => key(listed) === view().app)
  const covered = () => !!(view().app && opened())
  // The Marketplace stays mounted, hidden, under an app's page, so going back shows it as it was, scroll and all, as
  // a navigation stack keeps the screen before.
  return (
    <>
      <div class="under" hidden={covered()}>
        <Screen heading="App Store" back={!!(view().category || props.onBack)} onBack={() => (view().category ? history.back() : props.onBack?.())} class="apps">
          <Marketplace market={props.market} />
        </Screen>
      </div>
      <Show when={covered()}>
        <Show when={view().details} fallback={<Screen back><Page market={props.market} all={props.all} listed={opened()!} /></Screen>}>
          <Details listed={opened()!} />
        </Show>
      </Show>
    </>
  )
}

/// An app's page, by its artifact, as the launcher shows it for an app with no entrypoint of its own, with Back.
export function Listing(props: { app: string; onBack: () => void }) {
  return (
    <div class="app-store">
      <Errored fallback={(error) => <Screen back onBack={props.onBack}><Problem heading="Can't show it" text={error() as Error} /></Screen>}>
        <Loading fallback={<Screen back onBack={props.onBack}><Skeleton rows={4} /></Screen>}>
          <Show when={market().apps.find((one) => one.app === props.app) ?? installed(market(), owners(), props.app)} fallback={<Screen back onBack={props.onBack}><Empty>It's not in the App Store.</Empty></Screen>}>
            {(shown) => (
              <Screen back onBack={props.onBack}>
                <Page market={market()} all={owners()} listed={shown()} />
              </Screen>
            )}
          </Show>
        </Loading>
      </Errored>
    </div>
  )
}

/// An app that's installed as the Marketplace doesn't list it, by its artifact.
const installed = (market: Market, all: Owners, app: string) => {
  const install = all.owners.flatMap((owner) => owner.installs).find((one) => one.app === app)
  return install && listed(market, install)
}

/// The App Store, as its page and the launcher's show it. `onBack` is its Back, out of it, where it has one.
export default function Store(props: { onBack?: () => void }) {
  // A step in, to a category or an app, is a step the device's own back goes back out of.
  createEffect(
    createMemo(() => !!(view().category || view().app)),
    (deep) => (deep ? covers(() => history.back()) : undefined),
  )
  return (
    <div class="app-store">
      <Errored fallback={(error) => <Screen heading="App Store" back={!!props.onBack} onBack={props.onBack}><Problem heading="Can't show apps" text={error() as Error} /></Screen>}>
        <Loading fallback={<Screen heading="App Store" back={!!props.onBack} onBack={props.onBack}><Skeleton rows={4} /></Screen>}>
          <AppStore market={market()} all={owners()} onBack={props.onBack} />
        </Loading>
      </Errored>
    </div>
  )
}
