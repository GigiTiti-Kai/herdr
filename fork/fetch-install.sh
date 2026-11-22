#!/usr/bin/env bash
# Install the fork binary built by GitHub Actions, instead of building locally.
#   fork/fetch-install.sh            -> newest successful manual build of dev; refused unless it is dev's head
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
dest="${HERDR_INSTALL_DEST:-$HOME/.local/bin/herdr}"

run="${1:-}"
if [ -z "$run" ]; then
    run="$(gh run list -R "$slug" --workflow "$workflow" --branch dev --status success --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
    if [ -z "$run" ]; then
        echo "no successful $workflow run on dev" >&2
        exit 1
    fi
fi
sha="$(gh run view "$run" -R "$slug" --json headSha --jq .headSha)"

# Without a run id, an old build would pass every check below, so it must be dev's head on GitHub.
if [ -z "${1:-}" ]; then
    head="$(gh api "repos/$slug/branches/dev" --jq .commit.sha)"
    if [ "$sha" != "$head" ]; then
        echo "newest build (run $run) is ${sha:0:8}, but dev is at ${head:0:8}; start a build or pass a run id" >&2
        exit 1
    fi
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gh run download "$run" -R "$slug" -n "$name-ReleaseSafe-simd-false" -D "$tmp"

# The artifact must be the build of the run's commit, byte for byte.
if ! grep -qx "commit=$sha" "$tmp/BUILD_INFO.txt"; then
    echo "BUILD_INFO.txt does not say commit=$sha" >&2
    exit 1
fi
(cd "$tmp" && sed -n 's/^sha256=//p' BUILD_INFO.txt | sha256sum -c --quiet -)

# Running it here also proves the musl binary starts on this machine.
chmod +x "$tmp/$name"
version="$("$tmp/$name" --version)"
if [[ "$version" != *"-fork.${sha:0:8}" ]]; then
    echo "version is '$version', expected it to end with -fork.${sha:0:8}" >&2
    exit 1
fi

# Keep the previous binary: roll back with `mv $dest.prev $dest`.
if [ -e "$dest" ]; then
    cp -p "$dest" "$dest.prev"
fi
# install(1) unlinks the destination first, so replacing a running binary is safe.
install -m755 "$tmp/$name" "$dest"
"$dest" --version
echo "installed run $run ($sha). preserve running panes with: herdr server live-handoff --import-exe $dest"
