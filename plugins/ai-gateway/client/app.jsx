// AI Gateway: which models each job tries, in order, then every model there is by where it runs: each machine of
// yours and each hosted endpoint, with what each model does. A place that isn't answering says so where it is.
// Pressing a job chooses its models. Spending opens Insights, which keeps what each model used.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, Empty, leave, List, open, Problem, Row, Screen, Section, seen, Sheet, Skeleton, toast } from "ui"
import "./app.css"

const models = createMemo(() => call("ai-gateway_models").then(JSON.parse))
/// What AI used this week, as Insights keeps it, or nothing where the network has no Insights.
const spent = createMemo(() => call("insights_spending", { days: 7 }).then(JSON.parse, () => null))
changed(() => (refresh(models), refresh(spent)))
const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 })
/// This week's tokens, and what they cost where an endpoint said.
function spending(days) {
  const tokens = days.reduce((sum, day) => sum + day.input + day.output, 0)
  const priced = days.filter((day) => day.cost != null)
  const cost = priced.reduce((sum, day) => sum + day.cost, 0).toLocaleString(undefined, { style: "currency", currency: "USD", maximumFractionDigits: 4 })
  return tokens ? [`${compact.format(tokens)} tokens this week`, priced.length && cost].filter(Boolean).join(" · ") : "No AI used this week"
}

/// The jobs everyone has, and any other someone set.
const jobs = () => {
  const set = new Map(models().jobs.map((job) => [job.job, job]))
  return [...new Set(["chat", "vision", ...set.keys()])].map((job) => set.get(job) ?? { job, models: [], whose: null })
}
const named = (job) => job[0].toUpperCase() + job.slice(1)
/// Every model that's up, as a job can choose them.
const choosable = () => models().places.flatMap((place) => (place.up ? place.models : []))
const does = (model) => [model.vision && "Vision", model.structured && "Structured answers"].filter(Boolean).join(" · ")

/// The job whose models are being chosen, and those chosen so far, in order.
const [choosing, setChoosing] = createSignal()
const [picked, setPicked] = createSignal([])
const choose = (job) => {
  setPicked(job.models)
  setChoosing(job)
}

/// A job's models, in the order they're tried: pressing one adds it to the end, or takes it out.
function JobSheet() {
  const [saving, setSaving] = createSignal(false)
  const toggle = (id) => setPicked((now) => (now.includes(id) ? now.filter((was) => was !== id) : [...now, id]))
  const save = async () => {
    setSaving(true)
    try {
      await call("ai-gateway_set", { job: choosing().job, models: picked() })
      refresh(models)
      setChoosing()
    } catch (error) {
      toast(error.message)
    }
    setSaving(false)
  }
  // Models it tries that aren't up now stay in its list, after those that are.
  const offered = () => [...choosable().map((model) => model.id), ...picked().filter((id) => !choosable().some((model) => model.id === id))]
  return (
    <Sheet heading={choosing() && named(choosing().job)} open={!!choosing()} onClose={() => setChoosing()}>
      <Show when={choosing()}>
        <p class="hint">The first that's up answers. With none chosen, a model on your machines does.</p>
        <List>
          <For each={offered()}>
            {(id) => (
              <Row heading={id} text={does(choosable().find((model) => model.id === id) ?? {})} end={picked().includes(id) ? String(picked().indexOf(id) + 1) : ""} onClick={() => toggle(id)} />
            )}
          </For>
        </List>
        <Button onClick={save} loading={saving()}>
          Save
        </Button>
      </Show>
    </Sheet>
  )
}

render(
  () => (
    <Screen heading="AI Gateway" back onBack={leave}>
      <Errored fallback={(error) => <Problem heading="Can't show your models" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <Section heading="Jobs">
            <List>
              <For each={jobs()}>
                {(job) => (
                  <Row
                    heading={named(job.job)}
                    text={job.models.length ? job.models.join(", then ") : "A model on your machines"}
                    end={job.whose === "everyone's" ? "Everyone's" : ""}
                    onClick={() => choose(job)}
                  />
                )}
              </For>
            </List>
          </Section>
          <Show when={spent()}>
            <List>
              <Row heading="Spending" text={spending(spent())} onClick={() => open("insights")} />
            </List>
          </Show>
          <Show when={models().places.length} fallback={<Empty>No models yet. Run a model server on a machine of yours, or add a key in the Vault.</Empty>}>
            <For each={models().places}>
              {(place) => (
                <Section heading={place.name}>
                  <Show when={place.up} fallback={place.seen ? <Empty>{seen(place.seen)}</Empty> : <Problem warn heading="Isn't answering" text={place.problem} />}>
                    <Show when={place.models.length} fallback={<Empty>It has no models.</Empty>}>
                      <List>
                        <For each={place.models}>{(model) => <Row heading={model.name} text={does(model)} />}</For>
                      </List>
                    </Show>
                  </Show>
                </Section>
              )}
            </For>
          </Show>
        </Loading>
      </Errored>
      <JobSheet />
    </Screen>
  ),
  document.body,
)
