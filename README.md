# tern-projects (Customized Fork)

> **Fork Notice:** This repository is a customized fork of [thegeneralist01/tern-projects](https://github.com/thegeneralist01/tern-projects) (based on `v0.2.3` / `86c05d4`), maintained at [Haku0002/tern-projects](https://github.com/Haku0002/tern-projects).
>
> **Key modifications over upstream:**
> - **Pinned left sidebar & main-pane workspace**: opening or starting chats swaps/opens beside the sidebar instead of spawning new tabs by default; supports **Open in Main**, **Split Right**, **Split Down**, and **Open in New Tab**, plus live chat deduplication.
> - **Collapsible sidebar & per-tab persistence**: collapse as **Hide**, **Icon rail**, or floating **Tile**, with per-tab auto-open, remembered width/collapse state, and optional selection card toggle.
> - **Parked tile deck management**: foldable **Parked** section with live status, inline idle cleanup, direct tile closing, and a status-bar / command-palette picker.
> - **Safe chat forking & `[Fork] ` prefixing**: fork OMP, Claude Code, and Codex chats via context menu or `Ctrl+B` / `⌘B` without disrupting running sessions; unrenamed forks are automatically prefixed with `[Fork] `.
> - **Chat renaming (`Rename…` / `F2`) & grouped context menus**: durable custom chat titles (stored in `library.sqlite` with FTS5 sync and **Reset original**, without mutating transcript files) and horizontal dividers grouping context-menu actions.
> - **Windows & OMP fixes**: strips Windows `\\?\` verbatim path prefixes, groups rotated OMP session chains, and blocks duplicate `omp` launches on already-held sessions.

A curated project sidebar and unified agent history search for [Tern](https://stencil.so/tern).

Bring order to your AI coding sessions across **OMP, Claude Code, Codex, Pi, OpenCode, and Cursor**.

![tern-projects sidebar overview](docs/screenshots/showcase-overview.png)

---

## Quickstart

Run this inside Tern:

```sh
tern plugin install github.com/Haku0002/tern-projects
```

**Toggle the sidebar:**
- **macOS:** `⌘ ⌥ S`
- **Windows / Linux:** `Ctrl + Alt + S`

*(Or open **Projects** from the Tern command palette).*

That's it. Tern fetches the plugin, and the background engine builds automatically on first load. No manual compile steps, no extra configuration.

To update later:
```sh
tern plugin install github.com/Haku0002/tern-projects --force
```

---

## What it does

If you use multiple AI coding agents, your sessions are scattered across hidden SQLite databases and JSONL files. `tern-projects` unifies them into a fast, native sidebar:

- 📁 **Curated "My Projects" & Multi-Folder Workspaces** — Real projects rarely live in a single directory. Group an app, its backend API, shared libraries, and documentation into one named project with a custom icon.
- 🎯 **Primary Launch Directories** — Pick one folder as **Primary** (where fresh agent sessions spawn), while additional directories are automatically supplied via `--add-dir` to harnesses that support multi-folder workspaces (OMP, Claude Code, Codex).
- 🔍 **Instant Full-Text Search** — FTS5-powered search across all your past sessions. Filter by **Worktree**, **Repository**, or **All history** with live keyword highlighting.
- 🚀 **Fresh Chat Launching** — Press `⌘N` (`Ctrl+N`) to launch a new chat in your preferred CLI (`omp`, `claude`, `codex`, `pi`, `opencode`, or `cursor`).
- 📌 **Global Pins & Recents** — Keep critical threads pinned to the top of your sidebar. Recents keeps your 15 most recent chats one click away.
- 🗄️ **Local Archiving & Safe Deletion** — Archive threads to hide them without losing data, or permanently delete sessions via verified native CLI calls.

![tern-projects details, menus, and search results](docs/screenshots/showcase-details.png)

---

## Keyboard-First Controls

Optimized for speed and one-handed navigation:

| Action | macOS | Windows / Linux |
|---|---|---|
| **Toggle Projects sidebar** | `⌘ ⌥ S` | `Ctrl + Alt + S` |
| **Search everywhere** | `⌃ ⌥ ⌘ S` | `Ctrl + Alt + Shift + S` |
| **Cycle views** (Projects ➔ Found ➔ Search ➔ Settings) | `Tab` / `Shift + Tab` | `Tab` / `Shift + Tab` |
| **Quick Search** from Projects or Settings | `/` | `/` |
| **New chat** (in current project/folder) | `⌘ N` | `Ctrl + N` |
| **New chat with…** (choose harness) | `⇧ ⌘ N` | `Shift + Ctrl + N` |
| **Open chat in the main area** (also double-click, or the dock's Resume) | `Enter` | `Enter` |
| **Fork chat / Fork in Split Right** | `⌘ B` / `⇧ ⌘ B` | `Ctrl + B` / `Ctrl + Shift + B` |
| **Fold / unfold project** | `←` / `→` | `←` / `→` |
| **Context menu / Actions** | `Shift + F10` | `Shift + F10` |
| **Rename selected chat** | `F2` | `F2` |
| **Copy resume command** | `⌃ Y` | `Ctrl + Y` |
| **Dismiss / Clear search / Close modal** | `Esc` | `Esc` |

Inside any text field, standard terminal editing shortcuts just work (`⌘A`/`Ctrl+A` select all, `⌘⌫`/`Ctrl+U` delete line prefix, `⌥⌫`/`Ctrl+⌫` delete word).

### Opening chats

Every new tab starts with the Projects sidebar on its left (turn this off under **Settings → Sidebar → Open in new tabs**). A tab holds at most one; `Ctrl + Alt + S` still toggles it. Clicking a chat or folder shows its card (Resume / New chat, Edit, …) at the bottom of the sidebar; **Settings → Sidebar → Selection card → Off** turns the card off, so a click only selects and the row's right-click menu offers the same actions. Opening a chat never adds a tab unless you ask for one:

- **Open in Main** (default for double-click, `Enter` and Resume) shows the chat in the main area beside the sidebar. The chat it replaces is parked, not closed, so it keeps running; opening it again swaps it back.
- **Split Right** / **Split Down** open it beside or below the current chat.
- **Open in New Tab** opens it in a new tab.

- **Fork chat** / **Fork in Split Right / Split Down / New Tab** (on OMP, Claude Code and Codex chats) branches a conversation into a brand-new session (`omp --fork <file>`, `claude --resume <id> --fork-session`, `codex fork <id>`) without touching or stopping the original — even while the original chat is actively running. Unrenamed forks automatically display a `[Fork] ` prefix in their title until renamed via `/rename`.
A chat that is already open is focused (or swapped back in) instead of starting a second copy, and shows a live status dot (`●` open in a tab, `○` parked in the deck). Project menus offer **New chat in Split Right / Split Down** too; `Ctrl + Enter` (`⌘↵`) opens in Split Right and `Ctrl + Shift + Enter` (`⇧⌘↵`) opens in a new tab. Use **Rename…** (`F2`) in any chat's menu to give it a durable custom title (with **Reset original** to revert) without modifying source transcripts.

When panes sit in Tern's parked-pane deck, a **Parked** section at the top of Projects lists them (foldable, with an inline `×` button to close all idle parked tiles at once): right-click any parked row — or any open/parked chat row anywhere in the sidebar — to **Close parked tile** / **Close chat pane** directly without opening it first (`Delete` also closes a selected parked tile, and **Projects: Close Parked Panes…** in the command palette or status bar works even while the sidebar is hidden). Working chats ask for confirmation before closing.
The header's sidebar button collapses Projects; **Settings → Sidebar → When collapsed** picks how:

- **Hide** (default) parks the sidebar, so it takes no width; its state survives and no chat is affected. `Ctrl + Alt + S`, or the **Projects** button the status line shows in tabs without a sidebar (when the status bar is on), brings it back at the left edge. While it is hidden, new tabs open without one.
- **Icon rail** narrows it to a column of buttons: expand, Projects, Search, New chat and Settings. Tern keeps every split at least a tenth of its parent, so the rail can't get narrower than that.
- **Tile** floats it as a card over the main chat's top-left corner: a large **Expand Projects** button, the three latest chats (they open in the main area as usual) and New chat. The chats behind it get the whole tab width. Expand, or `Ctrl + Alt + S`, docks it back at the left edge.

A sidebar dealt back from Tern's parked-pane tiles also returns to the left edge. A sidebar that opens, returns or docks back appears at its remembered width right away. Width, collapsed state, collapse style and unfolded projects are remembered across restarts; a sidebar collapsed to a tile is still a tile after a restart.

---

## Supported Coding Harnesses

| Agent | Fresh Launch Command | Native Resume | Multi-Folder Support | Deletion Support |
|---|---|---|---|---|
| **OMP** | `omp --cwd <primary> [--add-dir <extra>...]` | `omp --resume <path>` | ✅ Multiple `--add-dir` | Transcript + artifact unlinking |
| **Claude Code** | `claude [--add-dir <extra>...]` | `claude --resume <id>` | ✅ Multiple `--add-dir` | Exact session history filtering |
| **Codex** | `codex --cd <primary> [--add-dir <extra>...]` | `codex resume <id>` | ✅ Multiple `--add-dir` | `codex delete <uuid> --force` |
| **Pi** | `pi` | `pi --session <path>` | Single root | Exact file unlinking |
| **OpenCode** | `opencode <primary>` | `opencode --session <id>` | Single root | `opencode session delete <id>` |
| **Cursor** | `cursor-agent` / native CLI | Resumes in directory | Directory | Clean database record pruning |

---

## How It Works

- **Non-destructive**: Your original git worktrees, folders, and transcripts are never touched or rearranged.
- **Fast SQLite FTS**: Full-text indexing runs in a separate local SQLite database (`index.sqlite`), refreshing in the background every two minutes.
- **Durable Library**: Your custom projects, icons, pins, and preferences are stored in an independent `library.sqlite` so reindexing the cache never wipes your setup.

---

## Contributing

Contributions are welcome! Whether you are fixing a bug, improving Luau block performance, or adding support for a new AI coding assistant, see [CONTRIBUTING.md](CONTRIBUTING.md) for architecture notes and step-by-step instructions.

---

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes.

---

## License

[MIT](LICENSE) © 2026 thegeneralist01
