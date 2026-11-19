// Notifications: what you've been told, newest first. Tapping one marks it read and opens the app it's from. A phone or
// a browser that isn't woken by them says so first, and why, and a browser turns them on here. Which apps may notify you
// is in Settings (settings.tsx).
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { call, changed, client, confirm, Empty, List, ListStyle, Menu, open, Problem, render, Row, Screen, Section, since, Skeleton, styles, Text, toast, tokens, View } from "ui"
import { type Push, PushState } from "./push.tsx"

/** What hasn't been read has a blue dot before it, as Mail's rows do, and a heavier weight. */
const look = styles({
  unread: { fontWeight: tokens.weight.medium },
  dot: { width: tokens.grid[2.5], height: tokens.grid[2.5], borderRadius: tokens.radius.full },
  blue: { backgroundColor: tokens.colour.blue },
})

/** A notification, as `notifications_inbox` answers it. */
interface Received {
  id: string
  title: string
  text?: string
  from?: string
  app?: string
  /// Its app's page a tap opens, where it isn't its first.
  page?: string
  /// What a tap opens, as `open` takes it: its app's page, or its section in Settings.
  opens?: string
  at: string
  read: boolean
}

/** An app that has told its person something, as `notifications_apps` answers it. */
interface App {
  app: string
  name: string
  allowed: boolean
}


const inbox = createMemo(async () => {
  const [{ notifications }, apps, { push, settings }]: [{ notifications: Received[] }, App[], { push?: Push; settings?: boolean }] = await Promise.all([
    call("notifications_inbox").then(JSON.parse),
    call("notifications_apps").then((text) => JSON.parse(text).apps),
    client(),
  ])
  return { notifications, apps, push, settings: settings === true }
})
changed(() => refresh(inbox))

/// Does something to the inbox, and says what the server said if it wouldn't.
async function change(tool: string, input = {}) {
  try {
    await call(tool, input)
  } catch (error) {
    toast(error as Error)
  }
  refresh(inbox)
}

function tapped(notification: Received) {
  if (!notification.read) change("notifications_read", { id: notification.id })
  if (notification.opens) open(notification.opens)
}

async function clearAll() {
  const question = { heading: "Clear all notifications?", text: "They're gone from every device of yours.", action: "Clear All", destructive: true }
  if (await confirm(question)) change("notifications_clear")
}

const More = () => (
  <Menu.Root onSelect={(value) => (value === "read" ? change("notifications_read") : clearAll())}>
    <Menu.Trigger />
    <Menu.Positioner>
      <Menu.Content>
        <Menu.Item value="read">Mark All Read</Menu.Item>
        <Menu.Separator />
        <Menu.Item value="clear" destructive>Clear All</Menu.Item>
      </Menu.Content>
    </Menu.Positioner>
  </Menu.Root>
)

/** What `inbox` answers, once it has. */
type Inbox = Awaited<ReturnType<typeof inbox>>

function Inbox(props: { inbox: Inbox }) {
  return (
    <ListStyle value="insetGrouped">
    <Screen heading="Notifications" action={<Show when={props.inbox.notifications.length}><More /></Show>}>
      <Show when={props.inbox.push}>{(push) => <PushState push={push()} settings={props.inbox.settings} again={() => refresh(inbox)} />}</Show>
      <Section>
        <Show when={props.inbox.notifications.length} fallback={<Empty>No notifications.</Empty>}>
          <List>
            <For each={props.inbox.notifications}>
              {(notification) => (
                <Row
                  start={<View style={[look.dot, !notification.read && look.blue]} />}
                  heading={<Text style={!notification.read && look.unread}>{notification.title}</Text>}
                  text={[notification.from, notification.text].filter(Boolean).join(" · ")}
                  end={since(notification.at, "narrow")}
                  onPress={() => tapped(notification)}
                />
              )}
            </For>
          </List>
        </Show>
      </Section>
    </Screen>
    </ListStyle>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Notifications"><Problem heading="Can't show your notifications" text={error() as Error} /></Screen>}>
      <Loading fallback={<Screen heading="Notifications"><Skeleton rows={4} /></Screen>}>
        <Inbox inbox={inbox()} />
      </Loading>
    </Errored>
  ),
)
