// The renderer's gate on a person's own devices (`clients/web/renderer/check.html` hosts it): a page drawn by the
// renderer with what each check needs, typing with suggestions, dictation, a password manager, a passkey and a screen
// reader, and two buttons each, which say to the check's host how it went.
import { createSignal, For } from "solid-js"
import { Button, Field, Input, render, Screen, Section, Tabs, Text, Textarea, View } from "ui"

/** Says how a check went, to the page's host, which tells Telemetry. */
const say = (check: string, worked: boolean, note?: string) =>
  (globalThis as unknown as { host: { post(message: object): void } }).host.post({ jsonrpc: "2.0", method: "notifications/one/checked", params: { check, worked, note } })

/** Its two buttons. */
function Verdict(props: { check: string }) {
  const [said, setSaid] = createSignal<string>()
  return (
    <View>
      <Button onPress={() => (say(props.check, true), setSaid("Sent: it worked"))}>It Worked</Button>
      <Button variant="outline" onPress={() => (say(props.check, false), setSaid("Sent: it didn't"))}>It Didn't</Button>
      <Text>{said() ?? ""}</Text>
    </View>
  )
}

// A passkey is offered as the passkey field takes the keyboard, as a site's sign-in offers one (WebAuthn's
// conditional mediation): which one was picked is said, and nothing signs in.
async function offerPasskey() {
  if (!(await PublicKeyCredential.isConditionalMediationAvailable?.())) return say("passkey", false, "this browser offers no passkeys in a field")
  try {
    const challenge = crypto.getRandomValues(new Uint8Array(32))
    const picked = await navigator.credentials.get({ mediation: "conditional", publicKey: { challenge, rpId: location.hostname, userVerification: "preferred" } } as CredentialRequestOptions)
    say("passkey", !!picked, picked ? "a passkey was picked from the field's suggestions" : "nothing was picked")
  } catch (error) {
    say("passkey", false, String(error))
  }
}
offerPasskey()

const TABS = ["One", "Two", "Three"]

function Check() {
  const [typed, setTyped] = createSignal("")
  const [dictated, setDictated] = createSignal("")
  const [user, setUser] = createSignal("")
  const [password, setPassword] = createSignal("")
  const [passkey, setPasskey] = createSignal("")
  const [tab, setTab] = createSignal(TABS[0])
  return (
    <Screen heading="Renderer Check">
      <Section heading="1. Typing with suggestions">
        <Text>Type a sentence with a typo. The keyboard's suggestions show, and picking one or autocorrect changes the word.</Text>
        <Field label="Words"><Input value={typed()} onChangeText={setTyped} /></Field>
        <Verdict check="typing" />
      </Section>
      <Section heading="2. Dictation">
        <Text>Tap the field, then the keyboard's microphone (or press the dictation key on a Mac), and say a sentence.</Text>
        <Field label="Dictated"><Textarea value={dictated()} onChangeText={setDictated} /></Field>
        <Verdict check="dictation" />
      </Section>
      <Section heading="3. Password manager">
        <Text>Tap the password field: the browser offers a strong password, and saving it offers it again here.</Text>
        <Field label="Username"><Input autoComplete="username" autoCapitalize="none" autoCorrect={false} value={user()} onChangeText={setUser} /></Field>
        <Field label="Password"><Input autoComplete="new-password" secureTextEntry value={password()} onChangeText={setPassword} /></Field>
        <Verdict check="autofill" />
      </Section>
      <Section heading="4. Passkey">
        <Text>Tap the field: your passkey for this network is offered above the keyboard (or under the field on a Mac). Pick it.</Text>
        <Field label="Passkey"><Input autoComplete="username webauthn" autoCapitalize="none" autoCorrect={false} value={passkey()} onChangeText={setPasskey} /></Field>
        <Verdict check="passkey-offered" />
      </Section>
      <Section heading="5. VoiceOver">
        <Text>Turn VoiceOver on. Swipe through this section: it reads each heading, tab and button, and double-tapping a tab selects it.</Text>
        <Tabs.Root value={tab()} onValueChange={setTab}>
          <Tabs.List>
            <For each={TABS}>{(name) => <Tabs.Trigger value={name}>{name}</Tabs.Trigger>}</For>
            <Tabs.Indicator />
          </Tabs.List>
        </Tabs.Root>
        <Text>{`Selected: ${tab()}`}</Text>
        <For each={TABS}>{(name) => <Button variant="outline" onPress={() => setTab(name)}>{`Choose ${name}`}</Button>}</For>
        <Verdict check="voiceover" />
      </Section>
    </Screen>
  )
}

render(() => <Check />)
