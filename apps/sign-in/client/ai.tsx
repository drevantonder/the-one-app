// AI: one short prompt that points an assistant at the network's llms.txt, which says how it comes in.
import { createMemo, Errored, Loading, Show } from "solid-js"
import { Button, client, Empty, Problem, Screen, share, Skeleton, styles, Text, Textarea, toast, tokens, View } from "ui"

const look = styles({ readable: { gap: tokens.length.gutter, maxWidth: tokens.length.readable } })

const network = createMemo(() => client().then(({ network }) => network))

function Connect(props: { network: string }) {
  const host = () => props.network.replace(/^[a-z]+:\/\//, "").split("/")[0]
  const prompt = () => `Connect to my network at ${host()}: read ${props.network}/llms.txt and do what it says.`
  const send = async () => {
    try {
      if ((await share(prompt())) === "copied") toast("Copied")
    } catch (error) {
      toast(error as Error)
    }
  }
  return (
    <View style={look.readable}>
      <Text>Send this to Claude, ChatGPT or any other AI.</Text>
      <Textarea aria-label="What to send" readOnly value={prompt()} />
      <Button onPress={send}>Share</Button>
    </View>
  )
}

/// AI, as Settings draws it: how to connect an AI to the network.
export default function AI() {
  return (
  <Screen heading="AI" back>
    <Errored fallback={(error) => <Problem heading="Can't show how to connect" text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={1} />}>
        <Show when={network()} fallback={<Empty>This network has no address, so an AI elsewhere can't reach it.</Empty>}>
          <Connect network={network()!} />
        </Show>
      </Loading>
    </Errored>
  </Screen>
)
}
