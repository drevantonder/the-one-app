# `python stream.py <file>`: an emulator's screen as live video to one viewer, over WebRTC (aiortc), with the viewer's
# mouse and keys back over its data channel. The video is the phone's own H.264, from scrcpy's server running in it
# (github.com/Genymobile/scrcpy), sent on as it comes, never decoded; what the viewer does goes to that server's
# control socket, which injects it as touches, keys and text.
#
# The file is JSON, `{"offer": <the viewer's SDP offer>, "iceServers": [<as RTCPeerConnection takes them>], "adb":
# <adb's path>, "serial": "emulator-5560", "version": <scrcpy's>, "log": <a file>}`, and is removed once read. It prints
# the SDP answer and carries on in a process of its own until the viewer goes. Both ends reach each other only through
# TURN (`relay`), and the answer carries only relay candidates, so neither learns the other's addresses.
#
# The data channel (id 0, negotiated) carries what the UI library's LiveScreen sends: `{"t": "move", "x", "y"}` in the
# video's pixels; `{"t": "button", "b", "down", "mods"}`, X's buttons: 1 is a finger, 2 Home, 3 Back, 4 to 7 the wheel;
# `{"t": "key", "key", "mods"}`, a key as X names it; and `{"t": "text", "text"}`.
import json
import os
import re
import secrets
import socket
import struct
import sys

with open(sys.argv[1]) as file:
    config = json.load(file)
os.remove(sys.argv[1])

# The answer comes back up a pipe, and the stream carries on in a process of its own once it has.
reading, writing = os.pipe()
if os.fork():
    os.close(writing)
    said = b""
    while chunk := os.read(reading, 65536):
        said += chunk
    said = json.loads(said or b'{"error": "it stopped before it answered"}')
    print(said.get("answer") or said["error"], end="", file=sys.stdout if "answer" in said else sys.stderr)
    sys.exit(0 if "answer" in said else 1)

os.close(reading)
os.setsid()
log = os.open(config["log"], os.O_WRONLY | os.O_CREAT | os.O_APPEND)
os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
os.dup2(log, 1)
os.dup2(log, 2)

import asyncio  # noqa: E402
from fractions import Fraction  # noqa: E402

import av  # noqa: E402
from aiortc import RTCConfiguration, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnection, RTCRtpSender, RTCSessionDescription  # noqa: E402
from aiortc.mediastreams import MediaStreamTrack  # noqa: E402

adb = [config["adb"], "-s", config["serial"]]

# Android's key codes for the keys X names, and its meta state for X's modifiers.
KEYS = {"Return": 66, "BackSpace": 67, "Tab": 61, "Escape": 4, "Delete": 112, "Left": 21, "Right": 22, "Up": 19, "Down": 20, "Home": 122, "End": 123, "Prior": 92, "Next": 93, "space": 62}
KEYS.update({chr(c): 29 + c - ord("a") for c in range(ord("a"), ord("z") + 1)})
KEYS.update({str(d): 7 + d for d in range(10)})
META = {"Shift_L": 0x41, "Control_L": 0x3000, "Alt_L": 0x12, "Super_L": 0x30000}
FINGER = 0xFFFFFFFFFFFFFFFE  # scrcpy's pointer id for a generic finger, -2


def answer(said):
    global writing
    if writing is not None:
        os.write(writing, json.dumps(said).encode())
        os.close(writing)
        writing = None


class Screen(MediaStreamTrack):
    """The phone's H.264, packet by packet, as scrcpy's server sends it."""

    kind = "video"

    def __init__(self):
        super().__init__()
        self.packets = asyncio.Queue()

    async def recv(self):
        return await self.packets.get()


async def main():
    # scrcpy's server connects to this end through adb (`reverse`): the video's socket, then the control's.
    scid = f"{secrets.randbits(31):08x}"
    sockets = asyncio.Queue()
    listener = await asyncio.start_server(lambda r, w: sockets.put_nowait((r, w)), "127.0.0.1", 0)
    port = listener.sockets[0].getsockname()[1]
    reverse = await asyncio.create_subprocess_exec(*adb, "reverse", f"localabstract:scrcpy_{scid}", f"tcp:{port}")
    await reverse.wait()
    # Its video is at most 1600 pixels tall, H.264's constrained baseline, which every browser decodes, and its
    # encoder is told to make a frame only when the screen changes.
    server = await asyncio.create_subprocess_exec(
        *adb, "shell", "CLASSPATH=/data/local/tmp/scrcpy-server.jar", "app_process", "/", "com.genymobile.scrcpy.Server",
        config["version"], f"scid={scid}", "log_level=warn", "tunnel_forward=false", "audio=false", "control=true",
        "video_codec=h264", "max_size=1600", "max_fps=60", "video_bit_rate=6000000", "video_codec_options=profile=1",
        "send_device_meta=false", "send_dummy_byte=false", "stay_awake=true", "cleanup=true",
    )
    video, _ = await asyncio.wait_for(sockets.get(), 15)
    control_in, control = await asyncio.wait_for(sockets.get(), 15)
    listener.close()
    await (await asyncio.create_subprocess_exec(*adb, "reverse", "--remove", f"localabstract:scrcpy_{scid}")).wait()

    size = [0, 0]
    at = [0, 0]
    screen = Screen()
    done = asyncio.Event()

    async def frames():
        """Packets off the video socket: a session's size, a config (SPS and PPS, put before the next frame), a frame."""
        await video.readexactly(4)  # the codec, h264
        config = b""
        while True:
            head, length = struct.unpack(">QI", await video.readexactly(12))
            if head >> 63:
                # A session's: its flags, width and height, in the same 12 bytes.
                size[:] = [head & 0xFFFFFFFF, length]
                continue
            data = await video.readexactly(length)
            if head >> 62 & 1:
                config = data
                continue
            packet = av.Packet(config + data)
            config = b""
            packet.pts = head & ((1 << 61) - 1)
            packet.time_base = Fraction(1, 1_000_000)
            screen.packets.put_nowait(packet)

    def send(message):
        control.write(message)

    def touch(action):
        send(struct.pack(">BBQiiHHHII", 2, action, FINGER, at[0], at[1], size[0], size[1], 0xFFFF if action != 1 else 0, 0, 0))

    def key(code, mods):
        meta = 0
        for mod in mods:
            meta |= META.get(mod, 0)
        for action in (0, 1):
            send(struct.pack(">BBIII", 0, action, code, 0, meta))

    held = [False]

    def act(message):
        m = json.loads(message)
        if m["t"] == "move":
            at[:] = [int(m["x"]), int(m["y"])]
            if held[0]:
                touch(2)
        elif m["t"] == "button" and m["b"] == 1:
            held[0] = m["down"]
            touch(0 if m["down"] else 1)
        elif m["t"] == "button" and m["b"] == 3:
            send(struct.pack(">BB", 4, 0 if m["down"] else 1))
        elif m["t"] == "button" and m["b"] == 2 and m["down"]:
            key(3, [])
        elif m["t"] == "button" and m["b"] in (4, 5, 6, 7) and m["down"]:
            # One step of the wheel, as scrcpy's own sends it: 1 of its range of 16, up and left positive.
            h, v = {4: (0, 1), 5: (0, -1), 6: (1, 0), 7: (-1, 0)}[m["b"]]
            send(struct.pack(">BiiHHhhI", 3, at[0], at[1], size[0], size[1], h * 2048, v * 2048, 0))
        elif m["t"] == "key":
            name = m["key"]
            code = KEYS.get(name) or KEYS.get(name.lower())
            if code:
                key(code, m.get("mods", []))
        elif m["t"] == "text":
            for i, line in enumerate(m["text"].split("\n")):
                if i:
                    key(66, [])
                if line:
                    data = line.encode()
                    send(struct.pack(">BI", 1, len(data)) + data)

    # TURN over TCP, by its IPv4 address: aiortc takes one TURN server, half the allocations over UDP to Cloudflare's
    # get no answer from a machine here, by the port they're from, and one over IPv6 can go nowhere, where TCP over
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
        print(f"{asked['serial']}: the connection is {pc.connectionState}", flush=True)
        if pc.connectionState in ("failed", "closed"):
            done.set()

    async def never_connected():
        await asyncio.sleep(30)
        if pc.connectionState != "connected":
            done.set()

    # H.264 only, as the phone sends it: aiortc chooses the codecs as the offer is taken, from the transceiver's.
    transceiver = pc.addTransceiver(screen, direction="sendonly")
    transceiver.setCodecPreferences([c for c in RTCRtpSender.getCapabilities("video").codecs if c.mimeType in ("video/H264", "video/rtx")])
    # A viewer that lost a frame asks for a whole one: scrcpy's server starts its encoder again, which begins with one.
    transceiver.sender._send_keyframe = lambda: send(bytes([17]))
    await pc.setRemoteDescription(RTCSessionDescription(sdp=config["offer"], type="offer"))
    await pc.setLocalDescription(await pc.createAnswer())
    sdp = "".join(line + "\r\n" for line in pc.localDescription.sdp.splitlines() if not line.startswith("a=candidate:") or " typ relay" in line)
    if " typ relay" not in sdp:
        raise RuntimeError("it couldn't reach TURN")
    answer({"answer": sdp})

    reader = asyncio.ensure_future(frames())
    waiting = asyncio.ensure_future(never_connected())
    try:
        await asyncio.wait([reader, asyncio.ensure_future(done.wait())], return_when=asyncio.FIRST_COMPLETED)
    finally:
        waiting.cancel()
        reader.cancel()
        await pc.close()
        control.close()
        if server.returncode is None:
            server.terminate()
            await server.wait()


try:
    asyncio.run(main())
except Exception as error:
    print(repr(error), flush=True)
    answer({"error": f"{type(error).__name__}: {error}"})
