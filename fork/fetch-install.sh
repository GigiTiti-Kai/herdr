#!/usr/bin/env bash
# Install the fork binary built by GitHub Actions, instead of building locally.
#   fork/fetch-install.sh            -> newest unexpired linux artifact built from dev's head (on GitHub)
#   fork/fetch-install.sh <run-id>   -> that run, whatever commit it built
# Start a build with:
#   gh workflow run build-artifacts-manual.yml -R GigiTiti-Kai/herdr --ref dev -f build_group=linux
# HERDR_INSTALL_DEST=<path> installs elsewhere (to try a build without replacing the daily binary).
# Artifacts are kept 7 days; older runs cannot be downloaded.
# Restarting the server is a separate, manual step (see fork/README.md).
set -euo pipefail

slug=GigiTiti-Kai/herdr
workflow=build-artifacts-manual.yml
name=herdr-linux-x86_64
artifact="$name-ReleaseSafe-simd-false"
dest="${HERDR_INSTALL_DEST:-$HOME/.local/bin/herdr}"
# A bare name would make "$dest" --version run whatever PATH finds, and live-handoff needs a full path.
[[ "$dest" == /* ]] || dest="$PWD/$dest"

run="${1:-}"
if [ -z "$run" ]; then
    # Only a build of dev's current head: an older one would pass every check below.
    # Ask for the artifact itself (newest first, every page), so runs without it (macos-only) never hide it.
    head="$(gh api "repos/$slug/branches/dev" --jq .commit.sha)"
    runs="$(gh api --paginate "repos/$slug/actions/artifacts?name=$artifact&per_page=100" \
        --jq ".artifacts[] | select((.expired | not) and .workflow_run.head_sha == \"$head\") | .workflow_run.id")"
    run="${runs%%$'\n'*}"
    if [ -z "$run" ]; then
        echo "no unexpired $artifact from $workflow for dev's head ${head:0:8}; start a build or pass a run id" >&2
        exit 1
    fi
fi
sha="$(gh run view "$run" -R "$slug" --json headSha --jq .headSha)"

tmp="$(mktemp -d)"
new="$dest.new.$$"
prev="$dest.prev.$$"
trap 'rm -rf "$tmp"; rm -f "$new" "$prev"' EXIT
gh run download "$run" -R "$slug" -n "$artifact" -D "$tmp"

# The artifact must be the build of the run's commit, and the binary must match its own sha256 line.
if ! grep -qx "commit=$sha" "$tmp/BUILD_INFO.txt"; then
    echo "BUILD_INFO.txt does not say commit=$sha" >&2
    exit 1
fi
if ! sum="$(grep -xE "sha256=[0-9a-f]{64}  $name" "$tmp/BUILD_INFO.txt")"; then
    echo "BUILD_INFO.txt has no sha256 line for $name" >&2
    exit 1
fi
(cd "$tmp" && sha256sum -c --quiet - <<<"${sum#sha256=}")

# Running it here also proves the musl binary starts on this machine.
chmod +x "$tmp/$name"
version="$("$tmp/$name" --version)"
if [[ "$version" != *"-fork.${sha:0:8}" ]]; then
    echo "version is '$version', expected it to end with -fork.${sha:0:8}" >&2
    exit 1
fi

# Write next to $dest, then rename: a failed write never leaves a broken $dest.
install -m755 "$tmp/$name" "$new"
cmp -s "$tmp/$name" "$new"
# Keep the previous binary: roll back with `mv $dest.prev $dest`.
# Copied to a fresh file and renamed, so a symlink at $dest.prev is replaced, never written through.
if [ -e "$dest" ]; then
    cp -p "$dest" "$prev"
    mv -fT "$prev" "$dest.prev"
fi
# rename(2) replaces the path, so replacing a running binary is safe.
mv -fT "$new" "$dest"
"$dest" --version
echo "installed run $run ($sha). preserve running panes with: herdr server live-handoff --import-exe $dest"
