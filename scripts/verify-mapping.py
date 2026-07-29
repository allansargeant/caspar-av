#!/usr/bin/env python3
"""Verify that MIXER FILL and MIXER PERSPECTIVE actually move pixels.

The whole caspar-AV thesis is that CasparCG can be driven as a live-events
media server because `MIXER FILL` places a screen on a canvas and
`MIXER PERSPECTIVE` corner-pins it. That claim is about *geometry*, so checking
the server returned `202` proves nothing. This drives real commands, has the
server `PRINT` a real frame, and looks at where the pixels landed.

Needs the CasparCG test VM: it reads PNGs back over `./ssh.sh` and samples them
with ffmpeg inside the guest. Kept as the record of how the geometry claim was
checked, not as a portable test.
"""

import socket
import subprocess
import sys
import time

GRID = 8  # sample the frame as GRID x GRID average-colour cells


def amcp(sock, cmd, wait=1.0):
    sock.sendall(cmd.encode() + b"\r\n")
    time.sleep(wait)
    sock.settimeout(0.5)
    out = b""
    try:
        while True:
            d = sock.recv(65536)
            if not d:
                break
            out += d
    except Exception:
        pass
    if not out.startswith(b"202") and not out.startswith(b"201"):
        print(f"    ! {cmd} -> {out!r}")
    return out


def ssh(cmd):
    return subprocess.run(
        ["./ssh.sh", cmd], capture_output=True, text=True, cwd=".", timeout=180
    ).stdout


def clear_shots():
    """Remove old frames so a new one is unambiguous."""
    ssh("rm -f /opt/caspar/media/*.png")


def wait_for_shot(timeout=60):
    """Wait for PRINT to actually write its file.

    PRINT returns 202 immediately and writes the PNG afterwards, so reading the
    newest file straight away yields the *previous* frame. Under emulation that
    gap is seconds, which is exactly long enough to make every result look
    plausible while being one step stale.
    """
    for _ in range(timeout):
        if ssh("ls /opt/caspar/media/*.png 2>/dev/null | head -1").strip():
            time.sleep(1.5)  # let the write finish
            return True
        time.sleep(1)
    raise RuntimeError("PRINT never produced a file")


def capture(sock):
    """PRINT a frame that actually reflects the current mixer state.

    PRINT is captured twice and the first result discarded. With no consumers
    configured, 2.5.0 does not mix a channel continuously ("only produce mixed
    frames on channels which have consumers"), so the transient consumer PRINT
    installs grabs the previously buffered frame — the result is a capture that
    is exactly one state stale, which looks entirely plausible and is wrong.
    """
    for _ in range(2):
        clear_shots()
        amcp(sock, "PRINT 1", 1.0)
        wait_for_shot()
    return sample()


def sample():
    """Return a GRID x GRID list of (r,g,b) cells from the printed frame."""
    raw = ssh(
        f"cd /opt/caspar/media && P=$(ls -t *.png | head -1) && "
        f"ffmpeg -v error -i \"$P\" -vf scale={GRID}:{GRID} -f rawvideo -pix_fmt rgb24 - | xxd -p -c 3"
    )
    cells = [c for c in raw.split() if len(c) == 6]
    if len(cells) != GRID * GRID:
        raise RuntimeError(f"expected {GRID*GRID} cells, got {len(cells)}")
    px = [tuple(int(c[i : i + 2], 16) for i in (0, 2, 4)) for c in cells]
    return [px[r * GRID : (r + 1) * GRID] for r in range(GRID)]


def brightness_map(grid):
    return [[max(c) for c in row] for row in grid]


def render(bm):
    ramp = " .:-=+*#%@"
    return "\n".join(
        "      " + "".join(ramp[min(9, v * 10 // 256)] for v in row) for row in bm
    )


def occupied(bm, threshold=96):
    """The set of cells that are lit."""
    return {(r, c) for r, row in enumerate(bm) for c, v in enumerate(row) if v >= threshold}


def expected_cells(x, y, w, h):
    """Cells whose centres fall inside the requested normalised rect."""
    out = set()
    for r in range(GRID):
        for c in range(GRID):
            cx = (c + 0.5) / GRID
            cy = (r + 0.5) / GRID
            if x <= cx < x + w and y <= cy < y + h:
                out.add((r, c))
    return out


def main():
    failures = 0
    s = socket.create_connection(("127.0.0.1", 5250), timeout=15)

    # Opaque white: CasparCG colours are #AARRGGBB, so alpha comes first.
    colour = "#FFFFFFFF"

    cases = [
        ("full frame", 0.0, 0.0, 1.0, 1.0),
        ("top-left quadrant", 0.0, 0.0, 0.5, 0.5),
        ("bottom-right quadrant", 0.5, 0.5, 0.5, 0.5),
        ("right half", 0.5, 0.0, 0.5, 1.0),
        ("centre box", 0.25, 0.25, 0.5, 0.5),
    ]

    for name, x, y, w, h in cases:
        print(f"\n  MIXER FILL {x} {y} {w} {h}   ({name})")
        amcp(s, "CLEAR 1")
        # CLEAR removes producers but NOT mixer transforms — they are separate
        # state and survive it. Without this reset each case inherits the
        # previous one's warp, which looks like a geometry bug and is not.
        amcp(s, "MIXER 1 CLEAR")
        amcp(s, f"PLAY 1-10 {colour}")
        amcp(s, f"MIXER 1-10 FILL {x} {y} {w} {h}")
        time.sleep(1.5)
        bm = brightness_map(capture(s))
        print(render(bm))

        got = occupied(bm)
        want = expected_cells(x, y, w, h)
        if got == want:
            print(f"      PASS — {len(got)} cells lit, exactly where asked")
        else:
            failures += 1
            print(f"      FAIL — extra {sorted(got - want)}, missing {sorted(want - got)}")

    # Corner pin: pull the two right-hand corners inward so the right edge
    # becomes a wedge. The left column should stay lit while the right column
    # loses its top and bottom.
    print("\n  MIXER PERSPECTIVE — right edge pinched to a wedge")
    amcp(s, "CLEAR 1")
    amcp(s, "MIXER 1 CLEAR")
    amcp(s, f"PLAY 1-10 {colour}")
    amcp(s, "MIXER 1-10 FILL 0 0 1 1")
    amcp(s, "MIXER 1-10 PERSPECTIVE 0 0 1 0.4 1 0.6 0 1")
    time.sleep(1.5)
    bm = brightness_map(capture(s))
    print(render(bm))

    left_lit = sum(1 for r in range(GRID) if bm[r][0] >= 96)
    right_lit = sum(1 for r in range(GRID) if bm[r][GRID - 1] >= 96)
    if left_lit > right_lit:
        print(f"      PASS — left edge {left_lit}/{GRID} rows lit, right edge {right_lit}/{GRID}")
    else:
        failures += 1
        print(f"      FAIL — no wedge: left {left_lit}, right {right_lit}")

    amcp(s, "CLEAR 1")
    amcp(s, "MIXER 1 CLEAR")
    s.close()

    print(f"\n  {'ALL GEOMETRY CHECKS PASSED' if not failures else f'{failures} FAILED'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
