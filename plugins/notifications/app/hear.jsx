// Notifications, in Settings: the apps that have notified whoever's looking, and whether they hear from each.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { call, changed, Empty, leave, Problem, Screen, Skeleton, Switch, toast } from "ui"

const apps = createMemo(() => call("notifications_apps").then(JSON.parse))
changed(() => refresh(apps))

/// Silences an app or hears from it again, and says why if that couldn't be done.
async function hear(app, hear) {
  await call("notifications_hear", { app, hear }).catch((error) => toast(error.message))
  refresh(apps)
}

render(
  () => (
    <Screen heading="Notifications" back onBack={leave}>
      <Errored fallback={(error) => <Problem heading="Can't show your notifications' apps" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={3} />}>
          <Show when={apps().length} fallback={<Empty>No app has notified you yet.</Empty>}>
            <form>
              <For each={apps()} keyed={(app) => app.app}>
                {(app) => <Switch label={app().name} checked={app().hear} onChange={(event) => hear(app().app, event.currentTarget.checked)} />}
              </For>
            </form>
          </Show>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
