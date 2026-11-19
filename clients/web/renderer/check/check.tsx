// The renderer's gate on a person's own devices (`clients/web/renderer/check.html` hosts it): a page drawn by the
// renderer with what each check needs, typing with suggestions, dictation, a password manager and a passkey, and two
// buttons each, which say to the check's host how it went. (Screen readers wait: Andre, 20 Nov, "we don't have
// accessibility requirements yet".)
import { createSignal } from "solid-js"
import { Button, Field, Input, render, Screen, Section, Text, Textarea, View } from "ui"

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

/// What became of the passkey offered, as the page shows it.
const [offered, setOffered] = createSignal("")

// A passkey is offered once the passkey field has the keyboard, so its real field is there for the browser or a
// password manager's extension (1Password's takes WebAuthn over in Chrome) to offer it in, as a site's sign-in does
// (WebAuthn's conditional mediation): which one was picked is told, and nothing signs in. A request that ends without
// one is shown with what it means, not counted: the person's buttons say whether it was offered.
let asked = false
async function offerPasskey() {
  if (asked) return
  asked = true
  if (!(await PublicKeyCredential.isConditionalMediationAvailable?.())) return setOffered("This browser offers no passkeys in a field.")
  setOffered("Offered: pick your passkey from the field's suggestions.")
  try {
    const challenge = crypto.getRandomValues(new Uint8Array(32))
    const picked = await navigator.credentials.get({ mediation: "conditional", publicKey: { challenge, rpId: location.hostname, userVerification: "preferred" } } as CredentialRequestOptions)
    if (picked) say("passkey", true, "a passkey was picked from the field's suggestions")
    setOffered(picked ? "Picked. Nothing signs in." : "Nothing was picked.")
  } catch (error) {
    // NotAllowedError is WebAuthn's for a request dismissed, timed out, or that the device or extension had no
    // passkey for (this network's, at this address) to offer.
    const name = (error as Error)?.name
    setOffered(name === "NotAllowedError" ? "It ended without a passkey: dismissed, timed out, or none here for this address in your browser or password manager." : `It didn't ask: ${String(error)}`)
    asked = false
  }
}

function Check() {
  const [typed, setTyped] = createSignal("")
  const [dictated, setDictated] = createSignal("")
  const [user, setUser] = createSignal("")
  const [password, setPassword] = createSignal("")
  const [passkey, setPasskey] = createSignal("")
  return (
    <Screen heading="Renderer Check">
      <Section heading="1. Typing with suggestions">
        <Text>Type a sentence with a typo. Safari, on a Mac and an iPhone, offers suggestions and autocorrect in a web field; Chrome on a Mac offers none in any web field, so there it worked if what's typed shows, with its caret, and a press, a drag and a double click select it.</Text>
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
        <Field label="Passkey" hint={offered()}><Input autoComplete="username webauthn" autoCapitalize="none" autoCorrect={false} value={passkey()} onChangeText={setPasskey} onFocus={offerPasskey} /></Field>
        <Verdict check="passkey-offered" />
      </Section>
    </Screen>
  )
}

render(() => <Check />)
