#!/usr/bin/env bash
# Build release artifacts locally.
#
# From an Apple Silicon Mac this builds all six binary targets — macOS, Linux
# and Windows, x86_64 and arm64 — without a VM or a container runtime:
#
#   macOS    rustc's own cross-compilation (both arches use the host toolchain)
#   Linux    cargo-zigbuild — zig ships the cross sysroot and acts as linker
#   Windows  cargo-xwin — downloads the MSVC CRT and Windows SDK headers
#
# The two extra tools are optional: a target whose tool is missing is skipped
# with a note rather than failing the build.
#
#   brew install zig && cargo install cargo-zigbuild cargo-xwin
#
# Cross-built binaries here are *compiled*, not *tested* — nothing in this
# script runs a Linux or Windows binary.
#
#   scripts/release-local.sh            build into dist-release/
#   scripts/release-local.sh --upload   also create/update the GitHub release
#
# The version comes from the workspace Cargo.toml, so tag and artifacts agree.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"
out="$repo/dist-release"
upload=0
[[ "${1:-}" == "--upload" ]] && upload=1

version="$(awk '/^\[workspace.package\]/{f=1} f&&/^version = /{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml)"
tag="v${version}"
echo "==> caspar-AV ${tag} (local build)"

rm -rf "$out"; mkdir -p "$out"
skipped=()

# 1. The console. caspar-avd serves web/, so every archive needs it built.
echo "==> building console"
npm --prefix console ci --silent
npm --prefix console run build >/dev/null

BINS=(caspar-avd)

# Package one built target: .zip for Windows, .tar.gz elsewhere.
package() {
  local name="$1" target="$2" ext="${3:-}"
  local stage="$out/stage-$name"
  rm -rf "$stage"; mkdir -p "$stage"
  for b in "${BINS[@]}"; do cp "target/$target/release/${b}${ext}" "$stage/${b}${ext}"; done
  cp -r web "$stage/web"
  cp README.md LICENSE "$stage/"
  mkdir -p "$stage/docs" "$stage/scripts"
  cp docs/*.md "$stage/docs/"
  # The fake server and the probe ship with the binary on purpose: the first
  # lets someone try the console with no CasparCG at all, and the second is how
  # you check this build's protocol assumptions against your own server.
  cp scripts/fake-caspar.py scripts/protocol-probe.py "$stage/scripts/"
  if [[ "$ext" == ".exe" ]]; then
    (cd "$stage" && zip -qr "$out/caspar-av-${name}.zip" .)
  else
    (cd "$stage" && tar czf "$out/caspar-av-${name}.tar.gz" .)
  fi
  rm -rf "$stage"
}

# 2. macOS — rustc cross-compiles between the two arches unaided.
for pair in "macos-aarch64:aarch64-apple-darwin" "macos-x86_64:x86_64-apple-darwin"; do
  name="${pair%%:*}"; target="${pair##*:}"
  echo "==> building ${name} (${target})"
  rustup target add "$target" >/dev/null 2>&1 || true
  cargo build --release --target "$target" --bins
  package "$name" "$target"
done

# 3. Linux — zig supplies the sysroot and linker, so no container is needed.
if command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
  for pair in "linux-x86_64:x86_64-unknown-linux-gnu" "linux-aarch64:aarch64-unknown-linux-gnu"; do
    name="${pair%%:*}"; target="${pair##*:}"
    echo "==> building ${name} (${target})"
    rustup target add "$target" >/dev/null 2>&1 || true
    cargo zigbuild --release --target "$target" --bins
    package "$name" "$target"
  done
else
  skipped+=("linux (needs: brew install zig && cargo install cargo-zigbuild)")
fi

# 4. Windows — xwin fetches the MSVC CRT + Windows SDK. Setting the licence
# variable here is deliberate: the script is the record of having accepted it.
if command -v cargo-xwin >/dev/null 2>&1; then
  export XWIN_ACCEPT_LICENSE=1
  for pair in "windows-x86_64:x86_64-pc-windows-msvc" "windows-aarch64:aarch64-pc-windows-msvc"; do
    name="${pair%%:*}"; target="${pair##*:}"
    echo "==> building ${name} (${target})"
    rustup target add "$target" >/dev/null 2>&1 || true
    cargo xwin build --release --target "$target" --bins
    package "$name" "$target" ".exe"
  done
else
  skipped+=("windows (needs: cargo install cargo-xwin)")
fi

echo
echo "==> artifacts in $out"
ls -lh "$out" | awk 'NR>1 {printf "    %-42s %s\n", $9, $5}'
if ((${#skipped[@]})); then
  echo
  for s in "${skipped[@]}"; do echo "    SKIPPED: $s"; done
fi
echo
echo "    Cross-built Linux/Windows binaries are compiled but not run here."

# 5. Publish.
if [[ "$upload" == "1" ]]; then
  echo "==> publishing $tag"
  shopt -s nullglob
  assets=("$out"/*.tar.gz "$out"/*.zip)
  if gh release view "$tag" >/dev/null 2>&1; then
    gh release upload "$tag" "${assets[@]}" --clobber
  else
    gh release create "$tag" "${assets[@]}" \
      --title "caspar-AV $tag" --notes-file "$repo/docs/release-notes.md"
  fi
  gh release view "$tag" --json assets --jq '.assets[] | "    \(.name)"'
fi
