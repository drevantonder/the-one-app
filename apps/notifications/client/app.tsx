// Notifications: what you've been told, newest first, and the apps that have told you
// something, each of which you hear from or not. Tapping one marks it read and opens the app it's from. A phone that
// isn't woken by them says so first, and why.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, client, confirm, Empty, List, Menu, open, Problem, Row, Screen, Section, Skeleton, Spinner, Switch, toast } from "ui"
import "./app.css"

/** A notification, as `notifications_inbox` answers it. */
interface Received {
  id: string
  title: string
  text?: string
  from?: string
  app?: string
  at: string
  read: boolean
}

/** An app that has told its person something, as `notifications_apps` answers it. */
interface App {
  app: string
  name: string
  allowed: boolean
}

/** Where this phone is with being woken by its notifications, as `client()` says: nothing on a computer or in a browser. */
type Push = { state: "notAsked" | "denied" | "registering" | "registered" } | { state: "failed"; why: string }

const inbox = createMemo(async () => {
  const [{ notifications }, apps, { push }]: [{ notifications: Received[] }, App[], { push?: Push }] = await Promise.all([
    call("notifications_inbox").then(JSON.parse),
    call("notifications_apps").then((text) => JSON.parse(text).apps),
    client(),
  ])
  return { notifications, apps, push }
})
changed(() => refresh(inbox))

/// How long ago, as a person says it: "now", "5 min", "3 h", then the day.
function ago(at: string) {
  const minutes = Math.round((Date.now() - new Date(at).getTime()) / 60_000)
  if (minutes < 1) return "now"
  if (minutes < 60) return `${minutes} min`
  if (minutes < 24 * 60) return `${Math.round(minutes / 60)} h`
  return new Date(at).toLocaleDateString(undefined, { day: "numeric", month: "short" })
}

/// Does something to the inbox, and says what the server said if it wouldn't.
async function change(tool: string, input = {}) {
  try {
    await call(tool, input)
  } catch (error) {
    toast(error as Error)
  }
  refresh(inbox)
}

/// Has this phone try again to be woken by its notifications, and shows where that left it.
async function tryPush() {
  try {
    await client({ push: true })
  } catch (error) {
    toast(error as Error)
  }
  refresh(inbox)
}

/// Where this phone is with being woken by its notifications, said only where it isn't: fine is silent, and so is a
/// computer, which shows them while the app runs, and a browser.
function PushState(props: { push: Push }) {
  return (
    <>
      <Show when={props.push.state === "registering"}>
        <p class="push" role="status">
          <Spinner />
          <span>Turning on notifications…</span>
        </p>
      </Show>
      <Show when={props.push.state === "denied"}>
        <Problem warn heading="Notifications are off for this phone" text="Turn them on for this app in its Settings." />
      </Show>
      <Show when={props.push.state === "failed"}>
        <Problem warn heading="This phone isn't woken by notifications" text={"why" in props.push ? props.push.why : undefined} action={<Button variant="outline" onClick={tryPush}>Try Again</Button>} />
      </Show>
    </>
  )
}

function tapped(notification: Received) {
  if (!notification.read) change("notifications_read", { id: notification.id })
  if (notification.app) open(notification.app)
}

async function clearAll() {
  const question = { heading: "Clear all notifications?", text: "They're gone from every device of yours.", action: "Clear All", destructive: true }
  if (await confirm(question)) change("notifications_clear")
}

const More = () => (
  <Menu.Root onSelect={({ value }) => (value === "read" ? change("notifications_read") : clearAll())}>
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

render(
  () => (
    <Screen heading="Notifications" action={<Show when={inbox()?.notifications.length}><More /></Show>}>
      <Errored fallback={(error) => <Problem heading="Can't show your notifications" text={error() as Error} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <Show when={inbox().push}>{(push) => <PushState push={push()} />}</Show>
          <Section>
            <Show when={inbox().notifications.length} fallback={<Empty>No notifications.</Empty>}>
              <List class="inbox">
                <For each={inbox().notifications}>
                  {(notification) => (
                    <Row
                      heading={<span class={{ unread: !notification.read }}>{notification.title}</span>}
                      text={[notification.from, notification.text].filter(Boolean).join(" · ")}
                      end={ago(notification.at)}
                      onClick={() => tapped(notification)}
                    />
                  )}
                </For>
              </List>
            </Show>
          </Section>
          <Show when={inbox().apps.length}>
            <Section heading="Allow Notifications">
              <form>
                <For each={inbox().apps} keyed={(app) => app.app}>
                  {(app) => <Switch label={app().name} checked={app().allowed} onChange={(event) => change("notifications_allow", { app: app().app, allow: event.currentTarget.checked })} />}
                </For>
              </form>
            </Section>
          </Show>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
