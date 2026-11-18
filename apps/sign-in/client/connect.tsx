// Connect Your AI: one short prompt that points an assistant at the network's llms.txt, which says how it comes in.
import { createMemo, Errored, Loading, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, client, Empty, Field, Problem, Screen, share, Skeleton, Textarea, toast } from "ui"
import "./app.css"

const network = createMemo(() => client().then(({ network }) => network))

function Connect(props: { network: string }) {
  const prompt = () => `Connect to my network at ${new URL(props.network).host}: read ${props.network}/llms.txt and do what it says.`
  const send = async () => {
    try {
      if ((await share(prompt())) === "copied") toast("Copied")
    } catch (error) {
      toast(error as Error)
    }
  }
  return (
    <div class="readable">
      <Field label="Send this to Claude, ChatGPT or any other AI.">
        <Textarea readonly rows={3} value={prompt()} onFocus={(event) => event.currentTarget.select()} />
      </Field>
      <Button onClick={send}>Share</Button>
    </div>
  )
}

render(
  () => (
    <Screen heading="Connect Your AI">
      <Errored fallback={(error) => <Problem heading="Can't show how to connect" text={error() as Error} />}>
        <Loading fallback={<Skeleton rows={1} />}>
          <Show when={network()} fallback={<Empty>This network has no address, so an AI elsewhere can't reach it.</Empty>}>
            <Connect network={network()!} />
          </Show>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
