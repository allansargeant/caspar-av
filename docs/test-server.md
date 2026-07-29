# Running a real CasparCG to test against

How the verification in [scope.md](scope.md) was done, so it can be repeated.

CasparCG ships **no macOS build and no arm64 build** — every release asset is
amd64 (a Windows zip, and Ubuntu jammy/noble debs). On an Apple Silicon Mac that
means a VM.

## Ubuntu 24.04 amd64 under QEMU

Full TCG emulation, since there is no hardware acceleration for a foreign
architecture. It is slower than native but perfectly usable — the guest boots in
about thirty seconds, and protocol testing is not throughput-bound.

```bash
curl -LO https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img
cp noble-server-cloudimg-amd64.img disk.qcow2
qemu-img resize disk.qcow2 24G
```

A cloud-init seed ISO avoids an interactive install. On macOS the volume label
is what matters — cloud-init's NoCloud source looks for `CIDATA`:

```bash
hdiutil makehybrid -iso -joliet -default-volume-name CIDATA -o seed.iso seed/
```

with `seed/meta-data` and `seed/user-data` setting a user and an SSH key.

```bash
qemu-system-x86_64 \
  -machine q35 -cpu max -smp 4 -m 6144 \
  -drive file=disk.qcow2,if=virtio,format=qcow2 \
  -drive file=seed.iso,if=virtio,format=raw,readonly=on \
  -netdev user,id=n0,hostfwd=tcp::2222-:22,hostfwd=tcp::5250-:5250 \
  -device virtio-net-pci,netdev=n0 \
  -display none -serial file:console.log
```

User-mode networking is enough, and matters for telemetry: the guest reaches the
host at **10.0.2.2**, which is where CasparCG's OSC ends up when the bridge
connects from the host. AMCP arrives via the `5250` forward.

## Installing the server

```bash
sudo apt-get install -y \
  ./casparcg-cef-142_*-noble1_amd64.deb \
  ./casparcg-server-2.5_*-noble1_amd64.deb \
  mesa-utils libgl1-mesa-dri xvfb ffmpeg
```

## Headless needs Xvfb, not the EGL path

This is the part that wastes an afternoon. CasparCG chooses its backend by
whether `DISPLAY` is set (`context.cpp:119`): unset means EGL, set means SFML on
GLX. The EGL path looks like the right one for a headless box, and it fails:

```
Failed to initialize OpenGL: eglChooseConfig
```

because it asks for `EGL_SURFACE_TYPE = EGL_PBUFFER_BIT`, and Mesa's
*surfaceless* platform — the one you get with no GPU and no display — offers no
pbuffer configs at all. Surfaceless EGL does report OpenGL 4.5, which makes the
failure look like a driver problem rather than a config mismatch.

So run a virtual X server and take the GLX path, where llvmpipe does provide
what SFML asks for:

```bash
Xvfb :99 -screen 0 1280x720x24 &
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 casparcg-server-2.5 casparcg.config
```

`glxinfo -B` under that display should report
`llvmpipe` and `OpenGL core profile version 4.5`.

CEF's GPU subprocesses will crash repeatedly — no GPU, no dbus. Noisy but
harmless; it only means the HTML producer is unavailable.

A minimal config is in [`casparcg-test.config`](casparcg-test.config): two
channels at 720p2500, no consumers, AMCP on 5250. Low resolution and frame rate
on purpose — emulation makes them expensive and geometry checks do not need
either. No consumers because 2.5.0 only mixes channels that have one, and
`PRINT` acts as a transient consumer when a still is needed.

## Then

```bash
python3 scripts/protocol-probe.py --host 127.0.0.1 --port 5250
python3 scripts/verify-mapping.py     # needs ssh access to the guest for PNGs
caspar-avd --host 127.0.0.1
```

Test media, without needing any:

```bash
ffmpeg -f lavfi -i testsrc=size=640x360:rate=25:duration=8 -pix_fmt yuv420p TESTCLIP.mp4
```

or skip it entirely — `PLAY 1-10 #FFFFFFFF` uses the built-in colour producer,
which is what the geometry checks use. Note the format is **`#AARRGGBB`**, alpha
first.
