#!/usr/bin/env python3
"""A minimal fake CasparCG server, for developing caspar-AV without one.

It speaks enough real AMCP to exercise the whole bridge:

* the response framing that actually matters — `200` runs to a blank line,
  `201` takes exactly one line, `202` carries none;
* `REQ <id>` / `RES <id>` correlation. It answers in order; a real server does
  not always, because it dispatches to one queue per channel. The out-of-order
  case is covered by the client's own tests in `crates/amcp/tests/client.rs`,
  which can force it deterministically — a fixture that reordered at random
  would only find the bug sometimes.
* `BEGIN` / `COMMIT` batching, answered the way 2.5.0 answers it: every inner
  command replies, then one `202 COMMIT OK`;
* `OSC SUBSCRIBE <port>`, after which it pushes a per-frame OSC bundle of
  playback state to that port, exactly as the server does.

It also stands in for **media-scanner** on port 8000, so the Media and Templates
pages have something to show. CasparCG 2.5 has no media listing of its own —
without a scanner those pages are empty no matter what the server is doing.

It is a test fixture, not an emulator: it does not decode media or render
anything, and every command it does not recognise is answered `202 ... OK`.

    python3 scripts/fake-caspar.py [--port 5250] [--scanner-port 8000]
    python3 scripts/fake-caspar.py --scanner-port 0     # AMCP only
"""

import argparse
import http.server
import json
import socket
import socketserver
import struct
import sys
import threading
import time
import urllib.parse
import zlib

VERSION = "2.5.0.0 STABLE"
CHANNELS = {1: "1080p5000", 2: "1080p5000"}

# What the fake channels are "playing", so the console has something to show.
# The layers match the screens in demo-show.json, so the Channels page lines up
# with the show rather than describing some other rig.
CLIPS = {
    (1, 10): ("WALK_IN/LOGO_LOOP", 12.0),
    (1, 20): ("KEYNOTE/SLIDE_BG", 60.0),
    (2, 10): ("WALK_IN/LOGO_LOOP", 12.0),
    (2, 30): ("LOOPS/PARTICLE_FIELD", 45.0),
}


def osc_string(value: str) -> bytes:
    data = value.encode("utf-8") + b"\0"
    return data + b"\0" * (-len(data) % 4)


def osc_message(address: str, *args) -> bytes:
    tags = ","
    payload = b""
    for arg in args:
        if isinstance(arg, bool):
            tags += "T" if arg else "F"
        elif isinstance(arg, int):
            tags += "i"
            payload += struct.pack(">i", arg)
        elif isinstance(arg, float):
            tags += "f"
            payload += struct.pack(">f", arg)
        else:
            tags += "s"
            payload += osc_string(str(arg))
    return osc_string(address) + osc_string(tags) + payload


def osc_bundle(messages) -> bytes:
    out = b"#bundle\0" + struct.pack(">Q", 1)
    for m in messages:
        out += struct.pack(">i", len(m)) + m
    return out


class Telemetry(threading.Thread):
    """Push OSC state to every subscriber, once per 'frame'."""

    daemon = True

    def __init__(self):
        super().__init__()
        self.subscribers: set[tuple[str, int]] = set()
        self.lock = threading.Lock()
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.start_time = time.time()

    def subscribe(self, host: str, port: int) -> None:
        with self.lock:
            self.subscribers.add((host, port))
        print(f"  → OSC telemetry to {host}:{port}")

    def unsubscribe(self, host: str, port: int) -> None:
        with self.lock:
            self.subscribers.discard((host, port))

    def run(self) -> None:
        while True:
            time.sleep(0.04)  # 25 fps is plenty for a fixture
            with self.lock:
                targets = list(self.subscribers)
            if not targets:
                continue

            elapsed = time.time() - self.start_time
            messages = []
            for index, mode in CHANNELS.items():
                base = f"/channel/{index}"
                messages.append(osc_message(f"{base}/format", mode))
                # A rational, not a float — this is what a real 2.5.0 sends, and
                # reading it as a scalar yields nothing at all.
                messages.append(osc_message(f"{base}/framerate", 50, 1))

            for (channel, layer), (name, duration) in CLIPS.items():
                fg = f"/channel/{channel}/stage/layer/{layer}/foreground"
                position = elapsed % duration
                # Exactly the key set a real server publishes for the ffmpeg
                # producer, captured from 2.5.0. Note what is absent: there is
                # no file/frame, no file/fps and no file/video/* on the producer
                # side, and no profiler/time anywhere. Emitting those here would
                # let a client that reads them look correct against this fixture
                # and show nothing against a real server — which is exactly the
                # bug this fixture failed to catch the first time round.
                messages += [
                    osc_message(f"{fg}/producer", "ffmpeg"),
                    osc_message(f"{fg}/paused", False),
                    osc_message(f"{fg}/loop", True),
                    osc_message(f"{fg}/file/path", f"/opt/caspar/media/{name}.mp4"),
                    osc_message(f"{fg}/file/name", name),
                    osc_message(f"{fg}/file/time", float(position), float(duration)),
                    osc_message(f"{fg}/file/clip", 0.0, float(duration)),
                    osc_message(f"{fg}/file/streams/0/fps", 50, 1),
                    osc_message(f"/channel/{channel}/stage/layer/{layer}/background/producer",
                                "empty"),
                ]

            packet = osc_bundle(messages)
            for target in targets:
                try:
                    self.socket.sendto(packet, target)
                except OSError:
                    pass


TELEMETRY = Telemetry()


class Handler(socketserver.StreamRequestHandler):
    def handle(self) -> None:
        peer = self.client_address[0]
        print(f"client connected from {peer}")
        self.batch: list[tuple[str, str]] | None = None
        self.batch_id = ""
        self.subscribed: list[int] = []

        try:
            for raw in self.rfile:
                line = raw.decode("utf-8", "replace").strip()
                if not line:
                    continue
                print(f"  ← {line}")
                for reply in self.dispatch(line):
                    print(f"  → {reply.strip()}")
                    self.wfile.write(reply.encode("utf-8"))
                    self.wfile.flush()
        except (ConnectionResetError, BrokenPipeError):
            pass
        finally:
            for port in self.subscribed:
                TELEMETRY.unsubscribe(peer, port)
            print(f"client {peer} gone")

    def dispatch(self, line: str):
        request_id = ""
        tokens = line.split()
        if tokens and tokens[0].upper() == "REQ":
            request_id = tokens[1]
            tokens = tokens[2:]
        if not tokens:
            return []

        verb = tokens[0].upper()

        # ---- batching -------------------------------------------------
        if verb == "BEGIN":
            self.batch = []
            self.batch_id = request_id
            return []  # BEGIN is never answered
        if verb == "DISCARD":
            self.batch = None
            return []
        if verb == "COMMIT":
            queued, self.batch = self.batch or [], None
            replies = [self.respond(rid, f"202 {v} OK") for rid, v in queued]
            replies.append(self.respond(self.batch_id, "202 COMMIT OK"))
            return replies
        if self.batch is not None:
            self.batch.append((request_id, verb))
            return []

        # ---- queries --------------------------------------------------
        if verb == "VERSION":
            return [self.respond(request_id, "201 VERSION OK", [VERSION])]
        if verb == "INFO" and len(tokens) > 1 and tokens[1].upper() == "PATHS":
            return [
                self.respond(
                    request_id,
                    "201 INFO PATHS OK",
                    ["<paths><media-path>media/</media-path></paths>"],
                )
            ]
        if verb == "INFO" and len(tokens) == 1:
            body = [f"{i} {mode} PLAYING" for i, mode in CHANNELS.items()]
            return [self.respond(request_id, "200 INFO OK", body, multiline=True)]
        if verb == "CLS":
            return [self.respond(request_id, "501 CLS FAILED")]
        if verb == "OSC" and len(tokens) > 2 and tokens[1].upper() == "SUBSCRIBE":
            port = int(tokens[2])
            self.subscribed.append(port)
            TELEMETRY.subscribe(self.client_address[0], port)
            return [self.respond(request_id, "202 OSC SUBSCRIBE OK")]

        return [self.respond(request_id, f"202 {verb} OK")]

    @staticmethod
    def respond(request_id: str, status: str, body=None, multiline: bool = False) -> str:
        prefix = f"RES {request_id} " if request_id else ""
        out = f"{prefix}{status}\r\n"
        for line in body or []:
            out += f"{line}\r\n"
        if multiline:
            out += "\r\n"  # a 200 runs until a blank line
        return out


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


# ------------------------------------------------------------ media-scanner

# A plausible live-event media set. The ids are what AMCP uses: the path under
# the media root, upper-cased, without its extension.
MEDIA = [
    ("OPENER", 24.0, 1920, 1080),
    ("WALK_IN/LOGO_LOOP", 12.0, 1920, 1080),
    ("LOOPS/AMBIENT_BLUE", 30.0, 3840, 1080),
    ("LOOPS/PARTICLE_FIELD", 45.0, 3840, 1080),
    ("KEYNOTE/SLIDE_BG", 60.0, 1920, 1080),
    ("BREAK/COUNTDOWN_5MIN", 300.0, 1920, 1080),
    ("STINGS/WIPE_LEFT", 1.2, 1920, 1080),
    ("STINGS/GLITCH", 0.8, 1920, 1080),
]
STILLS = [("LOWER_THIRDS/BUG", 1920, 1080), ("HOLDING/SPONSORS", 3840, 1080)]
AUDIO = [("AUDIO/BED_AMBIENT", 180.0), ("AUDIO/STING_HIT", 2.0)]

# One template publishes a GDD schema and one does not, because that is the
# split the console has to cope with: a generated form, or a JSON box.
TEMPLATES = [
    {
        "id": "LOWER_THIRDS/NAME",
        "path": "/opt/caspar/template/LOWER_THIRDS/NAME.html",
        "type": "html",
        "gdd": {
            "type": "object",
            "title": "Lower third",
            "properties": {
                "f0": {"type": "string", "title": "Name", "description": "Displayed large"},
                "f1": {"type": "string", "title": "Role", "description": "Displayed beneath"},
                "f2": {"type": "string", "title": "Organisation"},
                "theme": {
                    "type": "string",
                    "title": "Theme",
                    "enum": ["light", "dark", "accent"],
                    "default": "dark",
                },
            },
        },
    },
    {
        "id": "COUNTDOWN/CLOCK",
        "path": "/opt/caspar/template/COUNTDOWN/CLOCK.html",
        "type": "html",
        "gdd": {
            "type": "object",
            "properties": {
                "minutes": {"type": "string", "title": "Minutes", "default": "5"},
                "label": {"type": "string", "title": "Caption", "default": "Back in"},
            },
        },
    },
    {"id": "BUGS/CORNER", "path": "/opt/caspar/template/BUGS/CORNER.ft", "type": "ft", "gdd": None},
]

FONTS = ["ARIAL", "HELVETICA-NEUE", "ROBOTO-CONDENSED"]


def png(width: int, height: int, pixel) -> bytes:
    """Encode an RGB image. Hand-rolled to keep this script dependency-free."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter: none
        for x in range(width):
            raw.extend(pixel(x, y))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
        + chunk(b"IEND", b"")
    )


def thumbnail(name: str) -> bytes:
    """A distinct, stable thumbnail per media id.

    Derived from the name so a given clip always looks the same — a real
    thumbnail is a frame of the file, and the point here is only that tiles are
    visually distinguishable.
    """
    seed = sum((i + 1) * ord(c) for i, c in enumerate(name))
    base = (seed % 360) / 360.0

    def hsv(h, s, v):
        i = int(h * 6) % 6
        f = h * 6 - int(h * 6)
        p, q, t = v * (1 - s), v * (1 - f * s), v * (1 - (1 - f) * s)
        r, g, b = [(v, t, p), (q, v, p), (p, v, t), (p, q, v), (t, p, v), (v, p, q)][i]
        return int(r * 255), int(g * 255), int(b * 255)

    w, h = 256, 144

    def pixel(x, y):
        # A diagonal gradient with a band, so tiles read as images not swatches.
        d = (x / w + y / h) / 2
        hue = (base + d * 0.12) % 1.0
        value = 0.30 + 0.45 * d
        if abs(y - h * (0.45 + 0.12 * (x / w))) < 6:
            value = min(1.0, value + 0.35)
        return hsv(hue, 0.55, value)

    return png(w, h, pixel)


def media_json():
    out = []
    for name, duration, width, height in MEDIA:
        out.append(
            {
                "name": name,
                "path": f"/opt/caspar/media/{name}.mp4",
                "mediaSize": int(duration * 1_400_000),
                "mediaTime": 1785000000,
                "format": {"name": "mov,mp4", "duration": f"{duration}"},
                "streams": [
                    {
                        "codec": {"type": "video", "name": "h264"},
                        "width": width,
                        "height": height,
                        "duration": f"{duration}",
                    },
                    {"codec": {"type": "audio", "name": "aac"}, "channels": 2},
                ],
            }
        )
    for name, width, height in STILLS:
        out.append(
            {
                "name": name,
                "path": f"/opt/caspar/media/{name}.png",
                "mediaSize": 480_000,
                "format": {"name": "png_pipe", "duration": "0.04"},
                "streams": [
                    {"codec": {"type": "video", "name": "png"}, "width": width, "height": height}
                ],
            }
        )
    for name, duration in AUDIO:
        out.append(
            {
                "name": name,
                "path": f"/opt/caspar/media/{name}.wav",
                "mediaSize": int(duration * 176_000),
                "format": {"name": "wav", "duration": f"{duration}"},
                "streams": [
                    {"codec": {"type": "audio", "name": "pcm_s16le"}, "channels": 2,
                     "duration": f"{duration}"}
                ],
            }
        )
    return out


class ScannerHandler(http.server.BaseHTTPRequestHandler):
    """The media-scanner routes caspar-AV actually uses."""

    def log_message(self, *args):  # quieter than the default
        pass

    def _send(self, body: bytes, content_type: str, status: int = 200) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802 — name fixed by BaseHTTPRequestHandler
        path = urllib.parse.unquote(self.path.split("?")[0])

        if path == "/media":
            self._send(json.dumps(media_json()).encode(), "application/json")
        elif path.startswith("/media/thumbnail/"):
            self._send(thumbnail(path[len("/media/thumbnail/") :]), "image/png")
        elif path.startswith("/media/info/"):
            wanted = path[len("/media/info/") :].upper()
            item = next((m for m in media_json() if m["name"] == wanted), {})
            self._send(json.dumps(item).encode(), "application/json")
        elif path == "/templates":
            self._send(json.dumps({"templates": TEMPLATES}).encode(), "application/json")
        elif path == "/fls":
            body = "200 FLS OK\r\n" + "".join(f'"{f}"\r\n' for f in FONTS) + "\r\n"
            self._send(body.encode(), "text/plain")
        elif path in ("/cls", "/tls"):
            self._send(f"200 {path[1:].upper()} OK\r\n\r\n".encode(), "text/plain")
        else:
            self._send(b"", "text/plain", 404)


class ScannerServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=5250, help="AMCP port")
    parser.add_argument(
        "--scanner-port", type=int, default=8000, help="media-scanner port; 0 to disable"
    )
    args = parser.parse_args()

    # The AMCP transcript is the whole point of running this thing; block
    # buffering hides it whenever stdout is a file or a pipe.
    sys.stdout.reconfigure(line_buffering=True)

    TELEMETRY.start()

    if args.scanner_port:
        scanner = ScannerServer(("0.0.0.0", args.scanner_port), ScannerHandler)
        threading.Thread(target=scanner.serve_forever, daemon=True).start()
        print(f"fake media-scanner listening on port {args.scanner_port}")

    print(f"fake CasparCG {VERSION} listening on AMCP port {args.port}")
    with Server(("0.0.0.0", args.port), Handler) as server:
        server.serve_forever()


if __name__ == "__main__":
    main()
