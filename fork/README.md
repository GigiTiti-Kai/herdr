# fork/ — local fork tooling

This checkout is a personal fork of herdrdev/herdr used as the daily driver.

| command | binary | role |
| --- | --- | --- |
| `herdr` | `~/.local/bin/herdr` (built from `dev`) | daily driver |
| `herdr-upstream` | `~/.local/bin/herdr-upstream` (stock release) | fallback, and bug reproduction before filing an upstream issue |

Run the stock binary only as `herdr-upstream --session upstream` so it never
shares the default socket with the fork server.

## Branches

- `master`: local mirror of upstream, pinned to a release tag. Never commit here;
  `fork/sync.sh` fast-forwards it. It is not pushed (origin/master is whatever
  GitHub forked and is irrelevant).
- `dev`: everything of ours, merged on top of master. Build source. Pushed to origin.

## Commands

- `fork/build.sh` — build with `HERDR_BUILD_CHANNEL=fork`, install to `~/.local/bin/herdr`.
- `fork/sync.sh [<ref>]` — fetch upstream, ff master to the newest `v*` tag (or `<ref>`),
  merge into dev, push dev, build. Resolve conflicts by hand if the merge stops.
- After either: from a terminal **outside** herdr, `herdr server stop && herdr`.
  Layout is restored from `~/.config/herdr/session.json`.

## Refreshing herdr-upstream

`herdr update` replaces `env::current_exe()` (src/update.rs), so running
`herdr-upstream update` from a terminal outside herdr updates
`~/.local/bin/herdr-upstream` in place and never touches the fork binary.

## Upstream policy

herdrdev/herdr auto-closes pull requests from non-approved contributors.
Do not open PRs. Report reproduced bugs with the issue template (reproduce on
`herdr-upstream` first); post feature ideas in Discussions.

## Rollback

    install -m755 ~/.local/bin/herdr-upstream ~/.local/bin/herdr
    herdr server stop && herdr   # outside herdr

If the fork has moved past upstream's session format, the stock binary logs
"session file is from a newer herdr version, ignoring" and starts with an
empty layout.
