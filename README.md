# Memex

A keyboard-driven note-taking editor inspired by [Obsidian](https://obsidian.md), [Logseq](https://logseq.com), and [Emacs Org-roam](https://www.orgroam.com) — built from scratch in Rust with [gpui](https://gpui.rs) (Zed's UI framework).

Memex is a single-binary desktop app with a WYSIWYG markdown editor, PDF viewer, node graph, vim keybindings, and a wikilink-based knowledge graph — no Electron, no web stack.

## Features

### Markdown Editor
- **WYSIWYG rendering** — headings, bold, italic, strikethrough, code, blockquotes, and links are rendered inline as you type
- **Tables** — Tab/Shift-Tab auto-aligns columns and navigates between cells
- **Task lists** — clickable checkboxes toggle `- [ ]` / `- [x]` on click
- **Outline folding** — Tab folds/unfolds heading sections, Shift-Tab cycles global fold state
- **Outline navigation** — promote/demote headings, move sections up/down, jump between headings

### Wikilinks & Knowledge Graph
- **`[[wikilinks]]`** — type `[[` to trigger autocomplete via the minibuffer; click any wikilink to navigate to the target note
- **Backlinks** — `:backlinks` shows all notes that reference the current note
- **Node graph** — force-directed graph visualization of your vault's notes and their connections (like Obsidian's graph view)
  - Zoom, pan, click to select/open nodes
  - Local mode to show only connections to the current note
  - Open as a split panel alongside the editor

### PDF Viewer
- Built-in PDF rendering via [MuPDF](https://mupdf.com)
- Table of contents, bookmarks, full-text search
- Zoom, rotate, dark mode, two-page layout
- Copy page links, go-to-page
- Opens inline or in a right split panel

### Split View
- Side-by-side panels — editor + PDF, editor + graph, or any combination
- `Ctrl+H` / `Ctrl+L` to switch focus between panes
- `:split-open` (or `:vs`) to open any note or PDF in the right split

### Vim Mode
- Normal, insert, visual, and motion modes
- Leader key sequences (e.g. `space f f` for note search, `space v s` for vault switch)
- Toggleable — `:set-vim` / `:set-novim`

### Vault Management
- **Multi-vault** — switch between vaults with `:vault-switch`, open new ones with `:vault-open`
- **Recent vaults** — MRU-ordered vault list persisted across sessions
- **Per-vault config** — Rhai scripting for vault-specific keybindings and settings

### Minibuffer (Command Palette)
- Fuzzy-filtered command palette (`:` or `M-x`)
- Note search and creation (`Ctrl+P`)
- Contextual delegates: note search, vault switch, PDF TOC, wikilink autocomplete, backlinks

### Other
- **Undo/redo** with full history
- **Rhai plugin system** — extend with custom commands and keybindings
- **Configurable** — global and per-vault configuration via Rhai scripts

## Building

### Prerequisites

- Rust (stable, edition 2024)
- Linux system dependencies (for gpui):
  ```
  # Debian/Ubuntu
  sudo apt install libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libx11-xcb-dev libvulkan-dev libfontconfig-dev libfreetype-dev clang
  ```
- macOS: Xcode command line tools
- Windows: Visual Studio C++ build tools

### Build & Run

```sh
cargo build --release
./target/release/memex
```

### Run Tests

```sh
cargo test
```

## Releases

Pre-built binaries for Linux (x86_64, aarch64), macOS (x86_64, Apple Silicon), and Windows (x86_64) are available on the [Releases](https://github.com/moamenhredeen/memex/releases) page.

Releases are managed with [cargo-dist](https://opensource.axo.dev/cargo-dist/) — push a version tag to trigger a release:

```sh
git tag v0.1.0
git push --tags
```

## License

AGPL-3.0 — MuPDF (used for PDF rendering) is licensed under AGPL-3.0, which requires the entire project to be distributed under the same license.
