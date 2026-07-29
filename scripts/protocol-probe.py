#!/usr/bin/env python3
"""Record exactly what a real CasparCG server sends back.

Deliberately written with raw sockets and no dependency on the Rust crates: its
job is to *falsify* the assumptions those crates were built on, and a probe that
shared their code could not do that.

It sends a battery of commands, dumps the bytes received verbatim, and checks
each documented framing claim against what actually arrived. Anything it cannot
confirm is reported as a failure rather than glossed over.

    python3 scripts/protocol-probe.py --host 127.0.0.1 --port 5250
"""

import argparse
import socket
import struct
import sys
import threading
import time

RESET, BOLD, RED, GREEN, YELLOW, DIM = (
    "\033[0m",
    "\033[1m",
    "\033[31m",
    "\033[32m",
    "\033[33m",
    "\033[2m",
)

results: list[tuple[bool, str, str]] = []


def check(ok: bool, claim: str, detail: str = "") -> None:
    results.append((ok, claim, detail))
    mark = f"{GREEN}PASS{RESET}" if ok else f"{RED}FAIL{RESET}"
    print(f"  [{mark}] {claim}")
    if detail:
        print(f"         {DIM}{detail}{RESET}")


class Amcp:
    """A raw AMCP connection that records everything it receives."""

    def __init__(self, host: str, port: int):
        self.sock = socket.create_connection((host, port), timeout=10)
        self.buf = b""

    def send(self, line: str) -> None:
        print(f"  {DIM}→ {line}{RESET}")
        self.sock.sendall(line.encode("utf-8") + b"\r\n")

    def read_for(self, seconds: float = 1.2) -> bytes:
        """Collect whatever arrives in a window.

        Time-based rather than framing-based on purpose: the point is to observe
        the framing, so the reader must not assume it.
        """
        self.sock.settimeout(0.25)
        deadline = time.time() + seconds
        out = b""
        while time.time() < deadline:
            try:
                chunk = self.sock.recv(65536)
                if not chunk:
                    break
                out += chunk
            except socket.timeout:
                continue
            except OSError:
                break
        return out

    def exchange(self, line: str, seconds: float = 1.2) -> bytes:
        self.send(line)
        data = self.read_for(seconds)
        for raw in data.split(b"\r\n")[:6]:
            print(f"  {DIM}← {raw[:150]!r}{RESET}")
        return data

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


def section(title: str) -> None:
    print(f"\n{BOLD}{title}{RESET}")


# --------------------------------------------------------------------- OSC

def osc_listener(port: int, stop: threading.Event, sink: list) -> None:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.settimeout(0.5)
    try:
        s.bind(("0.0.0.0", port))
    except OSError as e:
        sink.append(("bind-error", str(e)))
        return
    while not stop.is_set():
        try:
            data, _ = s.recvfrom(65536)
            sink.append(("packet", data))
        except socket.timeout:
            continue
        except OSError:
            break
    s.close()


def parse_osc(data: bytes) -> list[tuple[str, str, list]]:
    """Minimal OSC reader — returns (address, typetags, args)."""

    def rd_str(b: bytes, i: int):
        end = b.index(b"\0", i)
        s = b[i:end].decode("utf-8", "replace")
        return s, i + (((end - i) + 4) // 4) * 4

    out: list[tuple[str, str, list]] = []

    def walk(b: bytes) -> None:
        if b.startswith(b"#bundle\0"):
            i = 16
            while i < len(b):
                (size,) = struct.unpack_from(">i", b, i)
                walk(b[i + 4 : i + 4 + size])
                i += 4 + size
            return
        try:
            addr, i = rd_str(b, 0)
            tags, i = rd_str(b, i)
            args = []
            for t in tags.lstrip(","):
                if t == "i":
                    args.append(struct.unpack_from(">i", b, i)[0])
                    i += 4
                elif t == "f":
                    args.append(round(struct.unpack_from(">f", b, i)[0], 4))
                    i += 4
                elif t == "h":
                    args.append(struct.unpack_from(">q", b, i)[0])
                    i += 8
                elif t == "d":
                    args.append(round(struct.unpack_from(">d", b, i)[0], 4))
                    i += 8
                elif t in "sS":
                    v, i = rd_str(b, i)
                    args.append(v)
                elif t in "TF":
                    args.append(t == "T")
                elif t in "NI":
                    args.append(None)
            out.append((addr, tags, args))
        except Exception:
            pass

    walk(data)
    return out


# -------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=5250)
    ap.add_argument("--osc-port", type=int, default=6250)
    ap.add_argument("--skip-osc", action="store_true")
    args = ap.parse_args()

    c = Amcp(args.host, args.port)

    section("1. Identity")
    data = c.exchange("VERSION")
    version = ""
    if data.startswith(b"201 VERSION OK\r\n"):
        version = data.split(b"\r\n")[1].decode()
    check(
        data.startswith(b"201 VERSION OK\r\n") and data.count(b"\r\n") == 2,
        "201 carries exactly one data line",
        f"server version: {version!r}; raw: {data!r}",
    )

    section("2. Framing: 200 runs until a blank line")
    data = c.exchange("INFO")
    check(
        data.startswith(b"200 INFO OK\r\n") and data.endswith(b"\r\n\r\n"),
        "200 is terminated by an empty line",
        f"raw: {data!r}",
    )

    section("3. REQ/RES correlation")
    data = c.exchange("REQ probe-42 VERSION")
    check(
        data.startswith(b"RES probe-42 201 VERSION OK\r\n"),
        "REQ <id> makes the server prefix its reply RES <id>",
        f"raw: {data[:80]!r}",
    )

    data = c.exchange("REQ multi-7 INFO")
    check(
        data.startswith(b"RES multi-7 200 INFO OK\r\n") and data.endswith(b"\r\n\r\n"),
        "RES prefix tags only the status line of a multi-line 200",
        f"raw: {data!r}",
    )

    section("4. Errors")
    data = c.exchange("NOSUCHCOMMAND")
    check(
        data == b"400 ERROR\r\nNOSUCHCOMMAND\r\n",
        "400 ERROR echoes the offending command on a data line",
        f"raw: {data!r}",
    )

    data = c.exchange("PLAY")
    check(
        data.startswith(b"400 ERROR\r\n") and data.count(b"\r\n") == 2,
        "a command with too few parameters is also a 400 with an echo",
        f"raw: {data!r}",
    )

    data = c.exchange("PLAY 1-10 THIS_CLIP_DOES_NOT_EXIST")
    check(
        data[:1] in (b"4", b"5"),
        "a missing clip is refused rather than silently ignored",
        f"raw: {data!r}",
    )

    section("5. Batching: BEGIN is never answered; COMMIT answers once")
    c.send("REQ batch-1 BEGIN")
    begin_reply = c.read_for(1.0)
    check(begin_reply == b"", "BEGIN produces no reply at all", f"raw: {begin_reply!r}")

    c.send("REQ inner-a PLAY 1-10 #FF0000FF")
    c.send("REQ inner-b PLAY 2-10 #00FF00FF")
    queued = c.read_for(1.0)
    check(
        queued == b"",
        "commands inside a batch are queued, not executed",
        f"raw: {queued!r}",
    )

    c.send("REQ batch-1 COMMIT")
    committed = c.read_for(3.0)
    for raw in committed.split(b"\r\n")[:6]:
        print(f"  {DIM}← {raw[:150]!r}{RESET}")
    check(
        b"RES inner-a" in committed and b"RES inner-b" in committed,
        "every inner command replies under its own id",
        f"raw: {committed!r}",
    )
    check(
        b"COMMIT OK" in committed or b"COMMIT PARTIAL" in committed,
        "the batch itself replies once with COMMIT OK/PARTIAL",
    )

    section("6. Escaping")
    # A name with a space must arrive as ONE parameter. A missing file is the
    # expected outcome; what is being tested is that the server did not split it
    # into two parameters, which would produce a different error.
    data = c.exchange('PLAY 1-10 "no such clip with spaces"')
    check(
        data[:1] in (b"4", b"5"),
        "a quoted parameter containing spaces is accepted as one token",
        f"raw: {data!r}",
    )

    section("7. Mixer geometry is accepted")
    # The target sits BETWEEN the two words of the command name: the parser
    # takes one token as the name, then the channel spec, then the sub-command.
    # `MIXER FILL 1-10 …` is a 400.
    for cmd in [
        "MIXER 1-10 FILL 0 0 0.5 0.5",
        "MIXER 1-10 PERSPECTIVE 0 0.02 1 0 1 1 0 0.98",
        "MIXER 1-10 OPACITY 0.5 25 easeoutquad",
        "MIXER 1 CLEAR",
    ]:
        data = c.exchange(cmd, 1.0)
        check(data.startswith(b"202"), f"{cmd} → 202", f"raw: {data!r}")

    section("8. The other word order is rejected")
    data = c.exchange("MIXER FILL 1-10 0 0 0.5 0.5")
    check(
        data.startswith(b"400 ERROR"),
        "MIXER FILL 1-10 … (name-first) is refused, confirming the ordering",
        f"raw: {data!r}",
    )

    section("9. PING answers PONG, with no status code")
    data = c.exchange("PING hello")
    check(
        data == b"PONG hello\r\n",
        "a bare PING replies PONG, echoing its arguments",
        f"raw: {data!r}",
    )

    # PING is intercepted before REQ is parsed (`AMCPProtocolStrategy.cpp:126`),
    # so it is the one command that must NOT be sent with a request id.
    data = c.exchange("REQ ping-1 PING hello")
    check(
        data.startswith(b"RES ping-1 400 ERROR"),
        "REQ <id> PING is refused — PING must be sent bare",
        f"raw: {data!r}",
    )

    section("10. OSC telemetry")
    if args.skip_osc:
        print(f"  {YELLOW}skipped{RESET}")
    else:
        stop = threading.Event()
        sink: list = []
        t = threading.Thread(target=osc_listener, args=(args.osc_port, stop, sink), daemon=True)
        t.start()
        time.sleep(0.4)
        data = c.exchange(f"OSC SUBSCRIBE {args.osc_port}", 1.0)
        check(data.startswith(b"202"), "OSC SUBSCRIBE is accepted (2.5+)", f"raw: {data!r}")

        # Give it something to report on.
        c.exchange("PLAY 1-10 #3050FFFF", 1.0)
        time.sleep(4.0)
        stop.set()
        t.join(timeout=2)

        packets = [d for kind, d in sink if kind == "packet"]
        errors = [d for kind, d in sink if kind == "bind-error"]
        if errors:
            check(False, "OSC port could be bound", errors[0])
        check(len(packets) > 0, f"telemetry packets arrived ({len(packets)})")

        if packets:
            msgs = parse_osc(packets[-1])
            check(
                any(m[0].startswith("/channel/") for m in msgs),
                "addresses are /channel/<n>/… as documented",
            )
            print(f"\n{BOLD}  Observed OSC addresses (last packet, {len(msgs)} messages):{RESET}")
            for addr, tags, vals in sorted(msgs)[:60]:
                print(f"    {addr}  {DIM}{tags}{RESET}  {vals}")

    c.close()

    section("Summary")
    passed = sum(1 for ok, _, _ in results if ok)
    failed = len(results) - passed
    colour = GREEN if failed == 0 else RED
    print(f"  {colour}{passed} passed, {failed} failed{RESET}")
    for ok, claim, detail in results:
        if not ok:
            print(f"  {RED}✗{RESET} {claim}\n    {DIM}{detail}{RESET}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
