#!/usr/bin/python3
# `webrtc-stream <file>`: the display as live video to one viewer, over WebRTC (GStreamer's webrtcbin), with the viewer's
# mouse and keys back over its data channel, played on the display through XTest. The file is JSON, `{"offer": <the
# viewer's SDP offer>, "iceServers": [<as RTCPeerConnection takes them>]}`, and is removed once read. It prints the
# SDP answer, with every candidate this end gathered, and carries on in a process of its own until the viewer goes,
# or the display is resized, which the viewer then connects again for. What it says goes to /tmp/webrtc-stream.log.
#
# Both ends reach each other only through the TURN servers given (`relay`), so neither learns the other's addresses.
#
# The viewer's data channel carries JSON: `{"t": "move", "x", "y"}`; `{"t": "button", "b": 1, "down": true}`, X's
# buttons, 1 left, 2 middle, 3 right and 4 to 7 the wheel; `{"t": "key", "key": "Return"}`, a key as X names it or
# a character, pressed and let go; and `{"t": "text", "text"}`, typed. A key or a button pressed says the modifiers
# held with it, as X names them (`"mods": ["Control_L"]`), and one let go the same.
import json
import os
import re
import socket
import subprocess
import sys
from urllib.parse import quote, urlsplit

with open(sys.argv[1]) as file:
    asked = json.load(file)
os.remove(sys.argv[1])

# The answer comes back up a pipe, and the stream carries on in a process of its own once it has, so the command
# that started it ends.
reading, writing = os.pipe()
if os.fork():
    os.close(writing)
    said = b""
    while chunk := os.read(reading, 65536):
        said += chunk
    said = json.loads(said or b'{"error": "it stopped before it answered: /tmp/webrtc-stream.log says why"}')
    print(said.get("answer") or said["error"], end="", file=sys.stdout if "answer" in said else sys.stderr)
    sys.exit(0 if "answer" in said else 1)

os.close(reading)
os.setsid()
log = os.open("/tmp/webrtc-stream.log", os.O_WRONLY | os.O_CREAT | os.O_APPEND)
os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
os.dup2(log, 1)
os.dup2(log, 2)

import gi  # noqa: E402

gi.require_version("Gst", "1.0")
gi.require_version("GstSdp", "1.0")
gi.require_version("GstWebRTC", "1.0")
from gi.repository import GLib, Gst, GstSdp, GstWebRTC  # noqa: E402
from Xlib import X, XK, display  # noqa: E402
from Xlib.ext import randr, xtest  # noqa: E402

Gst.init(None)
loop = GLib.MainLoop()


def answer(said):
    global writing
    if writing is not None:
        os.write(writing, json.dumps(said).encode())
        os.close(writing)
        writing = None


def stop(why):
    print(why, flush=True)
    answer({"error": why})
    loop.quit()


# The H.264 the viewer takes in its offer: one packet a NAL unit or more (packetization-mode 1), constrained baseline,
# which every browser decodes.
def h264(sdp):
    rtpmaps = [line.split()[0].split(":")[1] for line in sdp.splitlines() if line.startswith("a=rtpmap:") and " H264/90000" in line]
    fmtp = {line.split()[0].split(":")[1]: line.split(" ", 1)[1] for line in sdp.splitlines() if line.startswith("a=fmtp:")}
    fits = [pt for pt in rtpmaps if "packetization-mode=1" in fmtp.get(pt, "")]
    best = [pt for pt in fits if "profile-level-id=42e0" in fmtp[pt]]
    if not fits:
        raise RuntimeError("the viewer doesn't take H.264")
    return int((best or fits)[0])


screen = display.Display()
root = screen.screen().root
sdp = asked["offer"]
pt = h264(sdp)
# H.264 takes only an even width and height, so a display of an odd one leaves out its last column or row.
size = root.get_geometry()
pipeline = Gst.parse_launch(
    "webrtcbin name=rtc bundle-policy=max-bundle latency=0 "
    f"ximagesrc use-damage=false show-pointer=true endx={size.width - 1 - size.width % 2} endy={size.height - 1 - size.height % 2} ! video/x-raw,framerate=30/1 ! queue max-size-buffers=1 leaky=downstream "
    "! videoconvert n-threads=2 ! x264enc tune=zerolatency speed-preset=ultrafast bitrate=6000 key-int-max=300 "
    "! video/x-h264,profile=constrained-baseline ! rtph264pay config-interval=-1 aggregate-mode=zero-latency "
    f"! application/x-rtp,media=video,encoding-name=H264,clock-rate=90000,payload={pt} ! rtc."
)
rtc = pipeline.get_by_name("rtc")
rtc.set_property("ice-transport-policy", GstWebRTC.WebRTCICETransportPolicy.RELAY)
for server in asked["iceServers"]:
    for url in [server["urls"]] if isinstance(server["urls"], str) else server["urls"]:
        # TURN over UDP and TCP, not TLS: libnice gets no allocation from Cloudflare's over TLS, and one that fails
        # can stop the others being gathered. Both, since some flows over UDP to Cloudflare's address are dropped
        # all the time, by their port (seen from one machine here), and TCP is another way there.
        scheme, rest = url.split(":", 1)
        if scheme == "turn":
            at = urlsplit(f"{scheme}://{rest}")
            user = f"{quote(server['username'], safe='')}:{quote(server['credential'], safe='')}"
            # By its IPv4 address: a container has no route over IPv6, and libnice would wait two seconds on one.
            host = f"{socket.gethostbyname(at.hostname)}:{at.port}"
            rtc.emit("add-turn-server", f"{scheme}://{user}@{host}" + (f"?{at.query}" if at.query else ""))

transceiver = rtc.emit("get-transceiver", 0)
transceiver.set_property("direction", GstWebRTC.WebRTCRTPTransceiverDirection.SENDONLY)
# A packet lost on the way is sent again when the viewer asks (NACK), rather than the picture waiting for a keyframe.
transceiver.set_property("do-nack", True)
transceiver.set_property(
    "codec-preferences",
    Gst.Caps.from_string(f"application/x-rtp,media=video,encoding-name=H264,clock-rate=90000,payload={pt},packetization-mode=(string)1"),
)


# This end's candidates, as they're gathered. webrtcbin's local description can be without the last of them yet as
# it says gathering is complete, so the answer has them put in: all in the first media section, which the bundle
# shares. It goes a moment after the first, through TURN, rather than waiting on every TURN server.
candidates = []


def candidate(_rtc, _mline, line):
    if " typ relay" not in line:
        return
    candidates.append(line)
    if len(candidates) == 1:
        GLib.timeout_add(250, reply)


def reply():
    description = rtc.get_property("local-description")
    if description is None or not candidates:
        return False
    session, first, *rest = re.split(r"(?m)^(?=m=)", description.sdp.as_text())
    first += "".join(f"a={line}\r\n" for line in candidates if f"a={line}\r\n" not in first)
    answer({"answer": session + first + "".join(rest)})
    return False


def gathered(*_):
    if rtc.get_property("ice-gathering-state") == GstWebRTC.WebRTCICEGatheringState.COMPLETE:
        GLib.idle_add(reply)


def answered(promise, _):
    reply = promise.get_reply()
    if reply is None or reply.get_value("answer") is None:
        return GLib.idle_add(stop, f"no answer: {reply}")
    rtc.emit("set-local-description", reply.get_value("answer"), Gst.Promise.new())


def offered(promise, _):
    rtc.emit("create-answer", None, Gst.Promise.new_with_change_func(answered, None))


rtc.connect("on-ice-candidate", candidate)
rtc.connect("notify::ice-gathering-state", gathered)

# The viewer is gone once the connection is, or never came.
connected = False


def state(*_):
    global connected
    now = rtc.get_property("connection-state")
    if now == GstWebRTC.WebRTCPeerConnectionState.CONNECTED:
        connected = True
    elif now in (GstWebRTC.WebRTCPeerConnectionState.FAILED, GstWebRTC.WebRTCPeerConnectionState.CLOSED):
        GLib.idle_add(stop, f"the connection {now.value_nick}")


rtc.connect("notify::connection-state", state)
GLib.timeout_add_seconds(30, lambda: connected or stop("the viewer never connected") and False)

def keysym(name):
    if len(name) == 1:
        return ord(name) if ord(name) < 0x100 else 0x1000000 | ord(name)
    return XK.string_to_keysym(name)


def hold(names, down):
    for name in names if down else reversed(names):
        xtest.fake_input(screen, X.KeyPress if down else X.KeyRelease, screen.keysym_to_keycode(keysym(name)))


def press(name, held=()):
    """Presses the key `name` types, with the modifiers `held`, and Shift where the key's character takes it."""
    sym = keysym(name)
    code = screen.keysym_to_keycode(sym) if sym else 0
    if not code:
        # A character the keyboard has no key for, which xdotool types by mapping a key to it for a moment.
        return subprocess.run(["xdotool", "type", "--", name] if len(name) == 1 else ["xdotool", "key", "--", name])
    held = [*held, "Shift_L"] if screen.keycode_to_keysym(code, 0) != sym and "Shift_L" not in held else held
    hold(held, True)
    xtest.fake_input(screen, X.KeyPress, code)
    xtest.fake_input(screen, X.KeyRelease, code)
    hold(held, False)


def act(message):
    m = json.loads(message)
    if m["t"] == "move":
        xtest.fake_input(screen, X.MotionNotify, x=int(m["x"]), y=int(m["y"]))
    elif m["t"] == "button" and m["down"]:
        hold(m.get("mods", []), True)
        xtest.fake_input(screen, X.ButtonPress, int(m["b"]))
    elif m["t"] == "button":
        xtest.fake_input(screen, X.ButtonRelease, int(m["b"]))
        hold(m.get("mods", []), False)
    elif m["t"] == "key":
        press(m["key"], m.get("mods", []))
    elif m["t"] == "text":
        for character in m["text"]:
            press("Return" if character == "\n" else character)
    screen.flush()


# The data channel is one both ends make, as id 0 (`negotiated`), which opens with the connection.

# A display that's resized is a stream of another size: the viewer connects again for it.
root.xrandr_select_input(randr.RRScreenChangeNotifyMask)


def events(*_):
    while screen.pending_events():
        if screen.next_event().type == screen.extension_event.ScreenChangeNotify:
            stop("the display was resized")
    return True


GLib.io_add_watch(screen.fileno(), GLib.IO_IN, events)


def bus(_bus, message):
    if message.type == Gst.MessageType.ERROR:
        error, debug = message.parse_error()
        print(debug, flush=True)
        stop(error.message)


pipeline.get_bus().add_signal_watch()
pipeline.get_bus().connect("message", bus)
pipeline.set_state(Gst.State.PLAYING)
_, parsed = GstSdp.SDPMessage.new_from_text(sdp)
offer = GstWebRTC.WebRTCSessionDescription.new(GstWebRTC.WebRTCSDPType.OFFER, parsed)
rtc.emit("set-remote-description", offer, Gst.Promise.new_with_change_func(offered, None))
data = rtc.emit("create-data-channel", "input", Gst.Structure.new_from_string("config, negotiated=(boolean)true, id=(int)0"))
# What the viewer sends comes on the data channel's own thread, and is acted on here, in order.
data.connect("on-message-string", lambda _, message: GLib.idle_add(act, message) and None)
data.connect("on-close", lambda _: GLib.idle_add(stop, "the viewer closed its data channel"))
GLib.timeout_add_seconds(4, lambda: writing is not None and stop("it couldn't reach TURN within 4 seconds") and False)
loop.run()
pipeline.set_state(Gst.State.NULL)
