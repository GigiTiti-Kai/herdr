#!/usr/bin/env bash
# Sync the fork with upstream and rebuild.
#   fork/sync.sh                 -> fast-forward master to the newest v* release tag
#   fork/sync.sh upstream/master -> track upstream HEAD instead
# master mirrors upstream (never commit there, never pushed); dev carries our changes.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

if [ -n "$(git status --porcelain)" ]; then
    echo "working tree not clean; commit or stash first" >&2
    exit 1
fi

git fetch upstream --tags --prune
target="${1:-$(git tag --list 'v*' --sort=-v:refname | head -1)}"
echo "syncing master -> $target"

git checkout master
git merge --ff-only "$target"
git checkout dev
# On conflict this exits non-zero: resolve, `git commit`, then run fork/build.sh
# and `git push origin dev` by hand.
git merge --no-edit master
git push origin dev
"$repo/fork/build.sh"
