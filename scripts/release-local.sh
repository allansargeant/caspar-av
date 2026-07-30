#!/usr/bin/env bash
# release-local.sh — cut a full caspar-AV release from this Mac.
#
# caspar-avd serves web/, which the console build produces, so the console is
# built once up front and web/ ships inside every archive.
#
#   scripts/release-local.sh                  build into dist-release/
#   scripts/release-local.sh --version 0.2.0  set an explicit version
#   scripts/release-local.sh --upload         tag and publish the GitHub release
set -euo pipefail

RR_NAME="caspar-AV"
RR_SLUG="caspar-av"
RR_IDENT="com.stoatworks.caspar-av"
RR_EXTRA_FILES=("README.md" "LICENSE" "demo-show.json" "show.json")
RR_EXTRA_DIRS=("web" "docs")
RR_PREBUILD='npm --prefix console ci --silent && npm --prefix console run build >/dev/null'

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-rust.sh"
