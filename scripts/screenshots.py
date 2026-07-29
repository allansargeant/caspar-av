#!/usr/bin/env python3
"""Capture the console's pages for the README.

Point it at a running caspar-avd. With scripts/fake-caspar.py behind that, the
shots show a populated console needing neither CasparCG nor media-scanner:

    python3 scripts/fake-caspar.py &
    cargo run -p showd -- --show demo-show.json &
    python3 scripts/screenshots.py

Headless Chrome rather than an OS screen capture: window-focus races make
`screencapture` unreliable even when it appears to have worked.

Chrome is driven over the DevTools protocol rather than with `--screenshot`,
because that flag only offers two ways to decide *when* to shoot and both are
wrong here. `--virtual-time-budget` waits for the page to go idle, which never
happens while the console holds a live WebSocket, so it hangs forever.
`--timeout` shoots after a fixed delay, which races the first snapshot — and
when it loses, the result is a perfectly valid-looking picture of an
unpopulated console. Three runs in a row produced a different empty page each
time.

So: navigate, poll the DOM until the console has actually connected and
rendered, and only then capture. A page that never becomes ready is reported as
a failure instead of being quietly saved.
"""

import base64
import http.client
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request

PAGES = ["media", "screens", "channels", "cues", "templates", "grid"]

WIDTH, HEIGHT, SCALE = 1440, 780, 2
CHROME = os.environ.get(
    "CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
)

# The console has connected *and* painted: the header shows a connected server
# and the show's name. This is exactly the state the racy approach kept missing.
READY = """
  !!document.querySelector('.server-connected')
  && !!document.querySelector('.topbar-show')
  && document.querySelector('.topbar-show').textContent.trim().length > 0
"""


class WS:
    """The smallest WebSocket client that can carry CDP.

    Written out rather than pulled in: no WebSocket library is installed, and
    this needs exactly two things — send a masked text frame, read a frame that
    may be large (a screenshot is a megabyte of base64) or fragmented.
    """

    def __init__(self, url: str):
        _, rest = url.split("://", 1)
        hostport, path = rest.split("/", 1)
        host, port = hostport.split(":")
        self.sock = socket.create_connection((host, int(port)), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            f"Sec-WebSocket-Version: 13\r\n\r\n".encode()
        )
        buf = b""
        while b"\r\n\r\n" not in buf:
            buf += self.sock.recv(4096)
        if b"101" not in buf.split(b"\r\n", 1)[0]:
            raise RuntimeError(f"websocket upgrade refused: {buf[:120]!r}")
        self.buf = buf.split(b"\r\n\r\n", 1)[1]

    def _read(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("websocket closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, text: str) -> None:
        payload = text.encode()
        header = bytearray([0x81])  # FIN + text
        mask = os.urandom(4)
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)
        elif n < 1 << 16:
            header.append(0x80 | 126)
            header += struct.pack(">H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", n)
        header += mask
        self.sock.sendall(bytes(header) + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def recv(self) -> str:
        out = b""
        while True:
            b0, b1 = self._read(2)
            fin, length = b0 & 0x80, b1 & 0x7F
            if length == 126:
                length = struct.unpack(">H", self._read(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._read(8))[0]
            if b1 & 0x80:  # server frames should never be masked
                self._read(4)
            out += self._read(length)
            if fin:
                return out.decode("utf-8", "replace")

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


class Chrome:
    def __init__(self):
        self.profile = tempfile.mkdtemp(prefix="caspar-shot-")
        self.port = free_port()
        self.proc = subprocess.Popen(
            [
                CHROME, "--headless=new", "--disable-gpu", "--no-first-run",
                "--no-default-browser-check", "--hide-scrollbars",
                f"--user-data-dir={self.profile}",
                f"--remote-debugging-port={self.port}",
                f"--window-size={WIDTH},{HEIGHT}",
                "about:blank",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self.ws = WS(self._target())
        self.msg_id = 0
        self.call("Page.enable")
        self.call(
            "Emulation.setDeviceMetricsOverride",
            {"width": WIDTH, "height": HEIGHT, "deviceScaleFactor": SCALE, "mobile": False},
        )

    def _target(self) -> str:
        deadline = time.time() + 30
        while time.time() < deadline:
            try:
                conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=2)
                conn.request("GET", "/json/list")
                targets = json.loads(conn.getresponse().read())
                for t in targets:
                    if t.get("type") == "page" and t.get("webSocketDebuggerUrl"):
                        return t["webSocketDebuggerUrl"]
            except Exception:
                pass
            time.sleep(0.3)
        raise RuntimeError("Chrome never exposed a debuggable page")

    def call(self, method: str, params=None):
        self.msg_id += 1
        want = self.msg_id
        self.ws.send(json.dumps({"id": want, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(self.ws.recv())
            if msg.get("id") == want:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})
            # Anything else is an event we did not subscribe to caring about.

    def evaluate(self, expression: str):
        r = self.call(
            "Runtime.evaluate", {"expression": expression, "returnByValue": True}
        )
        return r.get("result", {}).get("value")

    def shoot(self, url: str, path: str, timeout: float = 30.0) -> bool:
        self.call("Page.navigate", {"url": url})
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                if self.evaluate(READY) is True:
                    break
            except RuntimeError:
                pass  # navigation in flight; the context is briefly gone
            time.sleep(0.25)
        else:
            return False

        # A beat for the final paint — readiness is about state, not pixels.
        time.sleep(0.6)
        data = self.call("Page.captureScreenshot", {"format": "png"})["data"]
        with open(path, "wb") as f:
            f.write(base64.b64decode(data))
        return True

    def close(self) -> None:
        self.ws.close()
        self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        shutil.rmtree(self.profile, ignore_errors=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def main() -> int:
    url = os.environ.get("URL", "http://localhost:8080")
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = os.path.join(repo, "docs", "assets")
    os.makedirs(out, exist_ok=True)

    try:
        urllib.request.urlopen(f"{url}/api/state", timeout=5).read()
    except Exception as e:
        print(f"no daemon at {url}: {e}", file=sys.stderr)
        return 1

    if not os.path.exists(CHROME):
        print(f"Chrome not found at: {CHROME}", file=sys.stderr)
        return 1

    print(f"==> capturing from {url}")
    chrome = Chrome()
    failed = 0
    try:
        for page in PAGES:
            path = os.path.join(out, f"{page}.png")
            if chrome.shoot(f"{url}/?page={page}", path):
                print(f"    {page:<14} {os.path.getsize(path) // 1024}K")
            else:
                print(f"    {page:<14} FAILED — console never reached a connected state")
                failed += 1
    finally:
        chrome.close()

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
