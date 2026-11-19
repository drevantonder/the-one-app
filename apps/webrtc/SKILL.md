---
name: webrtc
description: Read before making an app stream a screen or other live video to a person's app over WebRTC, or when a live view says it couldn't connect.
metadata:
  title: "WebRTC"
  subtitle: "Live video of your screens"
  about: "Shows the screens your apps open, like a browser you sign in on or your AI's desktop, as smooth live video you can click and type on, through Cloudflare's relays."
  category: Admin
---

WebRTC gives an app's code `ice_servers` for one stream: TURN credentials from Cloudflare Realtime, short-lived,
which both ends use with `iceTransportPolicy: "relay"`. A person or an agent can't call it: the app streaming is
what decides who may watch. A screen's tool takes the action `stream`: with no `offer` it answers these ICE servers,
and with the app's whole SDP `offer` its own `{"answer": <sdp>}`. The UI library's `LiveScreen` does the app's end.
The screen's end is WebRTC's sender, one program for every kind of screen: `sender` gives its build for each kind of
machine, which the app's code there downloads once, checks and runs (`server/bin/stream/src/main.rs` says what it
takes).

It needs the item `Cloudflare TURN` in the Vault of whoever it's installed for (the network's, most often): `key_id`
(not secret) and `api_token`, a TURN key's id and secret from Cloudflare (Realtime › TURN, or the API's
`/accounts/<id>/calls/turn_keys`), with `rtc.live.cloudflare.com` its site, switched on for `webrtc`. Ask for it
with `vault_request`; never in the chat. Without it, a live view falls back to screenshots and says why.
