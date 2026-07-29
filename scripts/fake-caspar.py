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

It is a test fixture, not an emulator: it does not decode media or render
anything, and every command it does not recognise is answered `202 ... OK`.

    python3 scripts/fake-caspar.py [--port 5250]
"""

import argparse
import math
import socket
import socketserver
import struct
import sys
import threading
import time

VERSION = "2.5.0.0 STABLE"
CHANNELS = {1: "1080p5000", 2: "720p5000"}

# What the fake channels are "playing", so the console has something to show.
CLIPS = {
    (1, 10): ("AMB", 30.0),
    (2, 10): ("OPENER", 12.5),
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
                messages.append(osc_message(f"{base}/framerate", 50.0))
                messages.append(
                    osc_message(f"{base}/profiler/time", 0.004 + 0.001 * math.sin(elapsed))
                )

            for (channel, layer), (name, duration) in CLIPS.items():
                fg = f"/channel/{channel}/stage/layer/{layer}/foreground"
                position = elapsed % duration
                messages += [
                    osc_message(f"{fg}/producer", "ffmpeg"),
                    osc_message(f"{fg}/paused", False),
                    osc_message(f"{fg}/loop", True),
                    osc_message(f"{fg}/file/path", f"{name}.mp4"),
                    osc_message(f"{fg}/file/name", name),
                    osc_message(f"{fg}/file/time", float(position), float(duration)),
                    osc_message(
                        f"{fg}/file/frame", float(int(position * 50)), float(int(duration * 50))
                    ),
                    osc_message(f"{fg}/file/fps", 50.0),
                    osc_message(f"{fg}/file/video/width", 1920),
                    osc_message(f"{fg}/file/video/height", 1080),
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=5250)
    args = parser.parse_args()

    # The AMCP transcript is the whole point of running this thing; block
    # buffering hides it whenever stdout is a file or a pipe.
    sys.stdout.reconfigure(line_buffering=True)

    TELEMETRY.start()
    print(f"fake CasparCG {VERSION} listening on AMCP port {args.port}")
    with Server(("0.0.0.0", args.port), Handler) as server:
        server.serve_forever()


if __name__ == "__main__":
    main()
