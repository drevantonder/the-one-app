// Settings: one place where every app brings its settings, laid out as Raycast's Settings: a sidebar with its search,
// whoever's looking (Sign In & Security under their name), the apps' sections in their groups, then every app they
// have, A to Z. What's chosen opens beside it on a computer and is pushed over it on a phone. A section is its app's
// component (`settings.sections`), drawn here if the app is trusted and by its page in a frame if it isn't, as the
// Vault draws an item type's. Updates and an app's page are Settings' own.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { Badge, Button, call, changed, client, component, confirm, Empty, Frame, History, Image, link, List, ListStyle, Problem, render, Row, Screen, Search, Section, Skeleton, Split, styles, Switch, Text, toast, tokens, View, viewport } from "ui"

/** What an app adds to Settings, as `settings_sections` answers it. */
interface Added {
  id: string
  app: string
  title: string
  icon: string | null
  app_icon: string | null
  trusted: boolean
  component: string
  in: "settings" | "device" | "app"
  name: string | null
  group: "you" | "network" | "general" | "system" | null
  order: number
  count: number | null
  kinds: string[] | null
  service: boolean
}

/** Where an app's update is, as `apps` answers it. */
interface Update {
  state: string
  stage?: string | null
  why?: string | null
}

/** An app as Settings lists it: `settings_apps` with what `apps` says of it. */
interface App {
  name: string
  app: string
  for: string
  title: string
  icon: string | null
  removes: boolean
  state?: string
  trusted?: boolean | null
  update?: Update | null
  required_by?: string[]
}

/** What `builds` answers that Updates shows. */
interface Builds {
  devices: { device: string; build?: string; target?: string }[]
  latest: Record<string, { build: string }>
}

const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

const added = createMemo(() => json<{ sections: Added[] }>("settings_sections").then(({ sections }) => sections))
const apps = createMemo(async () => {
  const [{ apps: listed }, { apps: installed }] = await Promise.all([json<{ apps: App[] }>("settings_apps"), json<{ apps: App[] }>("apps")])
  return listed.map((app) => ({ ...installed.find((one) => one.name === app.name && one.for === app.for), ...app }))
})
/// Whoever's looking, as they named themselves, or nothing where Sign-In can't say.
const me = createMemo(() => json<{ people: { who: string; called: string | null }[] }>("sign-in_people").then(({ people }) => people[0], () => undefined))
changed(() => [added, apps, me].forEach(refresh))

/// Where a client reads an icon a tool gave as `ui://<app>/<code>/<path>`: beside this page's own files.
const src = (icon: string) => icon.replace(/^ui:\/\/([^/]+)\/([^/]+)\//, "../../$1@$2/")

const look = styles({
  icon: { width: tokens.length.sidebarIcon, height: tokens.length.sidebarIcon },
  /// A group's heading in another section's page: its app's icon and name.
  heading: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2] },
  small: { width: tokens.grid[4], height: tokens.grid[4] },
  avatar: {
    width: tokens.grid[8],
    height: tokens.grid[8],
    borderRadius: tokens.radius.full,
    backgroundColor: tokens.colour.tertiaryLabel,
    alignItems: "center",
    justifyContent: "center",
  },
  initials: { font: tokens.type.small, fontWeight: tokens.weight.semibold, color: tokens.colour.onAccent },
  /// Why an app can't be removed, under Remove App.
  why: { font: tokens.type.small, color: tokens.colour.secondaryLabel, paddingInline: tokens.grid[3] },
  personName: { fontWeight: tokens.weight.semibold },
  readable: { gap: tokens.length.gutter, maxWidth: tokens.length.readable },
})

/// What's open beside the sidebar, by its id: a section's (`<app>/<component>`), `updates`, or an app's page
/// (`app:<for>:<name>`). A link opens one (`one:settings#sign-in/devices`), with what follows `&` for the section itself.
const [open, setOpen] = createSignal<string | undefined>(() => link()?.split("&")[0] || undefined)

/// Whether what's open sits beside the sidebar, where the first section shows until another is chosen.
const beside = () => viewport().width >= 768

/// The sidebar's groups, in order: whoever's looking, then the three the apps' sections are in.
const groups = ["you", "network", "general", "system"] as const
const sidebar = (all: Added[]) =>
  all.filter((one) => one.in === "settings").toSorted((a, b) => groups.indexOf(a.group!) - groups.indexOf(b.group!) || a.order - b.order)

function Tile(props: { icon: string | null; small?: boolean }) {
  return <Show when={props.icon}>{(icon) => <Image src={src(icon())} alt="" style={props.small ? look.small : look.icon} />}</Show>
}

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => [...word][0]!.toUpperCase())
    .join("")

/// The sidebar: search, whoever's looking, the apps' sections by group, then every app A to Z.
function Sidebar(props: { shown?: string }) {
  const [words, setWords] = createSignal("")
  const wanted = (name: string) => name.toLowerCase().includes(words().trim().toLowerCase())
  const of = (group: Added["group"]) => sidebar(added()).filter((one) => one.group === group && wanted(one.name!))
  const person = () => sidebar(added()).find((one) => one.group === "you")
  const choose = (id: string) => choosing(id)
  const row = (section: () => Added) => (
    <Row
      start={<Tile icon={section().icon ?? section().app_icon} />}
      heading={section().name!}
      end={<Show when={section().count}><Badge variant="destructive">{section().count}</Badge></Show>}
      current={props.shown === section().id}
      onPress={() => choose(section().id)}
    />
  )
  const own = () => wanted("Updates")
  return (
    <Screen heading="Settings">
      <Search placeholder="Search settings…" aria-label="Search Settings" value={words()} onChangeText={setWords} />
      <Show when={person() && !words()}>
        <List>
          <Row
            start={
              <View style={look.avatar}>
                <Text style={look.initials}>{initials(me()?.called ?? "You")}</Text>
              </View>
            }
            heading={<Text style={look.personName}>{me()?.called ?? "You"}</Text>}
            text={person()!.name!}
            current={props.shown === person()!.id}
            onPress={() => choose(person()!.id)}
          />
        </List>
      </Show>
      <For each={groups.slice(1)}>
        {(group) => (
          <Show when={of(group).length || (group === "system" && own())}>
            <List>
              <For each={of(group)} keyed={(one) => one.id}>{row}</For>
              <Show when={group === "system" && own()}>
                <Row start={<Image src="updates.svg" alt="" style={look.icon} />} heading="Updates" current={props.shown === "updates"} onPress={() => choose("updates")} />
              </Show>
            </List>
          </Show>
        )}
      </For>
      <Show when={apps().filter((app) => wanted(app.title)).length}>
        <Section heading="Apps">
          <List each={apps().filter((app) => wanted(app.title))} keyed={(app) => `${app.for}:${app.name}`}>
            {(app) => (
              <Row
                start={<Tile icon={app().icon} />}
                heading={app().title}
                end={<Show when={app().update}><Badge variant="secondary">Update</Badge></Show>}
                current={props.shown === `app:${app().for}:${app().name}`}
                onPress={() => choose(`app:${app().for}:${app().name}`)}
              />
            )}
          </List>
        </Section>
      </Show>
    </Screen>
  )
}

/// What's open: a section of an app's, Updates, or an app's page.
function Detail(props: { id: string }) {
  // The same section while what draws it is: `added` asked again, as it is whenever anything changes, gives new
  // objects, and a new one would draw the section from scratch, losing what's open in it (a live screen's stream).
  const section = createMemo(() => added().find((one) => one.id === props.id && one.in === "settings"), {
    equals: (a, b) => a?.id === b?.id && a?.app === b?.app && a?.component === b?.component && a?.name === b?.name && a?.title === b?.title,
  })
  const app = () => apps().find((one) => props.id === `app:${one.for}:${one.name}`)
  return (
    <Show when={section()} keyed fallback={<Show when={props.id === "updates"} fallback={<Show when={app()} keyed>{(app) => <AppPage app={app} />}</Show>}><Updates /></Show>}>
      {(section) => <Drawn section={section} />}
    </Show>
  )
}

/// A section, drawn by its app's own component in this page, with what Settings' page may do. It's given the groups
/// other apps add to the places it offers (`Groups`).
function Drawn(props: { section: Added }) {
  const drawn = createMemo(() => component(props.section.app, props.section.component).then((module) => module.default))
  const heading = () => props.section.name ?? props.section.title
  return (
    <Errored fallback={(error) => <Screen heading={heading()} back><Problem heading={`Can't show ${heading()}`} text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading={heading()} back><Skeleton rows={4} /></Screen>}>
        <Show when={drawn()} keyed>
          {(Drawn) => <Drawn groups={Groups} />}
        </Show>
      </Loading>
    </Errored>
  )
}

/// What other apps add to a place a section offers (`in`): each a group headed by its app's icon and name, drawn by
/// its app's component, given what the place is about (`device`), or where its app is untrusted, by its page for it
/// in a frame (`client/<component>.html`, the device after `#`), as tall as the page says it is. An app's own page has
/// only that app's.
function Groups(props: { in: Added["in"]; device?: string; kind?: string; server?: string; services?: string[]; app?: string }) {
  const fits = (one: Added) =>
    props.in === "app" ? one.app === props.app : (!one.kinds || one.kinds.includes(props.kind!)) && (!one.service || !!props.services?.includes(one.app))
  const here = () => added().filter((one) => one.in === props.in && fits(one))
  return (
    <For each={here()} keyed={(one) => one.id}>
      {(one) => (
        <Section
          heading={
            <View style={look.heading}>
              <Tile icon={one().app_icon} small />
              <Text>{one().title}</Text>
            </View>
          }
        >
          <Show
            when={one().trusted}
            fallback={<Frame src={`${one().app}/client/${one().component}.html${props.device ? `#${props.device}` : ""}`} title={one().title} />}
          >
            <Group added={one()} device={props.device} server={props.server} />
          </Show>
        </Section>
      )}
    </For>
  )
}

function Group(props: { added: Added; device?: string; server?: string }) {
  const drawn = createMemo(() => component(props.added.app, props.added.component).then((module) => module.default))
  return (
    <Errored fallback={(error) => <Problem heading={`Can't show ${props.added.title}`} text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={1} />}>
        <Show when={drawn()} keyed>
          {(Drawn) => <Drawn device={props.device} server={props.server} />}
        </Show>
      </Loading>
    </Errored>
  )
}

/// Does something, saying why where it couldn't, and shows everything as it is after. → whether it was done.
async function doing(does: () => Promise<unknown>) {
  try {
    await does()
    return true
  } catch (error) {
    toast(error as Error)
    return false
  } finally {
    ;[added, apps].forEach(refresh)
  }
}

/// What's waiting to update: this device and each app with a newer version ready. The network's servers' is Software
/// Update's, at the top of Servers (Devices).
const updates = createMemo(async () => {
  const [mine, network, here] = await Promise.all([json<Builds>("builds"), json<Builds>("builds", {}, "network").catch(() => undefined), client()])
  const latest = { ...network?.latest, ...mine.latest }
  const target = mine.devices.find((device) => device.device === here.device)?.target
  const newest = target && latest[`one/${target}`]?.build
  return { behind: !!newest && newest !== here.build }
})

function Updates() {
  const [working, setWorking] = createSignal<string>()
  const press = (id: string, does: () => Promise<unknown>) => async () => {
    setWorking(id)
    await doing(does)
    refresh(updates)
    setWorking()
  }
  const ready = () => apps().filter((app) => app.update)
  return (
    <Screen heading="Updates" back>
      <Errored fallback={(error) => <Problem heading="Can't show what's waiting to update" text={error() as Error} />}>
        <Loading fallback={<Skeleton rows={2} />}>
          <List>
            <Row
              heading="This Device"
              text={updates().behind ? "Update ready" : "Up to date"}
              end={<Show when={updates().behind}><Button variant="outline" loading={working() === "here"} onPress={press("here", () => client({ update: true }).then(({ message }) => toast(message)))}>Update</Button></Show>}
            />
          </List>
          <Show when={ready().length}>
            <Section heading="Apps">
              <List each={ready()} keyed={(app) => `${app.for}:${app.name}`}>
                {(app) => (
                  <Row
                    start={<Tile icon={app().icon} />}
                    heading={app().title}
                    text={app().update!.why ?? "Update ready"}
                    end={<Button variant="outline" loading={working() === app().name} onPress={press(app().name, () => call("apps_update", { name: app().name }, { subject: app().for }))}>Update</Button>}
                  />
                )}
              </List>
            </Section>
          </Show>
        </Loading>
      </Errored>
    </Screen>
  )
}

/// Who an app is installed for, as a person reads it.
const whose = (owner: string) => (owner === "network" ? "Everyone" : owner === me()?.who ? "You" : owner.split("/").pop()!)

/// Whether each app may notify whoever's looking, as Notifications keeps it, or nothing where it isn't installed.
const notifying = createMemo(() => json<{ apps: { app: string; allowed: boolean }[] }>("notifications_apps").then(({ apps }) => apps, () => undefined))

/// An app's page: who it's for, whether it's trusted, its notifications, its update, what it adds here, and Remove App.
function AppPage(props: { app: App }) {
  const app = () => props.app
  const [working, setWorking] = createSignal<string>()
  const allowed = () => notifying()?.find((one) => one.app === app().app)
  const trust = (trusted: boolean) => doing(() => call("apps_trust", { name: app().name, trusted }, { subject: app().for }))
  const allow = (allow: boolean) => doing(() => call("notifications_allow", { app: app().app, allow }).then(() => refresh(notifying)))
  const update = async () => {
    setWorking("update")
    await doing(() => call("apps_update", { name: app().name }, { subject: app().for }))
    setWorking()
  }
  const remove = async () => {
    const also = app().required_by ?? []
    const text = also.length
      ? `${also.join(", ")} ${also.length === 1 ? "needs" : "need"} it and will be removed too. They're in Recently Deleted for 30 days.`
      : "It's in Recently Deleted for 30 days, with what it kept."
    if (!(await confirm({ heading: `Remove ${app().title}?`, text, action: "Remove", destructive: true }))) return
    if (await doing(() => call("uninstall", { name: app().name, confirm: also.length > 0 }, { subject: app().for }))) setOpen()
  }
  return (
    <Screen heading={app().title} back>
      <List>
        <Row heading="Installed For" end={<Text>{whose(app().for)}</Text>} />
        <Show when={app().trusted != null}>
          <Row
            heading="Trusted"
            text={app().trusted ? "Runs on your servers, with everything you can reach." : "Runs in a sandbox, with only what it's given."}
            end={<Switch aria-label="Trusted" value={!!app().trusted} disabled={!app().removes} onValueChange={trust} />}
          />
        </Show>
        <Show when={allowed()}>
          {(allowed) => <Row heading="Notifications" end={<Switch aria-label="Notifications" value={allowed().allowed} onValueChange={allow} />} />}
        </Show>
        <Show when={app().update}>
          <Row heading="Update ready" text={app().update!.why ?? undefined} end={<Button variant="outline" loading={working() === "update"} onPress={update}>Update</Button>} />
        </Show>
      </List>
      <Groups in="app" app={app().name} />
      <List>
        <Row
          heading="Remove App…"
          text={app().removes ? "It's in Recently Deleted for 30 days, with what it kept." : `This is ${whose(app().for)}'s, and you're not one of its owners.`}
          destructive={app().removes}
          onPress={app().removes ? remove : undefined}
        />
      </List>
    </Screen>
  )
}

/// What a link names, as Settings opens it: a section, Updates or an app's page as they are; an app by its name alone
/// (`#sign-in`, as the client opens an app with no page of its own, or a page it no longer has), its first section
/// after the person's row, or else its page.
function resolved(id: string) {
  if (id === "updates" || id.startsWith("app:") || added().some((one) => one.id === id && one.in === "settings")) return id
  const theirs = sidebar(added()).filter((one) => one.app === id)
  const section = theirs.find((one) => one.group !== "you") ?? theirs[0]
  if (section) return section.id
  const app = apps().find((one) => one.name === id)
  return app ? `app:${app.for}:${app.name}` : id
}

function Settings() {
  // Beside the sidebar, the first section shows until another is chosen, as Raycast's Settings opens on General.
  const shown = () => {
    const id = open()
    return id ? resolved(id) : beside() ? (sidebar(added()).find((one) => one.group !== "you")?.id ?? "updates") : undefined
  }
  // Back and Forward go through what was open beside the sidebar, as macOS's System Settings' do.
  const [past, setPast] = createSignal<string[]>([])
  const [future, setFuture] = createSignal<string[]>([])
  const history = () => ({
    back: past().length ? () => (setFuture([shown()!, ...future()]), setOpen(past().at(-1)), setPast(past().slice(0, -1))) : undefined,
    forward: future().length ? () => (setPast([...past(), shown()!]), setOpen(future()[0]), setFuture(future().slice(1))) : undefined,
  })
  choosing = (id) => {
    if (id === shown()) return
    if (beside() && shown()) setPast([...past(), shown()!])
    setFuture([])
    setOpen(id)
  }
  // A section is a page of settings, its lists grouped cards, as Raycast's Settings and iOS's Settings draw them.
  return (
    <ListStyle value="sidebar">
    <Split
      open={!!open()}
      onClose={() => setOpen()}
      detail={
        <ListStyle value="insetGrouped">
          <History value={beside() ? history() : null}>
            <Show when={shown()} keyed fallback={<Empty>Choose a section.</Empty>}>{(id) => <Detail id={id} />}</Show>
          </History>
        </ListStyle>
      }
    >
      <Sidebar shown={shown()} />
    </Split>
    </ListStyle>
  )
}

/// Opens a section beside the sidebar, which Back then goes back from (`Settings`).
let choosing = (id: string) => void setOpen(id)

render(() => (
  <Errored fallback={(error, retry) => <Screen heading="Settings"><Problem heading="Can't show Settings" text={error() as Error} action={<Button onPress={() => ([added, apps].forEach(refresh), retry())}>Try Again</Button>} /></Screen>}>
    <Loading fallback={<Screen heading="Settings"><Skeleton rows={6} /></Screen>}>
      <Settings />
    </Loading>
  </Errored>
))
