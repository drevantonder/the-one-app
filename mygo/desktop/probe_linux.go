package main

// Linux has no platform passkey API (no WebAuthn in WebKitGTK either): a passkey is the browser's, which signing in
// already uses. Nor has it a push service a desktop app registers with: the node's own connection is how a Linux
// client hears (the web client's Web Push, in a browser).
func passkeyProbe() string {
	return "none on Linux: no platform authenticator API, and WebKitGTK has no WebAuthn; sign-in uses the system browser"
}

func pushProbe() string {
	return "none on Linux: no OS push service; a Linux client hears through its connection to the node"
}
