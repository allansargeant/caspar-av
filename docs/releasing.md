# Releasing

Releases are built **locally**, not in CI:

```bash
scripts/release-local.sh            # build into dist-release/
scripts/release-local.sh --upload   # and publish the GitHub release
```

Six targets — macOS, Linux and Windows × x86_64 and arm64 — all from an Apple
Silicon Mac, with no VM or container runtime. macOS uses rustc's own
cross-compilation, Linux uses `cargo-zigbuild` (zig supplies the sysroot and
linker), Windows uses `cargo-xwin` (which fetches the MSVC CRT and Windows SDK):

```bash
brew install zig && cargo install cargo-zigbuild cargo-xwin
```

A target whose tool is missing is skipped with a note rather than failing.

## Why no GitHub Actions workflow

This account's Actions minutes are exhausted, so a workflow here would fail in a
few seconds on every push and read as a broken build rather than a billing
limit. Building locally is the honest option until that changes; the script is
written so it can be lifted into a workflow unchanged when it does.

## Verify the artefacts, not the build log

A cross-compile that silently produces the host architecture looks exactly like
a successful build. Check what actually came out:

```bash
tar xzf dist-release/caspar-av-linux-x86_64.tar.gz -O caspar-avd | file -
```

Every archive should report its own architecture — `ELF … x86-64`,
`Mach-O … arm64`, `PE32+ … Aarch64` and so on.

## What is and is not tested

The macOS binary is run against a real CasparCG server before publishing. The
Linux and Windows binaries are **compiled, not executed** — nothing in this
script runs a foreign binary. The release notes say so.

## Version

The tag comes from `version` in the workspace `Cargo.toml`, so bump that first
and the tag and artefacts stay in agreement.
