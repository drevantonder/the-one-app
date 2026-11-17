# `python stream.py <file>`: a simulator's screen as live video to one viewer, over WebRTC (aiortc), with the viewer's
# mouse and keys back over its data channel. The video is the H.264 that idb (fbidb.io) encodes from the simulator's
# own display, sent on as it comes, never decoded; what the viewer does is played on the simulator. Both go through
# idb's client in this process, not its command: starting the command for each tap took two or three times as long
# as the tap, and the command stopped by a signal leaves its video encoding in idb's companion for good.
#
# The file is JSON, `{"offer": <the viewer's SDP offer>, "iceServers": [<as RTCPeerConnection takes them>],
# "companion": <idb's companion>, "udid": <the simulator's>, "points": <its points per pixel>, "log": <a file>}`, and
# is removed once read. It prints the SDP answer and carries on in a process of its own until the viewer goes. Both
# ends reach each other only through TURN (`relay`), and the answer carries only relay candidates, so neither learns
# the other's addresses.
#
# The data channel (id 0, negotiated) carries what the UI library's LiveScreen sends: `{"t": "move", "x", "y"}` in the
# video's pixels; `{"t": "button", "b", "down", "mods"}`, X's buttons: 1 is a finger, which taps, or swipes if it moved,
# 2 Home, 3 a long press, 4 to 7 the wheel; `{"t": "key", "key", "mods"}`, a key as X names it; and `{"t": "text"}`.
import json
import os
import re
import socket
import sys
import time

with open(sys.argv[1]) as file:
    config = json.load(file)
os.remove(sys.argv[1])

# The answer comes back up a pipe, and the stream carries on in a process of its own once it has.
reading, writing = os.pipe()
if os.fork():
    os.close(writing)
    response = b""
    while chunk := os.read(reading, 65536):
        response += chunk
    response = json.loads(response or b'{"error": "it stopped before it answered"}')
    print(response.get("answer") or response["error"], end="", file=sys.stdout if "answer" in response else sys.stderr)
    sys.exit(0 if "answer" in response else 1)

os.close(reading)
os.setsid()
log = os.open(config["log"], os.O_WRONLY | os.O_CREAT | os.O_APPEND)
os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
os.dup2(log, 1)
os.dup2(log, 2)

import asyncio  # noqa: E402
import logging  # noqa: E402
from fractions import Fraction  # noqa: E402

import av  # noqa: E402
from aiortc import RTCConfiguration, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnection, RTCRtpSender, RTCSessionDescription  # noqa: E402
from aiortc.mediastreams import MediaStreamTrack  # noqa: E402
from idb.common.types import HIDButtonType, VideoFormat  # noqa: E402
from idb.grpc.management import ClientManager  # noqa: E402

points = config["points"]

# The USB HID usage of each key X names, which is how idb presses one.
KEYS = {"Return": 40, "Escape": 41, "BackSpace": 42, "Tab": 43, "space": 44, "Delete": 76, "Right": 79, "Left": 80, "Down": 81, "Up": 82, "Home": 74, "End": 77, "Prior": 75, "Next": 78}
KEYS.update({chr(c): 4 + c - ord("a") for c in range(ord("a"), ord("z") + 1)})
KEYS.update({str(d): 30 + d - 1 for d in range(1, 10)})
KEYS["0"] = 39


def answer(response):
    global writing
    if writing is not None:
        os.write(writing, json.dumps(response).encode())
        os.close(writing)
        writing = None


class Screen(MediaStreamTrack):
    """The simulator's H.264, an access unit at a time, as idb's encoder makes them."""

    kind = "video"

    def __init__(self):
        super().__init__()
        self.packets = asyncio.Queue()

    async def recv(self):
        return await self.packets.get()


async def main(client):
    screen = Screen()
    done = asyncio.Event()

    async def frames():
        """idb's Annex B H.264, cut into access units by FFmpeg's parser, each timed as it arrives."""
        parser = av.CodecContext.create("h264", "r")
        start = time.monotonic()
        async for data in client.stream_video(output_file=None, fps=30, format=VideoFormat.H264, compression_quality=0.2):
            for packet in parser.parse(data):
                packet.pts = int((time.monotonic() - start) * 1_000_000)
                packet.time_base = Fraction(1, 1_000_000)
                screen.packets.put_nowait(packet)

    # What's done on the screen is played by idb's client, one after another, in the order it came: each is a
    # function of the client.
    acts = asyncio.Queue()

    async def play():
        while True:
            next_act = await acts.get()
            # One idb can't play, as text with a character it has no key for, is told to the log, and the rest are
            # played: the viewer has no way to hear of it.
            try:
                await next_act(client)
            except Exception as error:
                print(f"{asked['udid']}: {error!r}", flush=True)

    def at(x, y):
        return round(x * points), round(y * points)

    pointer = {"at": (0, 0), "down": None}

    def act(message):
        m = json.loads(message)
        if m["t"] == "move":
            pointer["at"] = (m["x"], m["y"])
        elif m["t"] == "button" and m["b"] in (1, 3) and m["down"]:
            pointer["down"] = (pointer["at"], time.monotonic(), m["b"])
        elif m["t"] == "button" and m["b"] in (1, 3) and pointer["down"]:
            (x0, y0), since, b = pointer["down"]
            pointer["down"] = None
            x1, y1 = pointer["at"]
            held = time.monotonic() - since
            if abs(x1 - x0) + abs(y1 - y0) > 12:
                acts.put_nowait(lambda c: c.swipe(at(x0, y0), at(x1, y1), duration=max(held, 0.1)))
            else:
                acts.put_nowait(lambda c: c.tap(*at(x1, y1), duration=1 if b == 3 or held > 0.5 else None))
        elif m["t"] == "button" and m["b"] == 2 and m["down"]:
            acts.put_nowait(lambda c: c.button(HIDButtonType.HOME))
        elif m["t"] == "button" and m["b"] in (4, 5, 6, 7) and m["down"]:
            # A step of the wheel is a swipe of 60 points the other way, as a finger scrolls.
            x, y = pointer["at"]
            dx, dy = {4: (0, 60), 5: (0, -60), 6: (60, 0), 7: (-60, 0)}[m["b"]]
            acts.put_nowait(lambda c: c.swipe(at(x, y), at(x + dx / points, y + dy / points), duration=0.1))
        elif m["t"] == "key":
            code = KEYS.get(m["key"]) or KEYS.get(m["key"].lower())
            if code:
                acts.put_nowait(lambda c: c.key(code))
        elif m["t"] == "text":
            acts.put_nowait(lambda c: c.text(m["text"]))

    # TURN over TCP, by its IPv4 address: aiortc takes one TURN server, half the allocations over UDP to Cloudflare's
    # get no answer from some machines, by the port they're from, and one over IPv6 can go nowhere, where TCP over
    # IPv4 always answers.
    tcp = [
        (re.sub(r"^turn:([^:?]+)", lambda host: "turn:" + socket.gethostbyname(host[1]), url), s)
        for s in config["iceServers"]
        for url in ([s["urls"]] if isinstance(s["urls"], str) else s["urls"])
        if url.startswith("turn:") and url.endswith("transport=tcp")
    ]
    pc = RTCPeerConnection(RTCConfiguration(
        iceServers=[RTCIceServer(urls=url, username=s.get("username"), credential=s.get("credential")) for url, s in tcp],
        iceTransportPolicy=RTCIceTransportPolicy.RELAY,
    ))
    channel = pc.createDataChannel("input", negotiated=True, id=0)
    channel.on("message", act)
    channel.on("close", done.set)

    @pc.on("connectionstatechange")
    def state():
        print(f"{asked['udid']}: the connection is {pc.connectionState}", flush=True)
        if pc.connectionState in ("failed", "closed"):
            done.set()

    async def never_connected():
        await asyncio.sleep(30)
        if pc.connectionState != "connected":
            done.set()

    # H.264 only, as idb sends it: aiortc chooses the codecs as the offer is taken, from the transceiver's.
    transceiver = pc.addTransceiver(screen, direction="sendonly")
    transceiver.setCodecPreferences([c for c in RTCRtpSender.getCapabilities("video").codecs if c.mimeType in ("video/H264", "video/rtx")])
    await pc.setRemoteDescription(RTCSessionDescription(sdp=config["offer"], type="offer"))
    await pc.setLocalDescription(await pc.createAnswer())
    sdp = "".join(line + "\r\n" for line in pc.localDescription.sdp.splitlines() if not line.startswith("a=candidate:") or " typ relay" in line)
    if " typ relay" not in sdp:
        raise RuntimeError("it couldn't reach TURN")
    answer({"answer": sdp})

    tasks = [asyncio.ensure_future(t) for t in (frames(), play(), never_connected())]
    try:
        # It ends when the video or the playing of input does, or the viewer goes; one that broke says why.
        ended, _ = await asyncio.wait([*tasks[:2], asyncio.ensure_future(done.wait())], return_when=asyncio.FIRST_COMPLETED)
        for task in ended:
            task.result()
    finally:
        # Waited for, as the video's own ending is what tells idb's companion to stop encoding it.
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        await pc.close()


async def streaming():
    manager = ClientManager(companion_path=config["companion"], logger=logging.getLogger("idb"))
    async with manager.from_udid(config["udid"]) as client:
        await main(client)


try:
    asyncio.run(streaming())
except Exception as error:
    print(repr(error), flush=True)
    answer({"error": f"{type(error).__name__}: {error}"})
