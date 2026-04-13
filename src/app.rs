use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::command::CommandRegistry;
use crate::editor::{EditorEvent, EditorState, EditorView};
use crate::keymap::{self, KeymapSystem, ResolvedKey, Action};
use crate::minibuffer::{Candidate, DelegateKind, Minibuffer, MinibufferAction, MinibufferVimMode};
use crate::pdf::{PdfState, PdfView, TocEntry};
use crate::state::AppState;

const MAX_RESULTS: usize = 15;

/// Which view is currently active in the main content area.
#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Editor,
    Pdf,
}

pub struct Memex {
    state: AppState,
    view_mode: ViewMode,
    editor_state: Entity<EditorState>,
    editor_view: Entity<EditorView>,
    pdf_view: Option<Entity<PdfView>>,
    keymap: KeymapSystem,
    minibuffer: Minibuffer,
    minibuffer_focus: FocusHandle,
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
            view_mode: ViewMode::Editor,
            editor_state,
            editor_view,
            pdf_view: None,
            keymap,
            minibuffer: Minibuffer::new(),
            minibuffer_focus: cx.focus_handle(),
            command_registry: CommandRegistry::new(),
            _subscriptions: vec![editor_sub],
        }
    }

    /// Sync vim_enabled and insert_mode flags to EditorState (for input handler / cursor rendering)
    fn sync_editor_mode_flags(&self, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        let insert = self.keymap.is_insert_active();
        self.editor_state.update(cx, |state, _cx| {
            state.vim_enabled = vim;
            state.insert_mode = insert;
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

    fn activate_pdf_toc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view_mode != ViewMode::Pdf {
            self.minibuffer.set_message("Not in PDF view");
            cx.notify();
            return;
        }
        let vim = self.keymap.vim_enabled;
        self.minibuffer.activate(DelegateKind::PdfToc, "TOC:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_pdf_goto_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view_mode != ViewMode::Pdf {
            self.minibuffer.set_message("Not in PDF view");
            cx.notify();
            return;
        }
        let page_count = self.pdf_view.as_ref()
            .map(|pv| pv.read(cx).state.read(cx).page_count)
            .unwrap_or(0);
        let vim = self.keymap.vim_enabled;
        let prompt = format!("Go to page (1-{}):", page_count);
        self.minibuffer.activate(DelegateKind::PdfGotoPage, &prompt, vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn dismiss_minibuffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer.dismiss();
        match self.view_mode {
            ViewMode::Pdf => {
                if let Some(ref pv) = self.pdf_view {
                    pv.read(cx).state.read(cx).focus(window);
                } else {
                    self.editor_state.read(cx).focus(window);
                }
            }
            ViewMode::Editor => {
                self.editor_state.read(cx).focus(window);
            }
        }
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
        match self.minibuffer.delegate_kind {
            DelegateKind::Command => {
                self.command_registry.fuzzy_filter(&self.minibuffer.input)
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
            DelegateKind::PdfToc => {
                self.get_pdf_toc_candidates(cx)
            }
            DelegateKind::PdfGotoPage => {
                self.get_pdf_goto_page_candidates(cx)
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
                    let keymap = &mut self.keymap;
                    let editor = self.editor_state.clone();
                    editor.update(cx, |state, cx| {
                        state.execute_ex_command(&input, keymap, window, cx);
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
            DelegateKind::PdfToc => {
                if let Some(candidate) = candidates.get(selected) {
                    if let Ok(page) = candidate.data.parse::<usize>() {
                        self.dismiss_minibuffer(window, cx);
                        if let Some(ref pv) = self.pdf_view {
                            let state = pv.read(cx).state.clone();
                            state.update(cx, |s, cx| {
                                s.goto_page(page);
                                cx.notify();
                            });
                        }
                    }
                }
            }
            DelegateKind::PdfGotoPage => {
                let page_str = if let Some(candidate) = candidates.get(selected) {
                    candidate.data.clone()
                } else {
                    input.clone()
                };
                if let Ok(page_num) = page_str.trim().parse::<usize>() {
                    self.dismiss_minibuffer(window, cx);
                    if let Some(ref pv) = self.pdf_view {
                        let state = pv.read(cx).state.clone();
                        state.update(cx, |s, cx| {
                            s.goto_page_number(page_num);
                            cx.notify();
                        });
                        self.minibuffer.set_message(format!("Page {}", page_num));
                    }
                } else {
                    self.minibuffer.set_message("Invalid page number");
                }
            }
        }
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
                let keymap = &mut self.keymap;
                let editor = self.editor_state.clone();
                let msg = editor.update(cx, |state, _cx| {
                    state.handle_set_command("", keymap)
                });
                self.minibuffer.set_message(msg);
            }
            "set-vim" => {
                self.keymap.set_vim_enabled(true);
                self.sync_editor_mode_flags(cx);
                self.minibuffer.set_message("Vim mode enabled");
                cx.notify();
            }
            "set-novim" => {
                self.keymap.set_vim_enabled(false);
                self.sync_editor_mode_flags(cx);
                self.minibuffer.set_message("Vim mode disabled");
                cx.notify();
            }
            "nohlsearch" => {
                self.minibuffer.set_message("Search highlighting cleared");
            }
            "toggle-vim" => {
                let new_state = !self.keymap.vim_enabled;
                self.keymap.set_vim_enabled(new_state);
                self.sync_editor_mode_flags(cx);
                if new_state {
                    self.minibuffer.set_message("Vim mode enabled");
                } else {
                    self.minibuffer.set_message("Vim mode disabled");
                }
            }
            // PDF commands
            "pdf-toc" | "pdf-bookmarks" => {
                self.activate_pdf_toc(window, cx);
            }
            "pdf-goto-page" => {
                self.activate_pdf_goto_page(window, cx);
            }
            "pdf-fit-width" => {
                self.pdf_command(cx, |s, viewport_w, _vh, cx| {
                    s.fit_width(viewport_w);
                    cx.notify();
                }, window);
            }
            "pdf-fit-page" => {
                self.pdf_command(cx, |s, viewport_w, viewport_h, cx| {
                    s.fit_page(viewport_w, viewport_h);
                    cx.notify();
                }, window);
            }
            "pdf-rotate-cw" => {
                self.pdf_command(cx, |s, _vw, vh, cx| {
                    let (first, _) = s.visible_range(vh);
                    let rotation = s.page_rotations.entry(first).or_insert(0);
                    *rotation = (*rotation + 90) % 360;
                    s.invalidate_cache();
                    cx.notify();
                }, window);
                self.minibuffer.set_message("Rotated clockwise");
            }
            "pdf-rotate-ccw" => {
                self.pdf_command(cx, |s, _vw, vh, cx| {
                    let (first, _) = s.visible_range(vh);
                    let rotation = s.page_rotations.entry(first).or_insert(0);
                    *rotation = (*rotation + 270) % 360;
                    s.invalidate_cache();
                    cx.notify();
                }, window);
                self.minibuffer.set_message("Rotated counter-clockwise");
            }
            "pdf-dark-mode" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.dark_mode = !s.dark_mode;
                    s.invalidate_cache();
                    cx.notify();
                }, window);
                let mode = self.pdf_view.as_ref()
                    .map(|pv| if pv.read(cx).state.read(cx).dark_mode { "on" } else { "off" })
                    .unwrap_or("off");
                self.minibuffer.set_message(format!("Dark mode {}", mode));
            }
            "pdf-two-page" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.spread_mode = !s.spread_mode;
                    cx.notify();
                }, window);
                let mode = self.pdf_view.as_ref()
                    .map(|pv| if pv.read(cx).state.read(cx).spread_mode { "on" } else { "off" })
                    .unwrap_or("off");
                self.minibuffer.set_message(format!("Two-page spread {}", mode));
            }
            "pdf-copy-link" => {
                if let Some(ref pv) = self.pdf_view {
                    let state = pv.read(cx).state.read(cx);
                    let vh: f32 = window.viewport_size().height.into();
                    let (first, _) = state.visible_range(vh);
                    let filename = state.path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file.pdf");
                    let link = format!("[[{}#page={}]]", filename, first + 1);
                    cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                    self.minibuffer.set_message(format!("Copied: {}", link));
                }
            }
            "pdf-extract-text" => {
                if let Some(ref pv) = self.pdf_view {
                    let state = pv.read(cx).state.read(cx);
                    let vh: f32 = window.viewport_size().height.into();
                    let (first, _) = state.visible_range(vh);
                    match state.extract_page_text(first) {
                        Some(text) if !text.is_empty() => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            self.minibuffer.set_message(
                                format!("Copied text from page {}", first + 1)
                            );
                        }
                        _ => {
                            self.minibuffer.set_message("No text on this page");
                        }
                    }
                }
            }
            "pdf-scroll-down" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    let max = s.max_scroll(_vh);
                    s.scroll_offset = (s.scroll_offset + px(60.)).min(max);
                    cx.notify();
                }, window);
            }
            "pdf-scroll-up" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.scroll_offset = (s.scroll_offset - px(60.)).max(px(0.));
                    cx.notify();
                }, window);
            }
            "pdf-half-page-down" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    let max = s.max_scroll(_vh);
                    s.scroll_offset = (s.scroll_offset + px(400.)).min(max);
                    cx.notify();
                }, window);
            }
            "pdf-half-page-up" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.scroll_offset = (s.scroll_offset - px(400.)).max(px(0.));
                    cx.notify();
                }, window);
            }
            "pdf-zoom-in" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.zoom = (s.zoom + 0.1).min(3.0);
                    s.invalidate_cache();
                    cx.notify();
                }, window);
            }
            "pdf-zoom-out" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.zoom = (s.zoom - 0.1).max(0.3);
                    s.invalidate_cache();
                    cx.notify();
                }, window);
            }
            "pdf-goto-first" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    s.scroll_offset = px(0.);
                    cx.notify();
                }, window);
            }
            "pdf-goto-last" => {
                self.pdf_command(cx, |s, _vw, _vh, cx| {
                    let max = s.max_scroll(_vh);
                    s.scroll_offset = max;
                    cx.notify();
                }, window);
            }
            _ => {
                // Try as editor command first (open-line-below, append-after, etc.)
                if self.view_mode == ViewMode::Editor {
                    let keymap = &mut self.keymap;
                    let editor = self.editor_state.clone();
                    editor.update(cx, |state, ecx| {
                        state.execute_command_by_id(cmd_id, count, keymap, window, ecx);
                        // Sync mode flags after command execution
                        state.insert_mode = keymap.is_insert_active();
                        state.vim_enabled = keymap.vim_enabled;
                    });
                    // Forward any status message from the editor to the minibuffer
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

    /// Build candidates for PDF table of contents.
    fn get_pdf_toc_candidates(&self, cx: &App) -> Vec<Candidate> {
        let toc = match self.pdf_view.as_ref() {
            Some(pv) => pv.read(cx).state.read(cx).toc.clone(),
            None => return Vec::new(),
        };
        if toc.is_empty() {
            return vec![Candidate {
                label: "(No table of contents)".to_string(),
                detail: None,
                is_action: false,
                data: String::new(),
            }];
        }

        let query = &self.minibuffer.input;
        let matcher = SkimMatcherV2::default();

        let mut scored: Vec<(i64, &TocEntry)> = toc.iter()
            .filter_map(|entry| {
                if query.is_empty() {
                    Some((0, entry))
                } else {
                    matcher.fuzzy_match(&entry.title, query).map(|s| (s, entry))
                }
            })
            .collect();

        if !query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0));
        }

        scored.into_iter()
            .take(MAX_RESULTS)
            .map(|(_, entry)| {
                let indent = "  ".repeat(entry.level);
                Candidate {
                    label: format!("{}{}",indent, entry.title),
                    detail: Some(format!("Page {}", entry.page + 1)),
                    is_action: false,
                    data: entry.page.to_string(),
                }
            })
            .collect()
    }

    /// Build candidates for go-to-page (shows matching page numbers).
    fn get_pdf_goto_page_candidates(&self, cx: &App) -> Vec<Candidate> {
        let page_count = match self.pdf_view.as_ref() {
            Some(pv) => pv.read(cx).state.read(cx).page_count,
            None => return Vec::new(),
        };
        let input = self.minibuffer.input.trim();
        if input.is_empty() {
            return Vec::new();
        }
        // Show matching page number as a candidate
        if let Ok(num) = input.parse::<usize>() {
            if num >= 1 && num <= page_count {
                return vec![Candidate {
                    label: format!("Page {}", num),
                    detail: Some(format!("of {}", page_count)),
                    is_action: false,
                    data: num.to_string(),
                }];
            }
        }
        Vec::new()
    }

    /// Helper to run a closure on PdfState if in PDF mode.
    fn pdf_command(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut PdfState, f32, f32, &mut Context<PdfState>),
        window: &mut Window,
    ) {
        if self.view_mode != ViewMode::Pdf {
            self.minibuffer.set_message("Not in PDF view");
            cx.notify();
            return;
        }
        if let Some(ref pv) = self.pdf_view {
            let vw: f32 = window.viewport_size().width.into();
            let vh: f32 = window.viewport_size().height.into();
            let state = pv.read(cx).state.clone();
            state.update(cx, |s, cx| f(s, vw, vh, cx));
        }
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
        self.view_mode = ViewMode::Editor;
        // Deactivate PDF layer, reactivate vim layers
        self.keymap.stack.deactivate_layer("pdf");
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
        let pdf_view = cx.new(|cx| PdfView::new(pdf_state.clone(), cx));
        pdf_state.read(cx).focus(window);
        self.pdf_view = Some(pdf_view);
        self.view_mode = ViewMode::Pdf;
        // Activate PDF layer, deactivate vim layers (motions don't apply to PDF)
        self.keymap.stack.activate_layer("pdf");
        self.keymap.stack.deactivate_layer("vim:normal");
        self.keymap.stack.deactivate_layer("vim:motion");
        self.keymap.stack.deactivate_layer("vim:insert");
        self.keymap.stack.deactivate_layer("vim:visual");
        self.keymap.stack.deactivate_layer("vim:operator-pending");
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
        let cursor = es.cursor;
        let content = es.content();
        let _ = es;

        let vault_name = self.state.vault_name();
        let note_title = self.state.current_title();
        let dirty = self.state.dirty;
        let dirty_indicator = if dirty { " ●" } else { "" };

        // Position info depends on view mode
        let position_text = match self.view_mode {
            ViewMode::Pdf => {
                if let Some(ref pv) = self.pdf_view {
                    let ps = pv.read(cx).state.read(cx);
                    let page_count = ps.page_count;
                    let zoom_pct = (ps.zoom * 100.0) as u32;
                    let (first, _) = ps.visible_range(600.0);
                    let current = first + 1;
                    format!("PDF {}/{} {}%", current, page_count, zoom_pct)
                } else {
                    String::new()
                }
            }
            ViewMode::Editor => {
                let mut pos = cursor.min(content.len());
                while pos > 0 && !content.is_char_boundary(pos) {
                    pos -= 1;
                }
                let before = &content[..pos];
                let line_num = before.matches('\n').count() + 1;
                let col_num = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
                format!("L{} C{}", line_num, col_num)
            }
        };

        // Mode badge (left)
        let mode_badge = if self.view_mode == ViewMode::Pdf {
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(rgb(0xCB4B16))  // solarized orange for PDF
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xFDF6E3))
                        .child("PDF"),
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

                // Central key dispatch through the keymap system
                let resolved = this.keymap.resolve_key(key, ctrl, shift, alt);
                match resolved {
                    ResolvedKey::Action(action, count) => {
                        match &action {
                            Action::Command(cmd_id) => {
                                let cmd = cmd_id.clone();
                                this.execute_command(&cmd, "", count, window, cx);
                            }
                            Action::ActivateLayer(layer_id) => {
                                let lid = layer_id.clone();
                                this.keymap.stack.activate_layer(&lid);
                                // Sync mode flags to editor
                                this.sync_editor_mode_flags(cx);
                                // Suppress OS input (don't insert the key character)
                                if this.view_mode == ViewMode::Editor {
                                    this.editor_state.update(cx, |state, _cx| {
                                        state.suppress_next_input = true;
                                        // Handle side effects of mode changes
                                        match lid {
                                            "vim:insert" => {
                                                state.history.break_coalescing();
                                            }
                                            "vim:normal" => {
                                                if !state.selected_range.is_empty() {
                                                    let pos = state.selected_range.start;
                                                    state.selected_range = pos..pos;
                                                    state.cursor = pos;
                                                }
                                                state.history.break_coalescing();
                                            }
                                            "vim:visual" | "vim:visual-line" => {
                                                if state.selected_range.is_empty() {
                                                    let pos = state.cursor;
                                                    let end = state.cursor + 1; // approximate next grapheme
                                                    state.selected_range = pos..end;
                                                    state.selection_reversed = false;
                                                }
                                            }
                                            _ => {}
                                        }
                                    });
                                }
                                cx.notify();
                            }
                            _ => {
                                // Motion, Operator, SelfInsert, etc. — editor-specific
                                if this.view_mode == ViewMode::Editor {
                                    let keymap = &mut this.keymap;
                                    let editor = this.editor_state.clone();
                                    editor.update(cx, |state, ecx| {
                                        let content = state.content();
                                        let cursor = state.cursor;
                                        let result = state.grammar.process(
                                            action, key, count, &content, cursor,
                                            &mut keymap.stack,
                                        );
                                        // Suppress OS input for handled keys
                                        state.suppress_next_input = true;
                                        state.execute_grammar_result(result, count, keymap, window, ecx);
                                        // Sync mode flags for input handler
                                        state.insert_mode = keymap.is_insert_active();
                                        state.vim_enabled = keymap.vim_enabled;
                                    });
                                }
                                // In PDF mode, text editing actions are ignored
                            }
                        }
                    }
                    ResolvedKey::TransientCapture { transient_id, ch, count } => {
                        // f/t/r char captures — editor-only
                        if this.view_mode == ViewMode::Editor {
                            let editor = this.editor_state.clone();
                            editor.update(cx, |state, ecx| {
                                let content = state.content();
                                let cursor = state.cursor;
                                match &*transient_id {
                                    "replace_char" => {
                                        // Replace char at cursor
                                        if cursor < content.len() {
                                            let next_char_len = content[cursor..].chars().next().map_or(1, |c| c.len_utf8());
                                            let mut new_content = content.clone();
                                            new_content.replace_range(cursor..cursor + next_char_len, &ch.to_string());
                                            state.set_content(new_content, window, ecx);
                                        }
                                        state.suppress_next_input = true;
                                        ecx.notify();
                                    }
                                    kind @ ("find_char_forward" | "til_char_forward"
                                          | "find_char_backward" | "til_char_backward") => {
                                        let pos = match kind {
                                            "find_char_forward" => keymap::find_char_forward(&content, cursor, ch, count),
                                            "til_char_forward" => keymap::til_char_forward(&content, cursor, ch, count),
                                            "find_char_backward" => keymap::find_char_backward(&content, cursor, ch, count),
                                            "til_char_backward" => keymap::til_char_backward(&content, cursor, ch, count),
                                            _ => cursor,
                                        };
                                        // Store last char search for ; and ,
                                        state.grammar.last_char_search = Some((ch, kind));
                                        state.cursor = pos;
                                        state.suppress_next_input = true;
                                        ecx.notify();
                                    }
                                    _ => {}
                                }
                            });
                        }
                    }
                    ResolvedKey::Pending => {
                        // Multi-key sequence or count digit — suppress OS input
                        if this.view_mode == ViewMode::Editor {
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
            // Main content area: editor or PDF viewer
            .child({
                let content = div().flex_1().w_full().overflow_hidden();
                match self.view_mode {
                    ViewMode::Editor => content.child(self.editor_view.clone()),
                    ViewMode::Pdf => {
                        if let Some(ref pdf_view) = self.pdf_view {
                            content.child(pdf_view.clone())
                        } else {
                            content.child(self.editor_view.clone())
                        }
                    }
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
