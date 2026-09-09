#!/usr/bin/env bash
# Build the fork binary and install it as ~/.local/bin/herdr.
# Run from anywhere. Restarting the server is a separate, manual step
# (see fork/README.md) because `herdr server stop` kills every pane.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

export ZIG="${ZIG:-$HOME/.local/opt/zig-0.15.2/zig}"
export HERDR_BUILD_CHANNEL=fork
HERDR_BUILD_ID="$(git rev-parse --short HEAD)"
export HERDR_BUILD_ID

cargo build --release --locked
# install(1) unlinks the destination first, so replacing a running binary is safe.
install -m755 target/release/herdr "$HOME/.local/bin/herdr"
"$HOME/.local/bin/herdr" --version
echo "installed. to pick it up: from a terminal OUTSIDE herdr, run: herdr server stop && herdr"
