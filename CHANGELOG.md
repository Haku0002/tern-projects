# Changelog

All notable changes to this customized fork (`Haku0002/tern-projects`, forked from `thegeneralist01/tern-projects`) will be documented in this file.

## [Unreleased]

### Added
- **Chat placement & grouped context menus**: chat context menus separate placement (**Open in Main**, **Split Right**, **Split Down**, **Open in New Tab**), forking, live-pane controls, organization (**Rename…**, **Pin**, **Move to project…**, **Archive**), copy/options, and **Delete permanently…** with dividers in both native right-click and keyboard menus. Project menus separate new-chat placement, project editing/removal, and options.
- **Collapsible sidebar**: a header button collapses Projects, chosen under **Settings → Sidebar → When collapsed**: hidden (parked; brought back by `Ctrl + Alt + S` or the status line's **Projects** button), an icon rail (expand, Projects, Search, New chat, Settings), or a tile (a card floating over the main chat's top-left corner with a large **Expand Projects** button, the three latest chats and New chat). Expand restores the last width. Width, collapsed state, collapse style and unfolded projects persist in `tern.kv`.
- **Sidebar in every tab**: new tabs open with the sidebar on their left, without taking focus (off switch: **Settings → Sidebar → Open in new tabs**; skipped while the sidebar is hidden).
- **Selection card switch**: **Settings → Sidebar → Selection card** turns off the card that clicking a chat or folder shows at the sidebar's foot; a click then only selects, and right-click, `Enter` and `Ctrl + Y` work as before.
- **Parked tile management & live chat state**: when panes sit in Tern's parked-pane deck, a foldable **Parked** section at the top of Projects lists them with live status (`Working`, `Waiting`, `Exited`), an inline `×` button to close all idle parked tiles in one click, `Delete` to close the selected tile, and a right-click menu (**Close parked tile**, **Close idle parked**, **Close all parked…**) that closes tiles directly without unparking them first. Chat rows across Pinned, Projects, Recents and Search also show live open (`●`) / parked (`○`) dots and offer **Close parked tile** / **Park chat pane** / **Close chat pane** on right-click. Added **Projects: Close Parked Panes…** in the command palette (and a `Parked · N` status segment when the status bar is on) plus `Ctrl + Enter` (Split Right) and `Ctrl + Shift + Enter` (Open in New Tab) shortcuts.
- **Fork chat from context menu & keyboard**: right-clicking an OMP, Claude Code or Codex chat (including in the **Parked** section) offers **Fork chat** (`Ctrl + B` / `⌘B`), **Fork in Split Right** (`Ctrl + Shift + B` / `⇧⌘B`), **Fork in Split Down** and **Fork in New Tab** (`omp --fork <file>`, `claude --resume <id> --fork-session`, `codex fork <id>`). Existing indexed rows gain fork commands on read without requiring a database wipe, and OMP's rotation grouping preserves `omp --fork` copies as separate conversations. Unrenamed forked sessions automatically show a `[Fork] ` title prefix (removed once you rename the session via `/rename`), and fork commands never hijack the parent chat's live indicator or lock guard.
- **Rename chats (`Rename…` / `F2`)**: right-clicking any chat (including chat-backed rows in **Parked**) or pressing `F2` opens a **Rename chat** dialog across all six harnesses without touching source transcript files. Custom titles are stored durably in `library.sqlite` (schema v2, migrated cleanly from v1), keep FTS5 search synced, survive `index.sqlite` rebuilds, and include a **Reset original** action to restore the indexed title.
- **Source-folder ignores**: right-clicking a folder in **Discovered** offers **Ignore source folder…** (hiding that folder, subfolders, and worktrees from Projects, Recents, Pinned, Search, and counts without deleting files); undo anytime in **Settings → Chats → Ignored source folders**.
- **Native shortcut inspection & settings**: **Settings → Global shortcuts** reads Tern's effective keybindings (`Open Projects`, `Search everywhere`, `Projects Settings`), lets you copy action IDs, and links directly to Tern's Keyboard preferences and `settings.json`, alongside a full **Panel controls** reference.

### Changed
- Double-click, `Enter` and the dock's **Resume** open chats in the main area beside the pinned sidebar instead of a new tab; the replaced chat is parked (still running) and swaps back when reopened. New chats follow the same placement. Only **Open in New Tab** creates a tab.
- A chat that is already open is focused or swapped back in rather than launched twice.
- The sidebar is one per tab: toggling deals a parked sidebar back in instead of starting another.
- A sidebar that opens in a new tab, comes back from hiding or docks back from the tile lands at the remembered width straight away instead of appearing at half the tab first. The window sizes it from the tab width the sidebar last measured (`sidebar.tab_cols`); until one is measured it still fits itself after landing.
- Opening a chat by double-click or `Enter` closes the selected chat's action card, including after a single click shortly before the double-click (Tern reports the third click as a fresh selection, which no longer reopens the card, also when the chat opens in another tab).
- Pressing `Ctrl + Alt + S` while the expanded sidebar is focused now collapses it using **Settings → Sidebar → When collapsed** (instead of destroying the pane), and expands a rail-collapsed sidebar back on the next press. Hiding sidebars in multiple tabs keeps at most one parked Projects sidebar in the deck, and replacing an exited pane or untouched OSC-133 shell prompt in the main area closes it instead of parking a dead/blank shell tile.
- Sidebar actions carry a shared per-VM capability token (`sidebar.auth`) and panel pane id, so right-clicking or clicking buttons in an unfocused sidebar works immediately while links in terminal/chat output still cannot trigger plugin routes; each sidebar also tracks its tab's active chat folder for **New chat** and **Worktree / Repository** search scopes, and dragging a floating **Tile** sidebar into a split (`float_moved`) docks and expands it cleanly.
- **Retained project chats across folder edits**: editing a curated project's folders retains existing chat associations (`library.sqlite` schema v4), migrating v1–v3 libraries cleanly.
- **ACP resume preferences**: resuming Codex/Claude chats through the sidebar honors Tern's `agents.auto_acp` and `agents.keep_tui` settings at the window launch boundary while keeping copied resume commands unmodified.
- **Responsive New chat launches**: New chat displays slow-progress feedback, cancels cleanly with `Esc` or **Cancel**, recovers cleanly on timeout, and checks `Retry` only on failure so the 44-column picker footer never clips.
- **Hardened chat deletion**: OMP deletions remove owned prompt-history and FTS entries cleanly (refusing on incompatible history schemas); Claude, Codex, OpenCode, Cursor, and Pi deletions strengthen ownership verification, symlink/identity checks, and rollback/revalidation under SQLite write locks.
- **Improved Cargo discovery**: automatic backend compilation searches `CARGO_HOME` and standard toolchain directories outside minimal GUI `PATH`s on Windows and Unix.

### Fixed
- Windows: the backend no longer stores or emits `\\?\` verbatim paths. The index schema version is bumped to 3 (`2` dropped verbatim paths; `3` adds `[Fork] ` title prefixing for unrenamed forks), so the history index rebuilds once on first run (the project library is untouched).
- OMP chats no longer show up twice (or more). When a session is open in more than one omp process, omp moves it to a new session file with the history copied over and keeps the old file; the index listed every file as its own chat. It now follows that chain (`continuationOf`, or for older omp a parent link plus the copied history and inherited prompt-cache key) and lists it once, as the most recently written file, under the first session's id so pins and project assignments stay put. `/new` sessions and branches are still separate chats. Permanently deleting such a chat removes every file of the chain.
- Opening an OMP chat that another omp process already has open (outside this sidebar's panes) no longer starts a second omp on it, which is what made omp copy it to a new file. A notice names the running process instead.
- Tern 0.9.5 compatibility: `window.luau` falls back to scanning `cx.session:panes()` whenever `cx.session:pane` returns `nil`, includes a `cx:new_block` split fallback when opening the sidebar, and guards `cx.settings:get("status_bar")` with `pcall` so hiding the sidebar never raises `Unknown preference 'status_bar'`.
- A chat inside nested projects (for example `E:\Game Development` and `E:\Game Development\OtogameWIKI`) is listed only under the most specific project instead of under both. Chats placed explicitly with **Move to project…** or retained after a folder edit stay where they were put.
- An OMP fork nobody has written in yet (only its parent's copied history) is no longer listed as another `[Fork]` copy of the same chat; it appears as soon as a message is sent in it. The index schema version is bumped to 4, so the history index rebuilds once (the project library is untouched).

---

## [0.2.3] — 2026-10-16

### Added
- **Visual hierarchy**: title-case section headings (`Pinned`, `Projects`, `Folders`, `Recents`), aligned margins, and an inline `+` button on the Projects header for immediate project creation.
- **Multi-folder workspaces**: projects group multiple source folders with a designated **Primary** launch directory; automatic `--add-dir` flag passing for supported harnesses (OMP, Claude Code, Codex).
- **One-handed keyboard navigation**: 4-stop `Tab` view cycling (`My projects` ➔ `Discovered` ➔ `Search` ➔ `Settings`), `Shift+F10` options menu, keyboard-navigable archived pages (`Ctrl+P` / `Ctrl+N`), and `Esc` hide for in-flight mutations.
- **Inspectable dock**: dedicated branch indicators (`⎇ main`) that don't truncate when directory paths are long.
- **Safe operation lifecycle**: operation errors stay inside their active sheet; in-flight tasks can be dismissed (`Esc`) without cancelling background execution.

### Changed
- Overlay cards, dialogs, and menus now use solid backgrounds (`var(--card)`) to eliminate background bleed-through.
- Unified primary buttons to match native Tern sheet styling in both light and dark appearances.

### Fixed
- Serialized SQLite first-open initialization lock to eliminate database busy errors during concurrent first launches.
- Preserved nearby row selection when an active project, pin, or chat is archived, unpinned, or deleted.

---

## [0.2.2] — 2026-10-15

### Added
- **Automatic backend build**: compiles the Rust indexer in the background on first plugin load without manual steps.
- **Product rename**: clean migration from legacy `tern-sessions` namespace to `tern-projects`.

### Fixed
- Serialized index schema migration in a single SQLite transaction with lock re-checks to prevent startup races.

---

## [0.2.0] — 2026-10-15

### Added
- Curated **My Projects** view alongside automatic **Discovered** folder groups.
- Durable project metadata storage in `library.sqlite`, isolated from the disposable search cache.
- Fresh chat launching directly into installed agent CLIs (`omp`, `claude`, `codex`, `pi`, `opencode`, `cursor`).
- Global pinned chats and 15-chat recents list.
- Local chat archiving and verified native deletion across all 6 supported harnesses.
