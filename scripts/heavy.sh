#!/usr/bin/env bash
# Run a heavy local job (cargo build, test, clippy) under the machine-wide heavy-job lock and memory caps of
# `heavy-run`, where it is installed (~/.local/bin/heavy-run, one job at a time across every repo and worktree).
# Without heavy-run, or already inside one (HEAVY_RUN_ACTIVE), the job runs directly: no second lock or cap.
#
# heavy-run starts the job as a systemd user unit, in the home folder and with none of this environment, and
# systemd-run rewrites `$` in its arguments. So the environment, working folder and arguments all travel on the
# unit's standard input; the job itself gets /dev/null. The unit outlives a killed caller, so a watcher inside it
# ends the unit when this script goes away. At the end it prints the unit's memory peaks. A job the memory cap
# stopped ends with systemd-run's `Finished with result: oom-kill`: not a test failure, do not rerun it unchanged.
set -u

hr=$(command -v heavy-run 2>/dev/null || true)
if [ -z "$hr" ] && [ -x "$HOME/.local/bin/heavy-run" ]; then hr=$HOME/.local/bin/heavy-run; fi
if [ -n "${HEAVY_RUN_ACTIVE:-}" ] || [ -z "$hr" ]; then exec "$@"; fi

# shellcheck disable=SC2016 # the last line's $(cat) is expanded by the bash inside the unit
{
  export -p
  printf 'cd -- %q || exit 2\n' "$PWD"
  printf 'caller=%q\n' "$$"
  printf 'set --'; printf ' %q' "$@"; printf '\n'
  cat <<'EOF'
# A caller stopped while waiting for the lock has nobody left to report to.
kill -0 "$caller" 2>/dev/null || exit 143
unit=/sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup | head -n 1)
( while kill -0 "$caller" 2>/dev/null; do sleep 1; done; echo 1 > "$unit/cgroup.kill" ) &
watcher=$!
"$@" < /dev/null
rc=$?
kill "$watcher" 2>/dev/null
mib() { awk '{ printf "%d", $1 / 1048576 }' "$1" 2>/dev/null; }
echo "heavy: rc=$rc memory.peak=$(mib "$unit/memory.peak")MiB swap.peak=$(mib "$unit/memory.swap.peak")MiB" >&2
exit "$rc"
EOF
} | setsid -w "$hr" -- bash -c 'eval "$(cat)"' "$@" & # the trailing arguments only name the job in jobs.log
# heavy-run holds the lock. In a session of its own, a stop of the caller's process group (the board's, or Ctrl-C)
# does not reach it: it keeps the lock until the watcher has ended the unit. `wait` lets this script stop at once.
trap 'exit 130' INT
wait "$!"
