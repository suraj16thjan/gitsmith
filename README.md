# gitsmith

A fast terminal UI for **GitLab and GitHub**, built with [`ratatui`](https://github.com/ratatui/ratatui). `gitsmith` wraps the official `glab` and `gh` CLIs and auto-detects which one to use from your git remote — one keyboard-driven interface for issues, merge/pull requests, pipelines, jobs, logs, diffs, and more.

## Installation

```bash
cargo install gitsmith --locked
```

## Requirements

- Rust (stable)
- [`glab`](https://gitlab.com/gitlab-org/cli) authenticated for GitLab repos, and/or [`gh`](https://cli.github.com) authenticated for GitHub repos
- Run it inside a git repo, or set `GITLAB_HOST` / `GH_HOST`

`gitsmith` detects the backend from the repo's remote (or the env vars) and shells out to the matching CLI, so it uses your existing auth — no tokens to configure.

## Features

- **10 tabs** — Issues, MRs/PRs, Pipelines, Runners, Releases, Tags, Branches, Commits, Todos, Milestones
- **Detail view** (`Enter`) — metadata, description, and the comment thread for issues/MRs
- **Side-by-side diffs** — syntax-highlighted (syntect), with line numbers and a **sticky file header**; `v` on an MR, or `Enter` on a commit
- **Pipeline drill-down** — `Enter` a pipeline → its jobs; `Enter` a job → its **live-tailing ANSI log** (follows the latest output like GitLab's trace). Parent/bridge jobs drill into downstream pipelines.
- **Actions** — approve / merge / close MRs, close/reopen/comment on issues, retry/cancel pipelines and individual jobs — all behind a `[y/n]` confirm
- **Fuzzy repo switcher** (`P`) — jump between your repos on the host (prefetched at startup)
- **Theme picker** (`t`) — Tokyo Night, Gruvbox, Rosé Pine, Catppuccin, Nord, Dracula, Solarized, Everforest (live preview)
- **Command visibility** (`x`) — see every `glab`/`gh` command gitsmith runs, with success markers
- Protected branches highlighted; state/status coloring throughout

## Keybindings

### Global
- `h` / `l` (or `←` / `→`) — previous / next tab
- `j` / `k` (or `↓` / `↑`) — move selection
- `Enter` — open detail (or jobs on Pipelines, diff on Commits)
- `v` — diff (MRs)
- `f` or `/` — search / filter
- `o` — open in browser
- `P` — switch repo (fuzzy)
- `t` — theme picker
- `R` — refresh
- `x` — command log
- `?` — help
- `Ctrl-C` — quit

### Tab actions (with confirm)
- Issues: `c` close · `O` reopen · `C` comment (`$EDITOR`)
- MRs/PRs: `a` approve · `M` merge · `c` close
- Pipelines: `r` retry · `d` cancel

### Overlays (diff / detail / jobs / log)
- `j` / `k` / `Ctrl-d` / `Ctrl-u` / `PageUp` / `PageDown` / `g` / `G` — scroll
- Jobs: `Enter` open log (or drill into a bridge's downstream) · `r` retry · `d` cancel
- Log: follows the latest by default; scroll up to pause, `G` to re-follow
- `Esc` / `q` — close

## Run from source

```bash
cargo run
```

## License

MIT
