use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::command::Command;
use crate::editor::{EditorEvent, EditorState, EditorView};
use crate::graph::{GraphEvent, GraphState, GraphView};
use crate::keymap::{KeymapSystem, ResolvedKey, Action};
use crate::minibuffer::{Candidate, DelegateKind, Minibuffer, MinibufferAction, MinibufferVimMode};
use crate::pane::{ActiveItem, ItemAction, VimSnapshot};
use crate::pdf::{PdfState, PdfView};
use crate::state::AppState;

const MAX_RESULTS: usize = 15;

pub struct Memex {
    state: AppState,
    editor_state: Entity<EditorState>,
    editor_view: Entity<EditorView>,
    active_item: ActiveItem,
    /// Optional right split pane (e.g., graph view).
    right_pane: Option<ActiveItem>,
    /// Which pane has focus — Left is the main pane, Right is the split.
    focused_pane: PaneSide,
    keymap: KeymapSystem,
    minibuffer: Minibuffer,
    minibuffer_focus: FocusHandle,
    global_commands: Vec<Command>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaneSide {
    Left,
    Right,
}

impl Memex {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::new();

        let initial_content = if state.content.is_empty() {
            "# Welcome to Memex

Open or create a vault to get started.
Use **Ctrl+P** to search and create notes.

---

Supports *italic*, **bold**, ~~strikethrough~~, `code`, and more.

## Lists

- First item
- Second item
- Third item with **bold**

1. Ordered one
2. Ordered two

- [ ] Unchecked task
- [x] Completed task

## Table

| Name | Role | Status |
|------|------|--------|
| Alice | Dev | Active |
| Bob | Design | Away |"
                .to_string()
        } else {
            state.content.clone()
        };

        let editor_state = cx.new(|cx| EditorState::new(initial_content, window, cx));
        let editor_view = cx.new(|cx| EditorView::new(editor_state.clone(), cx));

        let editor_sub = cx.subscribe_in(
            &editor_state,
            window,
            |this, _entity, ev: &EditorEvent, window, cx| {
                match ev {
                    EditorEvent::Changed => {
                        this.state.dirty = true;
                        // Clear stale minibuffer messages on editor activity
                        this.minibuffer.message = None;
                        cx.notify();
                    }
                    EditorEvent::RequestSave => {
                        this.save(window, cx);
                        this.minibuffer.set_message("Written");
                    }
                    EditorEvent::RequestQuit => {
                        cx.quit();
                    }
                    EditorEvent::RequestOpen(path) => {
                        let path = std::path::PathBuf::from(path.clone());
                        this.open_note_by_path(path, window, cx);
                    }
                    EditorEvent::RequestVaultSwitch => {
                        this.activate_vault_switch(window, cx);
                    }
                    EditorEvent::RequestVaultOpen => {
                        this.activate_vault_open(window, cx);
                    }
                    EditorEvent::RequestNoteSearch => {
                        this.activate_note_search(window, cx);
                    }
                    EditorEvent::RequestCommand => {
                        this.activate_command_palette(window, cx);
                    }
                }
            },
        );

        let keymap = KeymapSystem::new(true);

        Self {
            state,
            editor_state: editor_state.clone(),
            editor_view: editor_view.clone(),
            active_item: ActiveItem::Editor {
                state: editor_state,
                view: editor_view,
            },
            right_pane: None,
            focused_pane: PaneSide::Left,
            keymap,
            minibuffer: Minibuffer::new(),
            minibuffer_focus: cx.focus_handle(),
            global_commands: Self::global_commands(),
            _subscriptions: vec![editor_sub],
        }
    }

    /// Create a read-only snapshot of keymap state for item dispatch.
    fn vim_snapshot(&self) -> VimSnapshot {
        VimSnapshot {
            vim_enabled: self.keymap.vim_enabled,
            visual_active: self.keymap.is_visual_active(),
            insert_active: self.keymap.is_insert_active(),
        }
    }

    /// Sync vim flags from keymap to editor state.
    fn sync_editor_vim_flags(&self, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        let insert = self.keymap.is_insert_active();
        self.editor_state.update(cx, |state, _cx| {
            state.sync_vim_flags(vim, insert);
        });
    }

    fn activate_note_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        self.minibuffer.activate(DelegateKind::NoteSearch, "Find note:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_vault_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        self.minibuffer.activate(DelegateKind::VaultSwitch, "Switch vault:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_vault_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        self.minibuffer.activate(DelegateKind::VaultOpen, "Open vault:", vim);
        // Seed with home directory
        if let Some(home) = dirs::home_dir() {
            let seed = format!("{}/", home.to_string_lossy());
            self.minibuffer.input = seed.clone();
            self.minibuffer.cursor = seed.len();
        }
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        let prompt = if vim { ":" } else { "M-x" };
        self.minibuffer.activate(DelegateKind::Command, prompt, vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn dismiss_minibuffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer.dismiss();
        self.active_item.focus(window, cx);
        cx.notify();
    }

    /// Route a key press through the unified minibuffer and handle the resulting action.
    fn handle_minibuffer_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let candidates = self.get_candidates(cx);
        let action = self.minibuffer.handle_key(key, ctrl, shift, candidates.len());

        match action {
            MinibufferAction::Updated => {
                // Notify active item of input changes for item-owned delegates
                if let DelegateKind::Item(ref id) = self.minibuffer.delegate_kind {
                    let input = self.minibuffer.input.clone();
                    self.active_item.on_input_changed(id, &input, cx);
                }
                cx.notify();
            }
            MinibufferAction::Confirm => {
                let candidates = self.get_candidates(cx);
                self.handle_confirm(candidates, window, cx);
            }
            MinibufferAction::Complete => {
                let candidates = self.get_candidates(cx);
                if let Some(c) = candidates.get(self.minibuffer.selected) {
                    if self.minibuffer.delegate_kind == DelegateKind::VaultOpen {
                        // Tab descends into the selected directory
                        let path = format!("{}/", c.data);
                        self.minibuffer.input = path.clone();
                        self.minibuffer.cursor = path.len();
                    } else {
                        // Default: insert candidate label (vertico-insert)
                        self.minibuffer.input = c.label.clone();
                        self.minibuffer.cursor = self.minibuffer.input.len();
                    }
                    self.minibuffer.selected = 0;
                }
                cx.notify();
            }
            MinibufferAction::Dismiss => {
                self.dismiss_minibuffer(window, cx);
            }
        }
    }

    /// Get candidates for the current delegate kind.
    fn get_candidates(&self, cx: &App) -> Vec<Candidate> {
        match &self.minibuffer.delegate_kind {
            DelegateKind::Command => {
                self.palette_candidates(&self.minibuffer.input)
            }
            DelegateKind::NoteSearch => {
                self.get_note_candidates()
            }
            DelegateKind::VaultSwitch => {
                self.get_vault_switch_candidates()
            }
            DelegateKind::VaultOpen => {
                self.get_vault_open_candidates()
            }
            DelegateKind::Item(id) => {
                self.active_item.get_candidates(id, &self.minibuffer.input, cx)
            }
        }
    }

    /// Handle confirm action — dispatched by delegate kind.
    fn handle_confirm(
        &mut self,
        candidates: Vec<Candidate>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.minibuffer.selected;
        let input = self.minibuffer.input.clone();
        let kind = self.minibuffer.delegate_kind.clone();

        match kind {
            DelegateKind::Command => {
                if let Some(candidate) = candidates.get(selected) {
                    let cmd_id = candidate.data.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.execute_command(&cmd_id, &input, 1, window, cx);
                } else if !input.is_empty() {
                    // Try executing raw input as ex command
                    self.dismiss_minibuffer(window, cx);
                    let vim = self.vim_snapshot();
                    let editor = self.editor_state.clone();
                    let actions = editor.update(cx, |state, cx| {
                        state.execute_ex_command(&input, vim, window, cx)
                    });
                    self.process_item_actions(actions, window, cx);
                }
            }
            DelegateKind::NoteSearch => {
                if let Some(candidate) = candidates.get(selected) {
                    if candidate.is_action {
                        // "Create new" action
                        let title = input.clone();
                        self.dismiss_minibuffer(window, cx);
                        self.create_note_by_title(&title, window, cx);
                    } else {
                        let path = std::path::PathBuf::from(&candidate.data);
                        self.dismiss_minibuffer(window, cx);
                        self.open_note_by_path(path, window, cx);
                    }
                } else if !input.is_empty() {
                    let title = input.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.create_note_by_title(&title, window, cx);
                }
            }
            DelegateKind::VaultSwitch => {
                if let Some(candidate) = candidates.get(selected) {
                    let path = std::path::PathBuf::from(&candidate.data);
                    self.dismiss_minibuffer(window, cx);
                    self.open_vault_by_path(path, window, cx);
                    self.activate_note_search(window, cx);
                }
            }
            DelegateKind::VaultOpen => {
                if let Some(candidate) = candidates.get(selected) {
                    let path = std::path::PathBuf::from(&candidate.data);
                    if path.is_dir() {
                        self.dismiss_minibuffer(window, cx);
                        self.open_vault_by_path(path, window, cx);
                        self.activate_note_search(window, cx);
                    } else {
                        self.minibuffer.set_message(format!(
                            "Not a directory: {}",
                            candidate.data
                        ));
                    }
                } else if !input.is_empty() {
                    let path = std::path::PathBuf::from(&input);
                    if path.is_dir() {
                        self.dismiss_minibuffer(window, cx);
                        self.open_vault_by_path(path, window, cx);
                        self.activate_note_search(window, cx);
                    } else {
                        self.minibuffer.set_message(format!("Not a directory: {}", input));
                    }
                }
            }
            DelegateKind::Item(ref id) => {
                let candidate = candidates.get(selected);
                let id = id.clone();
                let actions = self.active_item.handle_confirm(&id, &input, candidate, cx);
                self.process_item_actions(actions, window, cx);
            }
        }
    }

    /// Process side-effect actions returned by an item.
    fn process_item_actions(
        &mut self,
        actions: Vec<ItemAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for action in actions {
            match action {
                ItemAction::SetMessage(msg) => {
                    if msg == "__close_split__" {
                        self.close_split(window, cx);
                    } else {
                        self.minibuffer.set_message(msg);
                    }
                }
                ItemAction::ActivateDelegate { id, prompt, highlight_input: _ } => {
                    let vim = self.keymap.vim_enabled;
                    self.minibuffer.activate(DelegateKind::Item(id), &prompt, vim);
                    self.minibuffer_focus.focus(window);
                }
                ItemAction::Dismiss => {
                    self.dismiss_minibuffer(window, cx);
                }
                ItemAction::WriteClipboard(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                ItemAction::ActivateLayer(layer_id) => {
                    self.keymap.stack.activate_layer(layer_id);
                    self.sync_editor_vim_flags(cx);
                }
                ItemAction::SetVimEnabled(enabled) => {
                    self.keymap.set_vim_enabled(enabled);
                    self.sync_editor_vim_flags(cx);
                }
                ItemAction::SyncVimFlags => {
                    self.sync_editor_vim_flags(cx);
                }
            }
        }
        cx.notify();
    }

    /// Execute a command by registry id.
    fn execute_command(
        &mut self,
        cmd_id: &str,
        raw_input: &str,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd_id {
            // App-level commands
            "command-palette" => {
                self.activate_command_palette(window, cx);
            }
            "find-note" => {
                self.activate_note_search(window, cx);
            }
            "write" => {
                self.save(window, cx);
                self.minibuffer.set_message("Written");
            }
            "quit" => {
                cx.quit();
            }
            "wq" => {
                self.save(window, cx);
                cx.quit();
            }
            "vault-switch" => {
                self.activate_vault_switch(window, cx);
            }
            "vault-open" => {
                self.activate_vault_open(window, cx);
            }
            "open-graph" => {
                self.open_graph(window, cx);
            }
            "close-split" | "close-graph" => {
                self.close_split(window, cx);
            }
            "notes" => {
                self.activate_note_search(window, cx);
            }
            "edit" => {
                // Extract path from raw input (e.g., "edit /path/to/file")
                let path = raw_input
                    .strip_prefix("edit ")
                    .or_else(|| raw_input.strip_prefix("e "))
                    .unwrap_or("")
                    .trim();
                if path.is_empty() {
                    self.minibuffer.set_message("Specify a file path");
                } else {
                    let p = std::path::PathBuf::from(path);
                    self.open_note_by_path(p, window, cx);
                }
            }
            _ => {
                // Dispatch to focused pane's item
                let vw: f32 = window.viewport_size().width.into();
                let vh: f32 = window.viewport_size().height.into();
                let vim = self.vim_snapshot();

                // Try right pane first if it's focused
                if self.focused_pane == PaneSide::Right {
                    if let Some(ref right) = self.right_pane {
                        let actions = right.execute_command(cmd_id, (vw, vh), vim, cx);
                        if !actions.is_empty() {
                            self.process_item_actions(actions, window, cx);
                            cx.notify();
                            return;
                        }
                    }
                }

                let actions = self.active_item.execute_command(cmd_id, (vw, vh), vim, cx);
                if !actions.is_empty() {
                    self.process_item_actions(actions, window, cx);
                } else if self.active_item.is_editor() && self.focused_pane == PaneSide::Left {
                    // Editor commands that need window access (editing, motions, etc.)
                    let vim = self.vim_snapshot();
                    let editor = self.editor_state.clone();
                    let item_actions = editor.update(cx, |state, ecx| {
                        state.execute_command_by_id(cmd_id, count, vim, window, ecx)
                    });
                    self.process_item_actions(item_actions, window, cx);
                    if let Some(msg) = self.editor_state.read(cx).status_message.clone() {
                        self.minibuffer.set_message(msg);
                    }
                }
            }
        }
        cx.notify();
    }

    /// Build note candidates from current vault.
    fn get_note_candidates(&self) -> Vec<Candidate> {
        let results = self.search_notes(&self.minibuffer.input);
        let has_exact = results
            .iter()
            .any(|(t, _)| t.to_lowercase() == self.minibuffer.input.to_lowercase());
        let show_create = !self.minibuffer.input.is_empty() && !has_exact;

        let mut candidates: Vec<Candidate> = results
            .into_iter()
            .map(|(title, path)| {
                let is_pdf = path.extension().and_then(|e| e.to_str()) == Some("pdf");
                let label = if is_pdf {
                    format!("📄 {}", title)
                } else {
                    title
                };
                Candidate {
                    label,
                    detail: None,
                    is_action: false,
                    data: path.to_string_lossy().to_string(),
                }
            })
            .collect();

        if show_create {
            candidates.push(Candidate {
                label: format!("+ Create \"{}\"", self.minibuffer.input),
                detail: None,
                is_action: true,
                data: self.minibuffer.input.clone(),
            });
        }

        candidates
    }

    /// Build candidates for `:vault-switch` — MRU-ordered recent vaults.
    fn get_vault_switch_candidates(&self) -> Vec<Candidate> {
        let current_path = self.state.vault.as_ref().map(|v| v.path.as_path());
        let recent = self.state.registry.recent_vaults(current_path);
        let query = &self.minibuffer.input;

        let entries: Vec<(&str, &str)> = recent
            .iter()
            .map(|entry| {
                let name = std::path::Path::new(&entry.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("vault");
                (name, entry.path.as_str())
            })
            .collect();

        if query.is_empty() {
            return entries
                .into_iter()
                .take(MAX_RESULTS)
                .map(|(name, path)| Candidate {
                    label: name.to_string(),
                    detail: Some(path.to_string()),
                    is_action: false,
                    data: path.to_string(),
                })
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &str, &str)> = entries
            .into_iter()
            .filter_map(|(name, path)| {
                let name_score = matcher.fuzzy_match(name, query);
                let path_score = matcher.fuzzy_match(path, query);
                let best = name_score.max(path_score);
                best.map(|score| (score, name, path))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, name, path)| Candidate {
                label: name.to_string(),
                detail: Some(path.to_string()),
                is_action: false,
                data: path.to_string(),
            })
            .collect()
    }

    /// Build candidates for `:vault-open` — live directory completion.
    fn get_vault_open_candidates(&self) -> Vec<Candidate> {
        let input = &self.minibuffer.input;
        if input.is_empty() {
            return Vec::new();
        }

        let expanded = if input.starts_with('~') {
            let rest = input.get(1..).unwrap_or("");
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            dirs::home_dir()
                .map(|h| if rest.is_empty() { h } else { h.join(rest) })
                .unwrap_or_else(|| std::path::PathBuf::from(input))
        } else {
            std::path::PathBuf::from(input)
        };

        let (parent, prefix) = if expanded.is_dir() && input.ends_with('/') {
            (expanded.clone(), String::new())
        } else {
            let parent = expanded
                .parent()
                .unwrap_or(std::path::Path::new("/"))
                .to_path_buf();
            let prefix = expanded
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            (parent, prefix)
        };

        let mut candidates = Vec::new();

        if let Ok(read_dir) = std::fs::read_dir(&parent) {
            let mut entries: Vec<std::path::PathBuf> = read_dir
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    if !p.is_dir() {
                        return false;
                    }
                    let name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    if name.starts_with('.') {
                        return false;
                    }
                    if !prefix.is_empty() {
                        name.to_lowercase().starts_with(&prefix)
                    } else {
                        true
                    }
                })
                .collect();

            entries.sort_by(|a, b| {
                a.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(
                        &b.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_lowercase(),
                    )
            });

            for path in entries.into_iter().take(MAX_RESULTS) {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                // Mark registered vaults
                let is_registered = self
                    .state
                    .registry
                    .vault_paths()
                    .iter()
                    .any(|vp| *vp == path);
                let suffix = if is_registered { "  ★" } else { "" };
                candidates.push(Candidate {
                    label: format!("{}/{}",  name, suffix),
                    detail: Some(path.to_string_lossy().to_string()),
                    is_action: false,
                    data: path.to_string_lossy().to_string(),
                });
            }
        }

        candidates
    }

    /// Global commands available in every item context.
    fn global_commands() -> Vec<Command> {
        vec![
            Command { id: "write", name: "Save", description: "Save current note to disk", aliases: &["w", "save"], binding: Some(":w") },
            Command { id: "quit", name: "Quit", description: "Quit memex", aliases: &["q", "exit"], binding: Some(":q") },
            Command { id: "wq", name: "Save and Quit", description: "Save current note and quit", aliases: &["x"], binding: Some(":wq") },
            Command { id: "vault-switch", name: "Switch Vault", description: "Switch to a recent vault", aliases: &["vault", "vaults", "switch-vault"], binding: Some(":vault-switch") },
            Command { id: "vault-open", name: "Open Vault", description: "Browse filesystem to open a vault", aliases: &["open-vault"], binding: Some(":vault-open") },
            Command { id: "notes", name: "Find Note", description: "Search and open a note in current vault", aliases: &["find-note", "find", "note"], binding: Some("Ctrl+P") },
            Command { id: "edit", name: "Edit File", description: "Open a file by path", aliases: &["e", "open"], binding: Some(":e <path>") },
            Command { id: "set", name: "Set Option", description: "Set an editor option", aliases: &[], binding: Some(":set <option>") },
            Command { id: "set-vim", name: "Enable Vim Mode", description: "Enable vim keybindings", aliases: &[], binding: None },
            Command { id: "set-novim", name: "Disable Vim Mode", description: "Disable vim keybindings", aliases: &[], binding: None },
            Command { id: "nohlsearch", name: "Clear Search Highlighting", description: "Remove search result highlighting", aliases: &["noh"], binding: Some(":noh") },
            Command { id: "toggle-vim", name: "Toggle Vim Mode", description: "Toggle vim mode on/off", aliases: &[], binding: None },
            Command { id: "open-graph", name: "Open Graph", description: "Open the vault graph in a split panel", aliases: &["graph"], binding: None },
            Command { id: "close-split", name: "Close Split", description: "Close the right split panel", aliases: &[], binding: None },
        ]
    }

    /// Fuzzy-filter commands for the palette: item commands + global commands.
    fn palette_candidates(&self, query: &str) -> Vec<Candidate> {
        let item_cmds = self.active_item.commands();
        let global_cmds = &self.global_commands;

        let all_cmds: Vec<&Command> = item_cmds.iter().chain(global_cmds.iter()).collect();

        if query.is_empty() {
            return all_cmds.iter()
                .take(MAX_RESULTS)
                .map(|c| command_to_candidate(c))
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &Command)> = all_cmds.iter()
            .filter_map(|c| {
                let scores = [
                    matcher.fuzzy_match(c.name, query),
                    matcher.fuzzy_match(c.description, query),
                    matcher.fuzzy_match(c.id, query),
                ];
                let alias_score = c.aliases.iter()
                    .filter_map(|a| matcher.fuzzy_match(a, query))
                    .max();
                let best = scores.into_iter().flatten().chain(alias_score).max();
                best.map(|score| (score, *c))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter()
            .take(MAX_RESULTS)
            .map(|(_, c)| command_to_candidate(c))
            .collect()
    }

    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor_state.read(cx).content();
        self.state.content = text;
        if let Err(e) = self.state.save() {
            eprintln!("save error: {}", e);
        }
        cx.notify();
    }

    fn open_note_by_path(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Check if this is a PDF
        if path.extension().and_then(|e| e.to_str()) == Some("pdf") {
            self.open_pdf(path, window, cx);
            return;
        }

        if let Err(e) = self.state.open_note(path) {
            eprintln!("failed to open note: {}", e);
            return;
        }
        let content = self.state.content.clone();
        self.editor_state.update(cx, |state, cx| {
            state.set_content(content, window, cx);
        });
        // Switch to editor item
        self.switch_to_item(ActiveItem::Editor {
            state: self.editor_state.clone(),
            view: self.editor_view.clone(),
        });
        // Re-activate vim layers if vim is enabled
        if self.keymap.vim_enabled {
            self.keymap.stack.activate_layer("vim:normal");
            self.keymap.stack.activate_layer("vim:motion");
        }
        cx.notify();
    }

    fn open_pdf(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Validate the PDF can be loaded before creating the entity
        let raw_bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.minibuffer.set_message(format!("Failed to read PDF: {}", e));
                cx.notify();
                return;
            }
        };
        if let Err(e) = mupdf::Document::from_bytes(&raw_bytes, "") {
            self.minibuffer.set_message(format!("Invalid PDF: {:?}", e));
            cx.notify();
            return;
        }

        self.state.current_note = Some(path.clone());
        let pdf_state = cx.new(|cx| PdfState::new(&path, cx).expect("PDF already validated"));
        pdf_state.update(cx, |s, cx| s.extract_text_cache(cx));
        let pdf_view = cx.new(|cx| PdfView::new(pdf_state.clone(), cx));
        pdf_state.read(cx).focus(window);
        // Observe PdfState so Memex re-renders when async operations complete
        let pdf_sub = cx.observe(&pdf_state, |_, _, cx| cx.notify());
        self._subscriptions.push(pdf_sub);
        // Switch to PDF item
        self.switch_to_item(ActiveItem::Pdf {
            state: pdf_state,
            view: pdf_view,
        });
        cx.notify();
    }

    fn open_graph(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Build graph from vault
        let graph_state = cx.new(|cx| {
            let mut gs = GraphState::new(cx);
            if let Some(vault) = &self.state.vault {
                gs.build_from_vault(&vault.path, &vault.notes);
            }
            // Set local root to current note if one is open
            if let Some(ref current) = self.state.current_note {
                gs.set_local_root_by_path(current);
            }
            gs
        });
        let graph_view = cx.new(|cx| GraphView::new(graph_state.clone(), cx));

        // Subscribe to graph events (node clicks)
        let graph_sub = cx.subscribe_in(
            &graph_state,
            window,
            |this, _entity, ev: &GraphEvent, window, cx| {
                match ev {
                    GraphEvent::OpenNote(path) => {
                        this.open_note_by_path(path.clone(), window, cx);
                    }
                }
            },
        );
        self._subscriptions.push(graph_sub);

        // Observe so we re-render on physics ticks
        let obs = cx.observe(&graph_state, |_, _, cx| cx.notify());
        self._subscriptions.push(obs);

        let graph_item = ActiveItem::Graph {
            state: graph_state,
            view: graph_view,
        };

        // Open in right split
        // Deactivate right pane's old layers if it had one
        if let Some(ref old) = self.right_pane {
            for layer in old.keymap_layers() {
                self.keymap.stack.deactivate_layer(layer);
            }
        }
        self.right_pane = Some(graph_item);
        self.focused_pane = PaneSide::Right;

        // Activate graph layers
        // Deactivate left pane layers first
        for layer in self.active_item.keymap_layers() {
            self.keymap.stack.deactivate_layer(layer);
        }
        self.right_pane.as_ref().unwrap().focus(window, cx);
        for layer in self.right_pane.as_ref().unwrap().keymap_layers() {
            self.keymap.stack.activate_layer(layer);
        }

        cx.notify();
    }

    fn close_split(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ref right) = self.right_pane {
            // Deactivate right pane layers
            for layer in right.keymap_layers() {
                self.keymap.stack.deactivate_layer(layer);
            }
        }
        self.right_pane = None;
        self.focused_pane = PaneSide::Left;

        // Re-activate left pane layers
        for layer in self.active_item.keymap_layers() {
            self.keymap.stack.activate_layer(layer);
        }
        self.active_item.focus(window, cx);
        self.sync_editor_vim_flags(cx);
        cx.notify();
    }

    fn focus_pane(
        &mut self,
        side: PaneSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if side == self.focused_pane {
            return;
        }
        // Deactivate old focused pane layers
        let old_item = match self.focused_pane {
            PaneSide::Left => Some(&self.active_item),
            PaneSide::Right => self.right_pane.as_ref(),
        };
        if let Some(item) = old_item {
            for layer in item.keymap_layers() {
                self.keymap.stack.deactivate_layer(layer);
            }
        }
        self.focused_pane = side;
        // Activate new focused pane layers and focus
        let new_item = match side {
            PaneSide::Left => Some(&self.active_item),
            PaneSide::Right => self.right_pane.as_ref(),
        };
        if let Some(item) = new_item {
            for layer in item.keymap_layers() {
                self.keymap.stack.activate_layer(layer);
            }
            item.focus(window, cx);
        }
        if side == PaneSide::Left {
            self.sync_editor_vim_flags(cx);
        }
        cx.notify();
    }

    /// Switch the active item, swapping keymap layers accordingly.
    fn switch_to_item(&mut self, new_item: ActiveItem) {
        // Deactivate old item's layers
        for layer in self.active_item.keymap_layers() {
            self.keymap.stack.deactivate_layer(layer);
        }
        // Deactivate layers the new item wants off
        for layer in new_item.deactivate_layers() {
            self.keymap.stack.deactivate_layer(layer);
        }
        // Activate new item's layers
        for layer in new_item.keymap_layers() {
            self.keymap.stack.activate_layer(layer);
        }
        self.active_item = new_item;
    }

    fn create_note_by_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.state.create_note(title) {
            Ok(_) => {
                let content = self.state.content.clone();
                self.editor_state.update(cx, |state, cx| {
                    state.set_content(content, window, cx);
                });
            }
            Err(e) => eprintln!("failed to create note: {}", e),
        }
        cx.notify();
    }

    fn open_vault_by_path(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.state.open_vault(path) {
            eprintln!("failed to open vault: {}", e);
            return;
        }
        let content = self.state.content.clone();
        self.editor_state.update(cx, |state, cx| {
            state.set_content(content, window, cx);
        });
        cx.notify();
    }
    fn search_notes(&self, query: &str) -> Vec<(String, std::path::PathBuf)> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return Vec::new(),
        };

        let titles = vault.note_titles();

        if query.is_empty() {
            return titles.into_iter().take(MAX_RESULTS).collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, String, std::path::PathBuf)> = titles
            .into_iter()
            .filter_map(|(title, path)| {
                matcher
                    .fuzzy_match(&title, query)
                    .map(|score| (score, title, path))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, title, path)| (title, path))
            .collect()
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = {
            let t = self.state.current_title();
            if t.is_empty() { "Memex".to_string() } else { t }
        };
        let dirty = self.state.dirty;
        let title_text = if dirty {
            format!("{} ●", title)
        } else {
            title
        };

        h_flex()
            .id("title-bar")
            .w_full()
            .items_center()
            .justify_between()
            // Drag to move window
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _e: &MouseDownEvent, window, _cx| {
                    window.start_window_move();
                }),
            )
            // Left: spacer for symmetry
            .child(div().w(px(72.)))
            // Center: title
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0x657B83))
                            .child(title_text),
                    ),
            )
            // Right: window controls
            .child(
                h_flex()
                    .gap(px(0.))
                    .child(self.title_bar_close_button(cx)),
            )
    }

    fn title_bar_close_button(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("close-btn")
            .w(px(24.))
            .h(px(24.))
            .m_2()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .bg(rgba(0x00000010))
            .cursor_pointer()
            .hover(|s| s
                .text_color(rgba(0x00000010))
                .bg(rgba(0xFF000040))
            )
            .on_mouse_down(MouseButton::Left, |_e, _w, cx| {
                cx.stop_propagation();
            })
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.quit();
            }))
            .child("✕")
    }

    fn render_mode_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let es = self.editor_state.read(cx);
        let vim_enabled = self.keymap.vim_enabled;
        let vim_state = self.keymap.active_vim_state().map(|s| s.to_string());
        let _ = es;

        let vault_name = self.state.vault_name();
        let note_title = self.state.current_title();
        let dirty = self.state.dirty;
        let dirty_indicator = if dirty { " ●" } else { "" };

        // Position info depends on focused pane
        let focused_item = match self.focused_pane {
            PaneSide::Left => &self.active_item,
            PaneSide::Right => self.right_pane.as_ref().unwrap_or(&self.active_item),
        };
        let position_text = focused_item.position_text(600.0, cx);

        // Mode badge (left) — show focused item's badge
        let show_non_editor = focused_item.is_pdf() || focused_item.is_graph();
        let mode_badge = if show_non_editor {
            let (label, color) = focused_item.mode_badge();
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(rgb(color))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xFDF6E3))
                        .child(label),
                )
        } else if vim_enabled {
            let (label, bg) = match vim_state.as_deref() {
                Some("NORMAL") => ("NOR", rgb(0x268BD2)),   // blue
                Some("INSERT") => ("INS", rgb(0x859900)),   // green
                Some("VISUAL") => ("VIS", rgb(0x6C71C4)),   // violet
                Some("V-LINE") => ("V-L", rgb(0x6C71C4)),
                _ => ("NOR", rgb(0x268BD2)),
            };
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(bg)
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xFDF6E3))  // base3 on badge
                        .child(label),
                )
        } else {
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(rgb(0x859900))  // green for EDT badge
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xFDF6E3))  // base3 on badge
                        .child("EDT"),
                )
        };

        h_flex()
            .w_full()
            .h(px(24.))
            .bg(rgb(0xEEE8D5))  // solarized base2
            .items_center()
            .gap(px(0.))
            .child(mode_badge)
            // Vault + file
            .child(
                div()
                    .px(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0x586E75))  // base01
                            .child(format!(
                                " {} › {}{}",
                                vault_name, note_title, dirty_indicator
                            )),
                    ),
            )
            // Spacer
            .child(div().flex_1())
            // Position (always L:C)
            .child(
                div()
                    .px(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0x93A1A1))  // base1
                            .child(position_text),
                    ),
            )
    }

    /// Render the minibuffer area — unified, single rendering path.
    /// Always visible like emacs: shows echo area messages when idle,
    /// prompt + input + vertico candidates when active.
    fn render_minibuffer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = v_flex()
            .w_full()
            .bg(rgb(0xFDF6E3)); // solarized base3

        if !self.minibuffer.active {
            // Idle — echo area: show message or status from editor
            let msg = self
                .minibuffer
                .message
                .clone()
                .or_else(|| self.editor_state.read(cx).status_message.clone())
                .unwrap_or_default();
            return base.child(
                h_flex()
                    .w_full()
                    .h(px(22.))
                    .px(px(8.))
                    .py(px(3.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0x93A1A1)) // base1 — idle message
                            .child(msg),
                    ),
            );
        }

        // Active — prompt + input with cursor + vertico candidate list
        let candidates = self.get_candidates(cx);
        let selected = self.minibuffer.selected;
        let (before_cursor, after_cursor) = self.minibuffer.input_parts();

        // Cursor character: block for vim normal, line for insert
        let cursor_char = match self.minibuffer.vim_mode {
            MinibufferVimMode::Normal => "█",
            MinibufferVimMode::Insert => "│",
        };

        // Fixed candidate area: 10 visible rows (each ~20px)
        let max_visible = 10usize;
        let candidate_area_h = px((max_visible as f32) * 20.0);

        // Compute scroll window so selected item stays visible
        let scroll_top = if candidates.len() <= max_visible {
            0
        } else if selected < max_visible / 2 {
            0
        } else if selected + max_visible / 2 >= candidates.len() {
            candidates.len().saturating_sub(max_visible)
        } else {
            selected - max_visible / 2
        };
        let visible_end = (scroll_top + max_visible).min(candidates.len());

        // Build candidate list (only visible window)
        let mut items = v_flex().w_full().h(candidate_area_h);
        for i in scroll_top..visible_end {
            let candidate = &candidates[i];
            let is_selected = i == selected;
            let bg_color = if is_selected {
                rgb(0xEEE8D5) // base2 — selected
            } else {
                rgb(0xFDF6E3) // base3 — default
            };

            let text_color = if candidate.is_action {
                rgb(0x859900) // green — create/action items
            } else if is_selected {
                rgb(0x073642) // base03 — selected text
            } else {
                rgb(0x657B83) // base00 — normal text
            };

            let label_element = if matches!(self.minibuffer.delegate_kind, DelegateKind::Item(ref id) if self.active_item.highlight_input(id))
                && !self.minibuffer.input.is_empty()
            {
                // Highlight the search term within the candidate label
                render_highlighted_label(
                    &candidate.label,
                    &self.minibuffer.input,
                    text_color,
                )
            } else {
                div()
                    .text_size(px(13.))
                    .text_color(text_color)
                    .child(candidate.label.clone())
            };

            let mut row = h_flex().gap(px(8.)).child(label_element);

            if let Some(detail) = &candidate.detail {
                row = row.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(0x93A1A1)) // base1 — detail/description
                        .child(detail.clone()),
                );
            }

            items = items.child(
                div()
                    .id(ElementId::Name(format!("mb-item-{}", i).into()))
                    .w_full()
                    .px(px(8.))
                    .py(px(2.))
                    .bg(bg_color)
                    .child(row),
            );
        }

        base.border_t_1()
            .border_color(rgb(0xD3CBB8))
            .track_focus(&self.minibuffer_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let ctrl = e.keystroke.modifiers.control;
                let shift = e.keystroke.modifiers.shift;
                this.handle_minibuffer_key(key, ctrl, shift, window, cx);
            }))
            // Prompt line with cursor-aware input
            .child(
                h_flex()
                    .w_full()
                    .px(px(8.))
                    .py(px(3.))
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0x268BD2)) // blue — prompt
                            .child(self.minibuffer.prompt.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0x073642)) // base03 — input text
                            .child(format!(
                                "{}{}{}",
                                before_cursor, cursor_char, after_cursor
                            )),
                    ),
            )
            // Vertico candidate list
            .child(items)
    }

}

impl Render for Memex {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let minibuffer_active = self.minibuffer.active;

        let mut root = v_flex()
            .id("memex-root")
            .size_full()
            .bg(rgb(0xFDF6E3))  // solarized base3
            .font_family("FiraCode Nerd Font")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                // Don't intercept keys when minibuffer is active
                if this.minibuffer.active {
                    return;
                }

                let key = e.keystroke.key.as_str();
                let ctrl = e.keystroke.modifiers.control;
                let shift = e.keystroke.modifiers.shift;
                let alt = e.keystroke.modifiers.alt;

                // Global shortcuts (bypass keymap system)
                if ctrl && key == "p" {
                    this.activate_note_search(window, cx);
                    return;
                }
                if ctrl && key == "s" {
                    this.save(window, cx);
                    return;
                }
                if alt && key == "x" {
                    this.activate_command_palette(window, cx);
                    return;
                }
                // Split focus switching: Ctrl+W h/l
                if ctrl && key == "w" {
                    // Ctrl+W prefix — mark pending, next key will be h or l
                    this.editor_state.update(cx, |state, _cx| {
                        state.suppress_next_input = true;
                    });
                    // We use the keymap system for ctrl-w bindings, so fall through
                }
                // Quick split navigation: Ctrl+H / Ctrl+L to switch panes
                if ctrl && key == "h" && this.right_pane.is_some() {
                    this.focus_pane(PaneSide::Left, window, cx);
                    return;
                }
                if ctrl && key == "l" && this.right_pane.is_some() {
                    this.focus_pane(PaneSide::Right, window, cx);
                    return;
                }

                // Central key dispatch through the keymap system
                let resolved = this.keymap.resolve_key(key, ctrl, shift, alt);
                match resolved {
                    ResolvedKey::Action(action, count) => {
                        match &action {
                            Action::Command(cmd_id) => {
                                this.execute_command(cmd_id, "", count, window, cx);
                            }
                            Action::ActivateLayer(layer_id) => {
                                this.keymap.stack.activate_layer(layer_id);
                                if this.active_item.is_editor() {
                                    this.editor_state.update(cx, |state, cx| {
                                        state.on_layer_activated(layer_id, cx);
                                    });
                                }
                                this.sync_editor_vim_flags(cx);
                                cx.notify();
                            }
                            _ => {
                                // Motion, Operator, SelfInsert, etc. — editor-specific
                                if this.active_item.is_editor() {
                                    let vim = this.vim_snapshot();
                                    let editor = this.editor_state.clone();
                                    let item_actions = editor.update(cx, |state, ecx| {
                                        state.process_vim_action(
                                            action, key, count, vim,
                                            &mut this.keymap.stack,
                                            window, ecx,
                                        )
                                    });
                                    this.process_item_actions(item_actions, window, cx);
                                }
                            }
                        }
                    }
                    ResolvedKey::TransientCapture { transient_id, ch, count } => {
                        if this.active_item.is_editor() {
                            this.editor_state.update(cx, |state, ecx| {
                                state.handle_transient_capture(&transient_id, ch, count, window, ecx);
                            });
                        }
                    }
                    ResolvedKey::Pending => {
                        if this.active_item.is_editor() {
                            this.editor_state.update(cx, |state, _cx| {
                                state.suppress_next_input = true;
                            });
                        }
                    }
                    ResolvedKey::Unhandled => {
                        // Key not bound — in editor insert mode, let OS handle it
                        // In normal mode or PDF mode, nothing to do
                    }
                }
            }))
            // Custom title bar with drag + window controls
            .child(self.render_title_bar(cx))
            // Main content area: active item's view + optional right split
            .child({
                let left_view = self.active_item.view_element();
                let has_right = self.right_pane.is_some();
                let focused = self.focused_pane;

                if has_right {
                    let right_view = self.right_pane.as_ref().unwrap().view_element();

                    h_flex()
                        .flex_1()
                        .w_full()
                        .overflow_hidden()
                        // Left pane
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .overflow_hidden()
                                .child(left_view),
                        )
                        // Divider
                        .child(
                            div()
                                .w(px(1.))
                                .h_full()
                                .bg(rgba(0x00000010)), // solarized base1
                        )
                        // Right pane
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .overflow_hidden()
                                .child(right_view),
                        )
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .w_full()
                        .overflow_hidden()
                        .child(left_view)
                        .into_any_element()
                }
            })
            // Mode line (always visible, like emacs mode-line)
            .child(self.render_mode_line(cx))
            // Minibuffer area (below mode line, like emacs)
            .child(self.render_minibuffer(cx));

        // Dim overlay when minibuffer is active
        if minibuffer_active {
            root = root.child(
                div()
                    .id("minibuffer-overlay")
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .w_full()
                    .h_full()
                    .bg(rgba(0x00000000))  // transparent — click-to-dismiss only, no dimming
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                            this.dismiss_minibuffer(window, cx);
                        }),
                    ),
            );
        }

        root
    }
}

/// Render a label with the search term highlighted in a distinct color.
fn render_highlighted_label(
    label: &str,
    query: &str,
    base_color: impl Into<Hsla> + Copy,
) -> Div {
    let highlight_color = rgb(0xCB4B16); // solarized orange
    let base_hsla: Hsla = base_color.into();
    let label_lower = label.to_lowercase();
    let query_lower = query.to_lowercase();

    let mut container = div().text_size(px(13.)).flex().flex_row();
    let mut pos = 0;

    while pos < label.len() {
        if let Some(match_start) = label_lower[pos..].find(&query_lower) {
            let abs_start = pos + match_start;
            let abs_end = abs_start + query_lower.len();
            // Snap to char boundaries
            let abs_start = snap_to_char(label, abs_start, false);
            let abs_end = snap_to_char(label, abs_end, true);

            // Text before match
            if abs_start > pos {
                container = container.child(
                    div().text_color(base_hsla).child(label[pos..abs_start].to_string()),
                );
            }
            // Highlighted match
            container = container.child(
                div()
                    .text_color(highlight_color)
                    .font_weight(FontWeight::BOLD)
                    .child(label[abs_start..abs_end].to_string()),
            );
            pos = abs_end;
        } else {
            // Remaining text after last match
            container = container.child(
                div().text_color(base_hsla).child(label[pos..].to_string()),
            );
            break;
        }
    }

    container
}

/// Snap byte index to a valid char boundary.
fn snap_to_char(s: &str, idx: usize, ceil: bool) -> usize {
    if idx >= s.len() { return s.len(); }
    if s.is_char_boundary(idx) { return idx; }
    if ceil {
        let mut i = idx;
        while i < s.len() && !s.is_char_boundary(i) { i += 1; }
        i
    } else {
        let mut i = idx;
        while i > 0 && !s.is_char_boundary(i) { i -= 1; }
        i
    }
}

fn command_to_candidate(cmd: &Command) -> Candidate {
    let detail = if let Some(binding) = cmd.binding {
        format!("{}  [{}]", cmd.description, binding)
    } else {
        cmd.description.to_string()
    };
    Candidate {
        label: cmd.name.to_string(),
        detail: Some(detail),
        is_action: false,
        data: cmd.id.to_string(),
    }
}
