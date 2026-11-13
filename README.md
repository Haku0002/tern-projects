# tern-projects

A customized fork of `tern-projects` for [Tern](https://stencil.so/tern), maintained at [Haku0002/tern-projects](https://github.com/Haku0002/tern-projects) (forked from `thegeneralist01/tern-projects` at `v0.2.3`).

---

## Install & Update

Run inside Tern:

```sh
tern plugin install github.com/Haku0002/tern-projects
```

To update to the latest commit:

```sh
tern plugin install github.com/Haku0002/tern-projects --force
```

The Rust indexer builds automatically on first load.

---

## Updates in This Fork

### 1. Pinned Left Sidebar & Main-Pane Workspace
- **Sidebar by default in new tabs**: opens at the left edge without stealing focus (configurable under **Settings → Sidebar → Open in new tabs**), enforces one sidebar per tab, and restores remembered width immediately using measured tab width (`sidebar.tab_cols`).
- **Open in Main by default**: double-click, `Enter`, and the dock's **Resume** button open chats in the main pane beside the sidebar instead of spawning new tabs.
- **Non-destructive pane swapping**: opening another chat in Main parks the running chat in Tern's deck so it continues executing in the background; reopening it swaps it back. Untouched shell prompts and already-exited panes close cleanly instead of accumulating in the deck.
- **Explicit placement options**: every chat menu offers **Open in Main**, **Split Right**, **Split Down**, and **Open in New Tab**; project menus add **New chat in Split Right** and **New chat in Split Down**.
- **Deduplication & lock safety**: already-open chats focus or unpark instead of launching duplicates, and opening an OMP chat held by an external `omp` process is safely blocked with a notice.

### 2. Collapsible Sidebar (`Hide`, `Icon rail`, `Tile`)
- Collapse and expand via the sidebar header button or `Ctrl + Alt + S` (`⌘ ⌥ S`).
- Choose collapse behavior under **Settings → Sidebar → When collapsed**:
  - **Hide**: parks the sidebar (at most one hidden sidebar in the deck; restored via shortcut or the status-bar **Projects** button).
  - **Icon rail**: narrows to a vertical strip with Expand, Projects, Search, New chat, and Settings.
  - **Tile**: floats as a compact card over the chat's top-left corner with **Expand Projects**, recent chats, **New chat**, and one-click idle parked cleanup.
- Width, collapsed state, collapse mode, expanded project rows, and parked-section fold state persist in `tern.kv`.
- **Selection card** toggle (**Settings → Sidebar → Selection card**) lets clicks select rows quietly without showing the bottom action card.

### 3. Parked Pane Deck & Live Status
- Foldable **Parked** section at the top of Projects lists parked panes with live state (`Working`, `Waiting`, `Exited`), inline **Close idle** (`×`), `Delete` key close, and right-click cleanup (**Close parked tile**, **Close idle parked**, **Close all parked…**) with confirmation when a working chat would be stopped.
- Live status indicators (`●` open in tab, `◐` parked working, `○` parked idle) appear across Pinned, Projects, Recents, and Search, alongside **Park chat pane** / **Close chat pane** menu actions.
- Command palette **Projects: Close Parked Panes…** and `Parked · N` status segment allow cleaning up parked tiles even while the sidebar is hidden.

### 4. Safe Chat Forking & `[Fork] ` Prefixing
- Fork **OMP**, **Claude Code**, and **Codex** chats directly from the context menu (**Fork chat**, **Fork in Split Right**, **Fork in Split Down**, **Fork in New Tab**) or keyboard (`Ctrl+B` / `⌘B`, `Ctrl+Shift+B` / `⇧⌘B`), including from chat-backed rows in **Parked**, without touching or interrupting the original session.
- **Automatic `[Fork] ` title prefix**: unrenamed forked chats across OMP (`omp --fork`), Claude Code (`--fork-session` / `forkedFrom`), and Codex (`forked_from_id`) are automatically prefixed with `[Fork] ` (index schema v3) so forks cannot be mistaken for original sessions when deleting; renamed forks drop the prefix cleanly.
- Live fork commands (`--fork`, `--fork-session`, `codex fork`) are excluded from parent-session matching and `omp` lock checks so they never hijack the original chat's live indicator or prevent opening the original chat.

### 5. Grouped Context Menus & Durable Chat Rename (`Rename…` / `F2`)
- **Grouped context menus**: native right-click menus and keyboard menus (`Shift+F10`) separate placement, forking, live-pane controls, title/organization, copy/options, and permanent deletion with horizontal dividers.
- **Rename chat (`Rename…` / `F2`)**: rename any chat across all six harnesses (OMP, Claude Code, Codex, Pi, OpenCode, Cursor) from its context menu, Parked row, or `F2`.
  - Custom titles are stored durably in `library.sqlite` (v2 schema, automatically migrated from v1) without modifying source transcript files or triggering OMP file-rotation locks.
  - Full-text search (`sessions_fts`) stays synchronized with custom titles and persists across `index.sqlite` rebuilds; includes a **Reset original** button to revert to the indexed title.

### 6. Windows & OMP Stability Fixes
- Eliminates Windows `\\?\` verbatim path prefixes across indexed sessions, Git root resolution, folder browsing, and harness launches.
- Collapses rotated OMP session chains (`continuationOf` and legacy pre-18.13 rotations) into a single entry while keeping `/new` sessions and forks separate, and deletes every file in a chain when permanently deleting an OMP chat.
- Selective upstream sync (`v0.2.4`–`v0.2.14` fixes, without upstream self-update checks): reversible **Ignore source folder…** rules in Discovered and **Settings → Ignored source folders**, retained project chats across folder edits (`library.sqlite` v4), ACP resume preference handling (`auto_acp` / `keep_tui`), live **Global shortcuts** inspection in Settings, responsive **New chat** timeout/cancel handling, OMP prompt-history cleanup & hardened fail-closed deletions across all harnesses, improved Cargo `PATH` discovery, and Tern 0.9.5 pane-lookup / `status_bar` preference guards.

---

## Keyboard Shortcuts

| Action | macOS | Windows / Linux |
|---|---|---|
| **Toggle / collapse sidebar** | `⌘ ⌥ S` | `Ctrl + Alt + S` |
| **Search everywhere** | `⌃ ⌥ ⌘ S` | `Ctrl + Alt + Shift + S` |
| **Cycle views** (`Mine` / `Found` / `Search` / `Settings`) | `Tab` / `Shift + Tab` | `Tab` / `Shift + Tab` |
| **Quick Search** | `/` | `/` |
| **Open in Main** | `Enter` | `Enter` |
| **Open in Split Right** | `⌘ Enter` | `Ctrl + Enter` |
| **Open in New Tab** | `⇧ ⌘ Enter` | `Ctrl + Shift + Enter` |
| **Fork chat / Fork in Split Right** | `⌘ B` / `⇧ ⌘ B` | `Ctrl + B` / `Ctrl + Shift + B` |
| **Rename selected chat** | `F2` | `F2` |
| **Close selected parked tile** | `Delete` | `Delete` |
| **New chat / New chat with…** | `⌘ N` / `⇧ ⌘ N` | `Ctrl + N` / `Ctrl + Shift + N` |
| **Fold / unfold project** | `←` / `→` | `←` / `→` |
| **Keyboard context menu / options** | `Shift + F10` | `Shift + F10` |
| **Copy resume command** | `⌃ Y` | `Ctrl + Y` |
| **Refresh index** | `Ctrl + R` | `Ctrl + R` |
| **Clear / dismiss** | `Esc` | `Esc` |

---

## Supported Harnesses

OMP, Claude Code, Codex, Pi, OpenCode, and Cursor. See [CHANGELOG.md](CHANGELOG.md) for detailed release notes and [CONTRIBUTING.md](CONTRIBUTING.md) for build/test instructions.

## License

[MIT](LICENSE)
