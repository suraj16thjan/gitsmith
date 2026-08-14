# gitsmith

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-orange.svg)](https://www.rust-lang.org)

A fast terminal UI for **GitLab and GitHub**, built with [`ratatui`](https://github.com/ratatui/ratatui). `gitsmith` wraps the official `glab` and `gh` CLIs and auto-detects which one to use from your git remote — one keyboard-driven interface for issues, merge/pull requests, pipelines, jobs, logs, diffs, and more.

## Installation

### Quick install (macOS / Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/suraj16thjan/gitsmith/main/install.sh | bash
```

Downloads the prebuilt binary for your platform from the latest GitHub release
and installs it to `~/.local/bin` (or `/usr/local/bin` when writable). Pin a
version with `GITSMITH_VERSION=v0.2.2` or change the target with
`GITSMITH_BIN_DIR=~/bin`. No Rust toolchain required.

### From crates.io

```bash
cargo install gitsmith --locked
```

### From source

```bash
git clone https://github.com/suraj16thjan/gitsmith
cd gitsmith
cargo install --path . --locked   # installs the `gitsmith` binary to ~/.cargo/bin
```

Or just run it in place without installing:

```bash
cargo run --release
```

## Requirements

- Rust (stable)
- [`glab`](https://gitlab.com/gitlab-org/cli) authenticated for GitLab repos, and/or [`gh`](https://cli.github.com) authenticated for GitHub repos
- Run it anywhere — inside a git repo it picks the host up from the remote; outside one it asks

`gitsmith` detects the backend from the repo's remote (or from `GITLAB_HOST` / `GH_HOST`) and shells out to the matching CLI, so it uses your existing auth — no tokens to configure. Started outside a git repo, it asks which host to use and then lists the repos on your account to open.

## Features

- **11 tabs** — Issues, MRs/PRs, Pipelines, Runners, Releases, Tags, Branches, Commits, Todos, Milestones, Members
- **Every state listed** — issues and MRs come back open *and* closed, so a closed issue can be reopened; open items sort above settled ones, and protected branches above the rest
- **Labels and branches** — labels as their own column on Issues and MRs/PRs, and each MR/PR shows its `source → target` branch (both matched by the filter)
- **Detail view** (`Enter`) — metadata, description, and the comment thread for issues/MRs
- **Side-by-side diffs** — syntax-highlighted (syntect), with line numbers, a **sticky file header**, and a file list beside the diff that follows your scroll (`[` / `]` jump between files); `v` on an MR, or `Enter` on a commit
- **Pipeline drill-down** — `Enter` a pipeline → its jobs; `Enter` a job → its **live-tailing ANSI log**, with severity coloring for plain output (ERROR/WARN/INFO, dim timestamps, `$` script lines) and the CI's own colors passed through untouched (follows the latest output like GitLab's trace). Parent/bridge jobs drill into downstream pipelines.
- **Members and their roles** — who has access, sorted strongest first (GitLab access levels, GitHub permissions), including group-inherited access on GitLab
- **Add a member** (`n` on the Members tab) — username plus a role picked from the host's own ladder (Guest…Owner, or read…admin); leaving the role blank grants the weakest one
- **Create MRs/PRs and issues** (`n`) — a new MR/PR on the MRs/PRs tab, a new issue on Issues, with a type-to-filter dropdown of the repo's branches. Works with or without a local checkout: an empty Source uses the branch you're on, an empty Target uses the project's default branch
- **Create pipelines and tags** (`n`) — a new CI pipeline on the Pipelines tab, or a new tag on the Tags tab; each is a small form with a type-to-filter dropdown of the repo's branches, and an empty branch means the project's default branch
- **Paged lists** — every tab pulls 100 rows a page and keeps fetching in the background (up to 500) while you read, instead of stopping at the API's 20–30 row default
- **Actions** — approve / merge / close MRs, close/reopen/comment on issues, retry/cancel pipelines and individual jobs — all behind a `[y/n]` confirm
- **Works outside a git repo** — start anywhere and gitsmith asks for a host (`1` GitLab / `2` GitHub), lists your repos, and drops you into the normal workflow
- **Multi-host / multi-config** — reads `glab`'s `config.yml` (and `gh`'s `hosts.yml`) directly, so every configured instance is offered instantly and offline. Looked for where each CLI keeps it: `$GLAB_CONFIG_DIR` / `$GH_CONFIG_DIR`, then `$XDG_CONFIG_HOME`, then `~/.config/…`, then macOS `~/Library/Application Support/…` or Windows `%APPDATA%\…`. Falls back to `auth status`, and says `run glab auth login` when there's nothing configured. Only host names are read — tokens in those files are never touched
- **Host switcher** (`S`) — jump between GitLab and GitHub (and between instances) without leaving the TUI
- **Fuzzy repo switcher** (`P`) — jump between your repos on the host (prefetched at startup)
- **Tab management** (`T`) — reorder the tab bar (`J`/`K`) and hide the tabs you don't use (`space`); `r` restores the defaults
- **Theme picker** (`t`) — Neon, Tokyo Night, Gruvbox, Rosé Pine, Catppuccin, Nord, Dracula, Solarized, Everforest (live preview)
- **Persisted preferences** — the theme and tab layout are saved to `~/.config/gitsmith/config.json` (`%APPDATA%\gitsmith\config.json` on Windows; override either with `GITSMITH_CONFIG`)
- **Command visibility** (`x`) — see every `glab`/`gh` command gitsmith runs, with success markers
- Protected branches highlighted; state/status coloring throughout

The header's right side always lists the keys that do something in the view you're in — the list, a detail, a diff, a picker, a form — so the full list below is reference, not something to memorize.

## Keybindings

### Global
- `h` / `l` (or `←` / `→`) — previous / next tab (hidden tabs are skipped)
- `j` / `k` (or `↓` / `↑`) — move selection
- `Ctrl-d` / `Ctrl-u` — half a screen down / up
- `Enter` — open detail (or jobs on Pipelines, diff on Commits)
- `v` — diff (MRs)
- `n` — new MR/PR (MRs/PRs), new issue (Issues), add member (Members), new pipeline (Pipelines), new tag (Tags)
- `f` or `/` — filter the current tab (each tab keeps its own query; `Esc` clears it)
- `o` — open in browser
- `P` — switch repo (fuzzy)
- `S` — switch host (GitLab / GitHub, and which instance)
- `T` — tab manager (reorder / hide tabs)
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

## Development

```bash
cargo run            # run the TUI from source (debug build)
cargo test           # run the test suite
cargo fmt            # format
cargo clippy         # lint
```

The codebase is a small set of focused modules: `backend/` (the `glab`/`gh`
shell-outs and host detection), `app.rs` (state and key handling), `ui.rs`
(rendering), plus `diff.rs`, `detail.rs`, `fetch.rs`, and `config.rs`.

## Contributing

Contributions are welcome. Please:

1. Fork the repo and create a branch off `main`.
2. Make your change, keeping it consistent with the existing style.
3. Ensure `cargo test`, `cargo fmt --check`, and `cargo clippy` all pass.
4. Open a pull request describing the change and how you verified it.

For bugs and feature ideas, open an issue at
<https://github.com/suraj16thjan/gitsmith/issues>.

## License

Licensed under the [MIT License](LICENSE) © 2026 Suraj Lama.
