// Plugins: Marketplace, every plugin the network can install, a row of each category, and Installed, what's
// installed for you, everyone and your groups, with its updates, what failed and Recently Deleted. A plugin's page
// has a row for each owner, with where its install is for them and what to do about it.
import { createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch } from "solid-js"
import { render } from "@solidjs/web"
import "./app.css"
import { Button, call, changed, confirm, Empty, go, Icon, leave, List, Menu, open, Problem, Row, Screen, Search, Section, Skeleton, Spinner, Split, Tabs, toast } from "ui"

/** A plugin the network can install, as `plugins_marketplace` lists it. */
interface Listed {
  name: string
  plugin: string | null
  source: string | null
  title: string
  icon: string | null
  subtitle: string | null
  about: string | null
  category: string | null
  apps: { name: string; icon: string | null }[]
  widgets: { name: string; sizes: string[] }[]
  tools: string[]
  trusted: boolean | null
  requires: { name: string; title: string; why: string | null }[]
}

/** An install, as `plugins` lists it. */
interface Install {
  name: string
  for: string
  plugin: string | null
  state: "installing" | "installed" | "not_installed" | "recently_deleted" | null
  stage: string | null
  why: string | null
  deleted: number | null
  trusted: boolean | null
  update: { state: "available" | "updating" | "failed"; why: string | null } | null
  requires: Record<string, string>
  required_by: string[]
}

/** Whoever plugins are installed for that this person sees: themselves, everyone and their groups, by the name a
 * person reads, with what's installed for them and who may install for them. */
interface Owner {
  subject?: string
  name: string
  who: string
  installs: Install[]
  installers: string[]
}

const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

const market = createMemo(() => json<{ categories: string[]; plugins: Listed[] }>("plugins_marketplace"))

const owners = createMemo(async () => {
  const [groups, people] = await Promise.all([
    json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => []),
    json<{ people: { who: string; called?: string }[] }>("sign-in_everyone").then(({ people }) => people, (): { who: string; called?: string }[] => []),
  ])
  const called = (who: string) => people.find((person) => person.who === who)?.called ?? who.replace(/^users\//, "").replace(/^./, (first) => first.toUpperCase())
  const whose: { subject?: string; name: string }[] = [{ name: "You" }, { subject: "network", name: "Everyone" }, ...groups.map(({ group, called }) => ({ subject: group, name: called }))]
  const all = await Promise.all(
    whose.map(async (owner): Promise<Owner> => {
      const listed = await json<{ for: string; plugins: Install[] }>("plugins", {}, owner.subject)
      // Anyone installs for themselves; for a group or everyone, whoever may.
      const installers = owner.subject ? await json<{ users: string[] }>("permissions_users", { artifact: owner.subject, relation: "installer" }).then(({ users }) => users, (): string[] => []) : [listed.for]
      return { ...owner, who: listed.for, installs: listed.plugins.filter((install) => install.for === listed.for), installers }
    }),
  )
  return { owners: all, me: all[0].who, called }
})
changed(() => (refresh(market), refresh(owners)))

type Owners = Awaited<ReturnType<typeof owners>>
type Market = Awaited<ReturnType<typeof market>>

/// Where the app is: Marketplace or Installed, a category's every plugin, a plugin's page and its See All. Each step
/// in is a step in the history, so the device's back goes back out.
interface View {
  tab: "marketplace" | "installed"
  category?: string
  plugin?: string
  details?: boolean
}
const [view, setView] = createSignal<View>(history.state?.plugins ?? { tab: "marketplace" })
const to = (next: View) => go(() => (history.pushState({ ...history.state, plugins: next }, ""), setView(next)))
addEventListener("popstate", () => {
  const now: View = history.state?.plugins ?? { tab: "marketplace" }
  if (JSON.stringify(now) !== JSON.stringify(view())) go(() => setView(now), { back: true })
})
const tab = (tab: View["tab"]) => (history.replaceState({ ...history.state, plugins: { tab } }, ""), setView({ tab }))

/// The plugin a page is of: its artifact, or for one only the directory has, its source and name.
const key = (listed: Listed) => listed.plugin ?? `${listed.source}#${listed.name}`

/// What an install's plugin is, where the Marketplace doesn't list it: its name alone.
const listed = (market: Market, install: Install): Listed =>
  market.plugins.find((listed) => listed.plugin === install.plugin) ?? { name: install.name, plugin: install.plugin, source: null, title: install.name, icon: null, subtitle: null, about: null, category: null, apps: [], widgets: [], tools: [], trusted: install.trusted, requires: [] }

const and = (words: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(words)
const days = new Intl.RelativeTimeFormat("en", { numeric: "auto" })
const ago = (at: number) => days.format(-Math.round((Date.now() - at) / 86_400_000), "day")

/// Who may install for an owner, if not this person: "Only Andre can install for Everyone".
function only(all: Owners, owner: Owner) {
  if (owner.installers.includes(all.me) || owner.installers.includes("everyone")) return undefined
  const who = new Intl.ListFormat("en", { type: "disjunction" }).format(owner.installers.map(all.called))
  return owner.installers.length ? `Only ${who} can install for ${owner.name}` : `Nobody can install for ${owner.name}`
}

/// What's being done, by owner and plugin, while its call runs.
const [working, setWorking] = createSignal<string>()

/// Runs a call about a plugin for an owner, says what else it did (`also`: "Gmail uses Google Account to reach your
/// mail, so it's installed too."), and what went wrong, unless the install shows it itself.
async function act(owner: Owner, plugin: string | null, what: string, run: () => Promise<string>) {
  setWorking(`${owner.who}|${plugin}|${what}`)
  try {
    const done = JSON.parse(await run())
    if (done.also?.length) toast(done.also.join(" "))
  } catch (error) {
    const now = await json<{ plugins: Install[] }>("plugins", {}, owner.subject).catch(() => ({ plugins: [] as Install[] }))
    const shown = now.plugins.find((install) => install.for === owner.who && install.plugin === plugin)
    if (!(shown?.state === "not_installed" || shown?.update?.state === "failed")) toast(error as Error)
  } finally {
    setWorking()
    refresh(owners)
  }
}

const install = (owner: Owner, listed: Listed) => act(owner, listed.plugin, "install", () => call("install", { plugin: listed.plugin }, { subject: owner.subject }))
const retry = (owner: Owner, done: Install) => act(owner, done.plugin, "retry", () => call("plugins_retry", { name: done.name }, { subject: owner.subject }))
const update = (owner: Owner, done: Install) => act(owner, done.plugin, "update", () => call("plugins_update", { name: done.name }, { subject: owner.subject }))
const restore = (owner: Owner, done: Install) => act(owner, done.plugin, "restore", () => call("plugins_restore", { name: done.name }, { subject: owner.subject }))

/// Uninstalls, first warning where plugins installed for them require it: they go too.
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
  act(owner, done.plugin, "uninstall", () => call("uninstall", { name: done.name, confirm: done.required_by.length > 0 }, { subject: owner.subject }))
}

async function trust(market: Market, owner: Owner, done: Install) {
  const title = listed(market, done).title
  if (!done.trusted && !(await confirm({ heading: `Trust ${title}?`, text: "It runs on your servers, with everything they can reach, for everyone who has it.", action: "Make Trusted" }))) return
  act(owner, done.plugin, "trust", () => call("plugins_trust", { name: done.name, trusted: !done.trusted }, { subject: owner.subject }))
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
    <Show when={props.onClick} fallback={<div class="tile">{inside()}</div>}>
      <button type="button" class="tile" onClick={() => props.onClick?.()}>
        {inside()}
      </button>
    </Show>
  )
}

/// A category's plugins in one row, sideways, cut so the next shows, with See All once more are there than fit.
function Shelf(props: { category: string; plugins: Listed[] }) {
  const [more, setMore] = createSignal(false)
  const watch = (row: HTMLElement) => new ResizeObserver(() => setMore(row.scrollWidth > row.clientWidth + 1)).observe(row)
  return (
    <section class="shelf">
      <header>
        <h2>{props.category}</h2>
        <Show when={more()}>
          <Button variant="ghost" onClick={() => to({ tab: "marketplace", category: props.category })}>
            See All
          </Button>
        </Show>
      </header>
      <div class="shelf-row" ref={watch}>
        <For each={props.plugins} keyed={key}>
          {(listed) => <Tile listed={listed()} onClick={() => to({ tab: "marketplace", plugin: key(listed()) })} />}
        </For>
      </div>
    </section>
  )
}

/// Plugins as a grid of tiles: a category's every one, or what a search found.
function Grid(props: { plugins: Listed[] }) {
  return (
    <div class="grid">
      <For each={props.plugins} keyed={key}>
        {(listed) => <Tile listed={listed()} onClick={() => to({ tab: "marketplace", plugin: key(listed()) })} />}
      </For>
    </div>
  )
}

function Marketplace(props: { market: Market }) {
  const [query, setQuery] = createSignal("")
  const found = () => {
    const words = query().trim().toLowerCase()
    return props.market.plugins.filter((listed) => `${listed.title} ${listed.subtitle ?? ""} ${listed.category ?? ""}`.toLowerCase().includes(words))
  }
  const categories = () => props.market.categories.map((category) => ({ category, plugins: props.market.plugins.filter((listed) => listed.category === category) })).filter(({ plugins }) => plugins.length)
  const chosen = () => view().category
  const choose = (category?: string) => (history.replaceState({ ...history.state, plugins: { tab: "marketplace", category } }, ""), setView({ tab: "marketplace", category }))
  return (
    <>
      <Search placeholder="Search" aria-label="Search plugins" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} />
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
          <Show when={chosen()} fallback={<div class="shelves"><For each={categories()} keyed={({ category }) => category}>{(shelf) => <Shelf category={shelf().category} plugins={shelf().plugins} />}</For></div>}>
            <Grid plugins={props.market.plugins.filter((listed) => listed.category === chosen())} />
          </Show>
        }
      >
        <Show when={found().length} fallback={<Empty>No plugins match.</Empty>}>
          <Grid plugins={found()} />
        </Show>
      </Show>
    </>
  )
}

/// A plugin's place for one owner: where its install is for them, and what to do about it.
function Where(props: { market: Market; all: Owners; owner: Owner; listed: Listed; first: boolean }) {
  const done = () => props.owner.installs.find((install) => install.plugin === props.listed.plugin)
  const busy = (what: string) => working() === `${props.owner.who}|${props.listed.plugin}|${what}`
  const state = () => done()?.state ?? null
  // Each state's controls stay as they are while what's listed is asked for again, so an open menu stays open.
  const now = () => done()!
  const may = () => !only(props.all, props.owner)
  const end = (
    <Switch
      fallback={
        <Show when={!only(props.all, props.owner) && state() !== "not_installed"}>
          <Button variant={props.first ? "default" : "secondary"} loading={busy("install")} onClick={() => install(props.owner, props.listed)}>
            Install
          </Button>
        </Show>
      }
    >
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
            <Match when={props.listed.apps.length}>
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
  const text = () => (state() === "not_installed" ? "Couldn't install" : state() === "recently_deleted" ? "Recently Deleted" : !state() ? only(props.all, props.owner) : undefined)
  return <Row heading={props.owner.name} text={text()} end={end} />
}

/// What's failed of a plugin for an owner, on the plugin's page: installing it, or its update.
function Failed(props: { all: Owners; owner: Owner; listed: Listed }) {
  const done = () => props.owner.installs.find((install) => install.plugin === props.listed.plugin)
  const busy = () => working() === `${props.owner.who}|${props.listed.plugin}|retry`
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

/// A plugin's page: what it is, a row for each owner, then what it gives, what it requires and why, and See All.
function Page(props: { market: Market; all: Owners; listed: Listed }) {
  // The first owner it could be installed for has the one main button.
  const first = () => props.all.owners.find((owner) => !only(props.all, owner) && !owner.installs.some((install) => install.plugin === props.listed.plugin))
  return (
    <div class="page">
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
      </div>
      <Show when={props.listed.plugin} fallback={<p class="quiet">Ask your AI to check it and install it.</p>}>
        <List class="owners">
          <For each={props.all.owners} keyed={(owner) => owner.who}>
            {(owner) => <Where market={props.market} all={props.all} owner={owner()} listed={props.listed} first={first() === owner()} />}
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
        <Show when={props.listed.apps.length}>
          <dt>Apps</dt>
          <dd class="apps">
            <For each={props.listed.apps}>{(app) => <Tile listed={app} />}</For>
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
        <Show when={props.listed.source}>
          <dt>Source</dt>
          <dd>{props.listed.source}</dd>
        </Show>
      </dl>
      <Show when={props.listed.tools.length}>
        <List>
          <Row heading="See All" onClick={() => to({ ...view(), plugin: key(props.listed), details: true })} end={<Icon name="caret-right" along />} />
        </List>
      </Show>
    </div>
  )
}

/// See All: everything else a plugin is made of.
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

/// Installed: Updates, each owner's plugins, then Recently Deleted.
function Installed(props: { market: Market; all: Owners; selected?: string; onSelect: (plugin: string) => void }) {
  const updates = () => {
    const each = new Map<string, { owners: Owner[]; install: Install }>()
    for (const owner of props.all.owners)
      for (const install of owner.installs)
        if (install.state === "installed" && install.update && install.plugin) {
          const at = each.get(install.plugin) ?? { owners: [], install: { ...install } }
          at.owners.push(owner)
          each.set(install.plugin, at)
        }
    return [...each.values()]
  }
  const available = () => updates().filter(({ install }) => install.update?.state === "available")
  const updateAll = async () => {
    for (const { owners, install } of available()) await update(owners[0], install)
  }
  const deleted = () => props.all.owners.flatMap((owner) => owner.installs.filter((install) => install.state === "recently_deleted").map((install) => ({ owner, install })))
  const anything = () => props.all.owners.some((owner) => owner.installs.length)
  return (
    <Show when={anything()} fallback={<Empty>Nothing is installed yet.</Empty>}>
      <Show when={updates().length}>
        <section class="group">
          <header>
            <h2>Updates</h2>
            <Show when={available().length > 1}>
              <Button variant="ghost" onClick={updateAll}>Update All</Button>
            </Show>
          </header>
          <List>
            <For each={updates()} keyed={({ install }) => install.plugin!}>
              {(entry) => {
                const it = () => listed(props.market, entry().install)
                const state = () => entry().install.update?.state
                const end = (
                  <Switch fallback={<Button variant="secondary" loading={working() === `${entry().owners[0].who}|${entry().install.plugin}|update`} onClick={() => update(entry().owners[0], entry().install)}>Update</Button>}>
                    <Match when={state() === "updating"}>
                      <Spinner />
                    </Match>
                    <Match when={state() === "failed"}>
                      <Button variant="outline" loading={working() === `${entry().owners[0].who}|${entry().install.plugin}|retry`} onClick={() => retry(entry().owners[0], entry().install)}>Try Again</Button>
                    </Match>
                  </Switch>
                )
                return (
                  <Row
                    heading={it().title}
                    text={entry().install.update?.state === "failed" ? "Couldn't update" : `for ${and(entry().owners.map((owner) => owner.name))}`}
                    start={<Show when={it().icon}><img class="list-icon" src={it().icon!} alt="" /></Show>}
                    action={end}
                    current={props.selected === entry().install.plugin}
                    onClick={() => props.onSelect(entry().install.plugin!)}
                  />
                )
              }}
            </For>
          </List>
        </section>
      </Show>
      <For each={props.all.owners.filter((owner) => owner.installs.some((install) => install.state !== "recently_deleted"))} keyed={(owner) => owner.who}>
        {(owner) => (
          <Section heading={owner().name}>
            <List>
              <For each={owner().installs.filter((install) => install.state !== "recently_deleted")} keyed={(install) => install.name}>
                {(install) => {
                  const it = () => listed(props.market, install())
                  const text = () => (install().state === "installing" ? "Installing" : install().state === "not_installed" ? "Couldn't install" : (it().subtitle ?? undefined))
                  return (
                    <Row
                      heading={it().title}
                      text={text()}
                      start={<Show when={it().icon}><img class="list-icon" src={it().icon!} alt="" /></Show>}
                      end={<Show when={install().state === "installing"}><Spinner /></Show>}
                      current={props.selected === install().plugin}
                      onClick={() => props.onSelect(install().plugin!)}
                    />
                  )
                }}
              </For>
            </List>
          </Section>
        )}
      </For>
      <Show when={deleted().length}>
        <Section heading="Recently Deleted">
          <List>
            <For each={deleted()} keyed={({ owner, install }) => `${owner.who}|${install.name}`}>
              {(entry) => {
                const it = () => listed(props.market, entry().install)
                return (
                  <Row
                    heading={it().title}
                    text={`for ${entry().owner.name}, ${ago(entry().install.deleted ?? Date.now())}`}
                    start={<Show when={it().icon}><img class="list-icon" src={it().icon!} alt="" /></Show>}
                    action={<Button variant="secondary" loading={working() === `${entry().owner.who}|${entry().install.plugin}|restore`} onClick={() => restore(entry().owner, entry().install)}>Restore</Button>}
                  />
                )
              }}
            </For>
          </List>
        </Section>
      </Show>
    </Show>
  )
}

function Plugins(props: { market: Market; all: Owners }) {
  const [selected, setSelected] = createSignal<string>()
  const opened = () => props.market.plugins.find((listed) => key(listed) === view().plugin)
  const chosen = () => {
    const plugin = selected()
    if (!plugin) return undefined
    const install = props.all.owners.flatMap((owner) => owner.installs).find((install) => install.plugin === plugin)
    return props.market.plugins.find((listed) => listed.plugin === plugin) ?? (install && listed(props.market, install))
  }
  const count = () => new Set(props.all.owners.flatMap((owner) => owner.installs.filter((install) => install.update?.state === "available").map((install) => install.plugin))).size
  const top = (
    <Tabs.Root value={view().tab} onValueChange={({ value }) => tab(value as View["tab"])}>
      <Tabs.List>
        <Tabs.Trigger value="marketplace">Marketplace</Tabs.Trigger>
        <Tabs.Trigger value="installed">
          Installed
          <Show when={count()}>
            <i class="count">{count()}</i>
          </Show>
        </Tabs.Trigger>
        <Tabs.Indicator />
      </Tabs.List>
    </Tabs.Root>
  )
  return (
    <Show
      when={!(view().plugin && opened())}
      fallback={
        <Show when={view().details} fallback={<Screen back><Page market={props.market} all={props.all} listed={opened()!} /></Screen>}>
          <Details listed={opened()!} />
        </Show>
      }
    >
      <Show
        when={view().tab === "installed"}
        fallback={
          <Screen heading="Plugins" back onBack={() => (view().category ? history.back() : leave())} class="plugins">
            {top}
            <Marketplace market={props.market} />
          </Screen>
        }
      >
        <Split open={!!chosen()} onClose={() => setSelected()} detail={<Show when={chosen()} fallback={<Empty>Pick a plugin to see it.</Empty>}><Screen back><Page market={props.market} all={props.all} listed={chosen()!} /></Screen></Show>}>
          <Screen heading="Plugins" back onBack={leave} class="plugins">
            {top}
            <Installed market={props.market} all={props.all} selected={selected()} onSelect={setSelected} />
          </Screen>
        </Split>
      </Show>
    </Show>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Plugins" back onBack={leave}><Problem heading="Can't show plugins" text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading="Plugins" back onBack={leave}><Skeleton rows={4} /></Screen>}>
        <Plugins market={market()} all={owners()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
