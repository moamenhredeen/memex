use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::command::CommandRegistry;
use crate::editor::{EditorEvent, EditorState, EditorView};
use crate::minibuffer::{Candidate, DelegateKind, Minibuffer, MinibufferAction, MinibufferVimMode};
use crate::state::AppState;

const MAX_RESULTS: usize = 15;

pub struct Memex {
    state: AppState,
    editor_state: Entity<EditorState>,
    editor_view: Entity<EditorView>,
    minibuffer: Minibuffer,
    command_registry: CommandRegistry,
    _subscriptions: Vec<Subscription>,
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
                    EditorEvent::RequestVaultSearch => {
                        this.activate_vault_search(window, cx);
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

        Self {
            state,
            editor_state,
            editor_view,
            minibuffer: Minibuffer::new(),
            command_registry: CommandRegistry::new(),
            _subscriptions: vec![editor_sub],
        }
    }

    fn activate_note_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.editor_state.read(cx).keymap.vim_enabled;
        self.minibuffer.activate(DelegateKind::NoteSearch, "Find note:", vim);
        cx.notify();
    }

    fn activate_vault_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.editor_state.read(cx).keymap.vim_enabled;
        self.minibuffer.activate(DelegateKind::VaultSearch, "Switch vault:", vim);
        cx.notify();
    }

    fn activate_command_palette(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.editor_state.read(cx).keymap.vim_enabled;
        let prompt = if vim { ":" } else { "M-x" };
        self.minibuffer.activate(DelegateKind::Command, prompt, vim);
        cx.notify();
    }

    fn dismiss_minibuffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer.dismiss();
        self.editor_state.read(cx).focus(window);
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
        let candidates = self.get_candidates();
        let action = self.minibuffer.handle_key(key, ctrl, shift, candidates.len());

        match action {
            MinibufferAction::Updated => {
                cx.notify();
            }
            MinibufferAction::Confirm => {
                let candidates = self.get_candidates();
                self.handle_confirm(candidates, window, cx);
            }
            MinibufferAction::Complete => {
                let candidates = self.get_candidates();
                if let Some(c) = candidates.get(self.minibuffer.selected) {
                    // Tab inserts the candidate's primary text (vertico-insert)
                    self.minibuffer.input = c.label.clone();
                    self.minibuffer.cursor = self.minibuffer.input.len();
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
    fn get_candidates(&self) -> Vec<Candidate> {
        match self.minibuffer.delegate_kind {
            DelegateKind::Command => {
                self.command_registry.fuzzy_filter(&self.minibuffer.input)
            }
            DelegateKind::NoteSearch => {
                self.get_note_candidates()
            }
            DelegateKind::VaultSearch => {
                self.get_vault_candidates()
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
                    self.execute_command(&cmd_id, &input, window, cx);
                } else if !input.is_empty() {
                    // Try executing raw input as ex command
                    self.dismiss_minibuffer(window, cx);
                    self.editor_state.update(cx, |state, cx| {
                        state.execute_ex_command(&input, window, cx);
                    });
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
            DelegateKind::VaultSearch => {
                if let Some(candidate) = candidates.get(selected) {
                    if candidate.is_action {
                        // "Open directory" action
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
                            self.dismiss_minibuffer(window, cx);
                        }
                    } else {
                        let path = std::path::PathBuf::from(&candidate.data);
                        self.dismiss_minibuffer(window, cx);
                        self.open_vault_by_path(path, window, cx);
                        // Auto-chain: open note search after vault switch
                        self.activate_note_search(window, cx);
                    }
                } else if !input.is_empty() {
                    let path = std::path::PathBuf::from(&input);
                    if path.is_dir() {
                        self.dismiss_minibuffer(window, cx);
                        self.open_vault_by_path(path, window, cx);
                        self.activate_note_search(window, cx);
                    } else {
                        self.minibuffer.set_message(format!("Not a directory: {}", input));
                        self.dismiss_minibuffer(window, cx);
                    }
                }
            }
        }
    }

    /// Execute a command by registry id.
    fn execute_command(
        &mut self,
        cmd_id: &str,
        raw_input: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd_id {
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
            "vault" => {
                self.activate_vault_search(window, cx);
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
            "set" => {
                let msg = self.editor_state.update(cx, |state, _cx| {
                    state.handle_set_command("")
                });
                self.minibuffer.set_message(msg);
            }
            "set-vim" => {
                self.editor_state.update(cx, |state, cx| {
                    state.keymap.set_vim_enabled(true);
                    cx.notify();
                });
                self.minibuffer.set_message("Vim mode enabled");
            }
            "set-novim" => {
                self.editor_state.update(cx, |state, cx| {
                    state.keymap.set_vim_enabled(false);
                    cx.notify();
                });
                self.minibuffer.set_message("Vim mode disabled");
            }
            "nohlsearch" => {
                self.minibuffer.set_message("Search highlighting cleared");
            }
            "toggle-vim" => {
                let enabled = self.editor_state.update(cx, |state, cx| {
                    let new_state = !state.keymap.vim_enabled;
                    state.keymap.set_vim_enabled(new_state);
                    cx.notify();
                    new_state
                });
                if enabled {
                    self.minibuffer.set_message("Vim mode enabled");
                } else {
                    self.minibuffer.set_message("Vim mode disabled");
                }
            }
            _ => {
                // Try as raw ex command (for plugin commands, etc.)
                self.editor_state.update(cx, |state, cx| {
                    state.execute_ex_command(cmd_id, window, cx);
                });
                // Forward any status message from the editor to the minibuffer
                if let Some(msg) = self.editor_state.read(cx).status_message.clone() {
                    self.minibuffer.set_message(msg);
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
            .map(|(title, path)| Candidate {
                label: title,
                detail: None,
                is_action: false,
                data: path.to_string_lossy().to_string(),
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

    /// Build vault candidates from registry + directory listing.
    fn get_vault_candidates(&self) -> Vec<Candidate> {
        let results = self.search_vaults(&self.minibuffer.input);
        let show_new = !self.minibuffer.input.is_empty()
            && !results
                .iter()
                .any(|(_, p)| p.to_string_lossy() == self.minibuffer.input);

        let mut candidates: Vec<Candidate> = results
            .into_iter()
            .map(|(name, path)| {
                let is_current = self
                    .state
                    .vault
                    .as_ref()
                    .map(|v| v.path == path)
                    .unwrap_or(false);
                let suffix = if is_current { "  (current)" } else { "" };
                Candidate {
                    label: format!("{}{}", name, suffix),
                    detail: Some(path.to_string_lossy().to_string()),
                    is_action: false,
                    data: path.to_string_lossy().to_string(),
                }
            })
            .collect();

        if show_new {
            candidates.push(Candidate {
                label: format!("⊕ Open directory: {}", self.minibuffer.input),
                detail: None,
                is_action: true,
                data: self.minibuffer.input.clone(),
            });
        }

        candidates
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
        if let Err(e) = self.state.open_note(path) {
            eprintln!("failed to open note: {}", e);
            return;
        }
        let content = self.state.content.clone();
        self.editor_state.update(cx, |state, cx| {
            state.set_content(content, window, cx);
        });
        cx.notify();
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

    /// Search registered vaults + scan home for directories.
    fn search_vaults(&self, query: &str) -> Vec<(String, std::path::PathBuf)> {
        let registered = self.state.registry.vault_paths();

        // Build display entries: (display_name, path)
        let mut entries: Vec<(String, std::path::PathBuf)> = registered
            .iter()
            .map(|p| {
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("vault")
                    .to_string();
                (name, p.clone())
            })
            .collect();

        // If query looks like a path, try listing that directory's subdirectories
        if query.starts_with('/') || query.starts_with('~') || query.starts_with('.') {
            let expanded = if query.starts_with('~') {
                let rest = query.get(1..).unwrap_or("");
                let rest = rest.strip_prefix('/').unwrap_or(rest);
                dirs::home_dir()
                    .map(|h| if rest.is_empty() { h } else { h.join(rest) })
                    .unwrap_or_else(|| std::path::PathBuf::from(query))
            } else {
                std::path::PathBuf::from(query)
            };

            // List subdirectories of the parent that match the partial name
            let (parent, prefix) = if expanded.is_dir() {
                (expanded.clone(), String::new())
            } else {
                let parent = expanded.parent().unwrap_or(std::path::Path::new("/")).to_path_buf();
                let prefix = expanded
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                (parent, prefix)
            };

            if let Ok(read_dir) = std::fs::read_dir(&parent) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    // Skip hidden directories
                    if name.starts_with('.') {
                        continue;
                    }
                    if !prefix.is_empty() && !name.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    // Don't duplicate registered vaults
                    if !entries.iter().any(|(_, p)| *p == path) {
                        entries.push((name, path));
                    }
                }
            }
        }

        if query.is_empty() {
            return entries.into_iter().take(MAX_RESULTS).collect();
        }

        // Fuzzy match against query
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, String, std::path::PathBuf)> = entries
            .into_iter()
            .filter_map(|(name, path)| {
                // Match against both name and full path
                let name_score = matcher.fuzzy_match(&name, query);
                let path_score = matcher.fuzzy_match(&path.to_string_lossy(), query);
                let best = name_score.max(path_score);
                best.map(|score| (score, name, path))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, name, path)| (name, path))
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
            .hover(|s| s
                .text_color(rgba(0x00000010))
                .bg(rgba(0xFF000040))
                .cursor_pointer()
            )
            .on_mouse_down(MouseButton::Left, |_e, _w, _cx| {})  // stop propagation
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.quit();
            }))
            .child("✕")
    }

    fn render_mode_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let es = self.editor_state.read(cx);
        let vim_enabled = es.keymap.vim_enabled;
        let vim_state = es.keymap.active_vim_state().map(|s| s.to_string());
        let cursor = es.cursor;
        let content = es.content();
        let _ = es;

        // Compute line:col from cursor
        let mut pos = cursor.min(content.len());
        while pos > 0 && !content.is_char_boundary(pos) {
            pos -= 1;
        }
        let before = &content[..pos];
        let line_num = before.matches('\n').count() + 1;
        let col_num = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;

        let vault_name = self.state.vault_name();
        let note_title = self.state.current_title();
        let dirty = self.state.dirty;
        let dirty_indicator = if dirty { " ●" } else { "" };

        // Mode badge (left)
        let mode_badge = if vim_enabled {
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
                            .child(format!("L{} C{}", line_num, col_num)),
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
        let candidates = self.get_candidates();
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

            let mut row = h_flex().gap(px(8.)).child(
                div()
                    .text_size(px(13.))
                    .text_color(text_color)
                    .child(candidate.label.clone()),
            );

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
                // If minibuffer is active, route keys there
                if this.minibuffer.active {
                    let key = e.keystroke.key.as_str();
                    let ctrl = e.keystroke.modifiers.control;
                    let shift = e.keystroke.modifiers.shift;
                    this.handle_minibuffer_key(key, ctrl, shift, window, cx);
                    return;
                }

                if e.keystroke.modifiers.control && e.keystroke.key == "p" {
                    this.activate_note_search(window, cx);
                } else if e.keystroke.modifiers.control && e.keystroke.key == "s" {
                    this.save(window, cx);
                } else if e.keystroke.modifiers.alt && e.keystroke.key == "x" {
                    // M-x — command palette
                    this.activate_command_palette(window, cx);
                }
            }))
            // Custom title bar with drag + window controls
            .child(self.render_title_bar(cx))
            // Editor canvas
            .child(div().flex_1().w_full().child(self.editor_view.clone()))
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
                    .bg(rgba(0x00000020))
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
