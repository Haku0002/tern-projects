# AGENTS.md — tern-projects

A high-level guide for AI agents working in this repository.

## Repo layout

```
plugin.toml          Tern plugin manifest
styles.css           Theme & layout styling for panel surfaces
CHANGELOG.md         Release history and notable changes
CONTRIBUTING.md      Development workflows and PR guidelines
LICENSE              Repository license
docs/                Documentation and technical guides
  adding-a-harness.md Step-by-step guide for implementing new assistant adapters
  screenshots/       Showcase images for README and documentation
host.luau            Host process half: registers the panel block
window.luau          Window process half: keybind migration, per-tab sidebar toggle/sizing, chat placement routes
blocks/panel.luau    Main UI: Projects (My Projects / Discovered), Search, Settings
lib/backend.luau     Automatic Cargo build at host startup, binary cache, request queuing
lib/cli.luau         Binary discovery, CLI invocation, JSON decoding
lib/fmt.luau         Formatting helpers: dates, agent labels/icons, URL encoding, snippets
lib/input.luau       Keyboard shortcuts and native UTF-16 edit handling
lib/picker.luau      Modal picker component for selection sheets
lib/resume.luau      Window-side ACP resume command selection using Tern ACP preferences
lib/store.luau       `tern.kv` keys and helpers shared by the panel and window (sidebar width/collapsed/collapse style/measured tab width/auto tabs/expansion, open chats)
lib/project-editor.luau  Unsaved project draft state and child directory browser
lib/textfield.luau   UTF-8 text field component
lib/ui.luau          Reusable UI elements, layout widgets, and list views
indexer/             Rust backend (`tern-projects` CLI binary)
  src/main.rs        CLI subcommands: index, query, projects, library, migrate-data
  src/model.rs       Data types: Candidate, Parsed, Resume, SessionJson
  src/text.rs        String utilities: clean_user, shell_join, ISO timestamps
  src/git.rs         Filesystem-based Git repository and worktree resolution
  src/db.rs          Disposable index SQLite schema, migrations, and queries
  src/library.rs     Durable user library SQLite schema, CRUD, folder membership
  src/sessions.rs    Session projections, scopes, annotations, counts, and paging
  src/harnesses.rs   Identity-verified CLI discovery and fresh-chat launch planning
  src/delete*.rs     Fail-closed per-harness source deletion logic
  src/adapters/      Session adapters for harnesses (claude, codex, omp, pi, opencode, cursor)
  tests/             Real-binary integration test suites with isolated environments
scripts/build.sh     Build helper: compiles release binary to bin/
scripts/check-luau.py Compile runtime Luau sources and enforce `--!strict` headers
bin/tern-projects    Compiled binary (gitignored)
tern.d.luau          Generated Tern type stubs (gitignored)
```

## Architecture Overview

`tern-projects` is a curated project manager, search engine, and launcher for AI agent histories, built as a Tern plugin.

1. **Host / Window Split**:
   - **Host** (`host.luau`): Runs in the Tern host process. Registers blocks, manages persistent host state, and triggers on-demand background builds (`lib/backend.luau`).
   - **Window** (`window.luau`): Runs in each Tern window. Handles keyboard shortcuts, action palette commands, the per-tab sidebar, and dispatches IPC routes (`tern-projects://launch`, `tern-projects://resume`, `tern-projects://sidebar`).

2. **Frontend UI (Luau)**:
   - `blocks/panel.luau`: Primary sidebar UI featuring three main views: Projects (My Projects & Discovered), Search (scoped FTS), and Settings.
   - `lib/`: Reusable components (`ui.luau`, `picker.luau`, `project-editor.luau`, `textfield.luau`, `input.luau`).

3. **Backend CLI (Rust)**:
   - A native Rust binary (`indexer/`) invoked via stdin/stdout JSON requests.
   - Handles background filesystem indexing, full-text search (SQLite FTS5), metadata queries, harness detection, and atomic source chat deletions.


## Maintaining AGENTS.md

This file is the living architectural map for AI agents and human contributors working on `tern-projects`. Always keep it up to date:
- **Structural updates**: Whenever files are added, moved, or deleted, update the `Repo layout` above.
- **Architectural shifts**: If database schemas, storage separation, IPC routes, or deletion contracts change, document the new invariants here.
- **Tooling & workflow**: If build commands, test harnesses, or plugin lifecycle hooks change, update the commands section.
Keeping this file accurate prevents regressions and context drift across autonomous agent sessions.
## VCS Conventions

This repository uses Jujutsu (`.jj`) as its primary version-control interface.
- Always prefer `jj` commands (`jj status`, `jj describe`, `jj diff`, `jj new`) for VCS tasks.
- Avoid destructive or mutating `git` commands unless specifically requested.

## Build & Test Commands

```sh
./scripts/build.sh                                # Build release binary into bin/tern-projects
cargo test --manifest-path indexer/Cargo.toml     # Run backend unit and integration tests
tern plugin types .                              # Regenerate tern.d.luau after Tern updates
tern plugin link .                               # Link plugin to local Tern instance
tern plugin reload                               # Reload plugin in running Tern instance
```

## Core Invariants

- **Durable Library vs. Disposable Index**:
  - `index.sqlite` is a rebuildable cache of history transcripts. Schema version bumps force a reindex.
  - `library.sqlite` stores durable user configurations (curated projects, folder associations, retained `project_chats`, `ignored_sources`, custom pins, chat assignments, global settings; schema v4). Never drop or clear `library.sqlite` during index rebuilds.
- **Automatic Compilation**:
  - `lib/backend.luau` automatically compiles the Rust binary with Cargo when the plugin loads and caches the binary inside `<plugin.data>/backend/`. Requests queue until compilation finishes.
- **Fail-Closed Deletion Safety**:
  - Permanent chat deletion verifies source identity and idle store conditions before removing any files or database records.
  - Harnesses with native CLI commands (e.g. Codex, OpenCode) use their verified native CLI deletion subcommands.
  - Harnesses without native deletion (OMP, Claude, Pi, Cursor) remove only verified session files/artifacts and refuse if the process is running or store ownership cannot be proven.
  - Never delete user project files, repositories, or worktrees during chat deletion.
- **Launch and Resume Contracts**:
  - Fresh chats launch with the primary project directory as `cwd`. Any additional folders are passed via harness-supported CLI flags (e.g. `--add-dir`), or a warning is surfaced before launching primary-only.
  - Command generation must be safely quoted per OS (`shell_join` on Unix, Base64-encoded command blocks on Windows).
  - Panel → window URLs: `tern-projects://resume?...`, `tern-projects://launch?...`, `tern-projects://pane?op=<show|park|close|close-idle-parked|close-all-parked>[&pane=<id>][&place=<…>]`, `tern-projects://sidebar?state=<collapse|expand|fit|sync>&pane=<id>[&cols=<n>][&rail=1]`, and `tern-projects://shortcuts?` / `keyboard?` / `keyconfig?`. Every panel URL carries `&src=<panel_pane>&tok=<sidebar.auth>` (a capability token written by the panel VM to `tern.kv`), verified by `window.luau` along with checking that `src` is a live `tern-projects.panel` (via `find_pane`, falling back to `cx.session:panes()` when `cx.session:pane` returns nil in Tern 0.9.5) in a tab of this window; this allows unfocused right-click actions while preventing terminal/chat links from spoofing routes. `main` parks live chats (closing only already-exited panes or untouched OSC-133 shell prompts) and launches beside them; `right`/`down` split the main pane; `tab` is the only path that opens a new tab. `resume` applies `lib/resume.luau` at the window boundary using current `agents` preferences (`auto_acp`, `keep_tui`) while keeping panel copy commands plain.
  - Live pane sync: `window.luau` inspects `cx.session:panes()` and `cx.agents:list()` (`tern.json.null`-safe) and pushes deduplicated `live-panes` events (`open`, `sessions`, `parked`, and the tab's active main-chat `cwd`) to each open panel; `float_moved` docks a dragged floating tile back to the left edge at its remembered width. Optional `cx.settings:get(...)` reads (such as `status_bar`) are wrapped in `pcall` so older or differing Tern versions never fail the route.
  - Width snapping: the window API can't read pane sizes, so once a fit settles within 2 cells of the remembered width the panel stores `cols / share` as `sidebar.tab_cols`. The window computes a sidebar's share from `cx.session:layout(tab)` (product of right-split ratios on its path) and resizes it to the remembered width in the same handler that docks it, so no frame shows it at Tern's default half split.
  - OMP chains: omp moves a live session to a new file (new id, history copied) when another omp process holds it, and keeps the old file. `adapters/omp.rs` groups such files per folder (`continuationOf`; older omp: `parentSession` + inherited `providerPromptCacheKey` + the same first entry) and lists each chain once, as its newest file, with uid `omp:<first session id>` and `resume.sessions` = every id in the chain. `db::upsert_session` keeps one row per source, so a file that joins a chain drops its old row. Resume URLs carry `&sessions=<ids>`; the window refuses to start omp when a running omp process's command line names one of them (`cx.procs`), unless the chat is already open in a tracked pane (then it is shown). Deletion (`delete_files.rs`) removes every file of the chain and its `history.db` rows.
- **Paths**: Stored and emitted paths never use the Windows verbatim prefix (`\\?\`). The backend normalizes them before writing `library.sqlite`/`index.sqlite` or emitting JSON; Luau compares directories case- and separator-insensitively on Windows.

## Code Conventions

### Luau
- Every file MUST begin with `--!strict`.
- Window scripts (`window.luau`) must not call host-only APIs (`tern.fs.*`, `tern.pane.list`).
- Guard against nil execution contexts and check pane/generation liveness before applying async CLI callback results.
- Avoid text field cursor mismatches: use `lib/input.luau` for consistent native UTF-16 edit conversions.

### Rust
- Clean cutovers: avoid deprecated paths, dead feature flags, and backward-compatibility shims.
- All database operations across tables or multi-step mutations must use transactions with an active busy timeout (5000ms).
- Keep adapters isolated in `indexer/src/adapters/` with uniform error handling via `anyhow`.
- Tests must run against isolated temporary home directories (`tests/fixtures/home/`) using real CLI executions.

## Adding a New Harness / Adapter

See the complete technical guide in **[docs/adding-a-harness.md](docs/adding-a-harness.md)** for detailed implementation patterns, error handling, and test requirements.
