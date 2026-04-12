use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::editor::keymap::EditorMode;
use crate::editor::{EditorEvent, EditorState, EditorView};
use crate::state::AppState;

const MAX_RESULTS: usize = 15;

/// What the minibuffer is currently being used for.
#[derive(Clone, PartialEq)]
enum MinibufferMode {
    /// Inactive — shows status line only
    Idle,
    /// Note search (Ctrl+P) — vertico-style vertical completion
    NoteSearch,
    /// Vim command line (:) — handled by EditorState, we just display it
    VimCommand,
}

pub struct Memex {
    state: AppState,
    editor_state: Entity<EditorState>,
    editor_view: Entity<EditorView>,
    minibuffer_mode: MinibufferMode,
    minibuffer_input: String,
    minibuffer_selected: usize,
    vault_dropdown_visible: bool,
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
                        cx.notify();
                    }
                    EditorEvent::RequestSave => {
                        this.save(window, cx);
                        this.editor_state.update(cx, |state, _cx| {
                            state.status_message = Some("Written".into());
                        });
                    }
                    EditorEvent::RequestQuit => {
                        cx.quit();
                    }
                    EditorEvent::RequestOpen(path) => {
                        let path = std::path::PathBuf::from(path.clone());
                        this.open_note_by_path(path, window, cx);
                    }
                }
            },
        );

        Self {
            state,
            editor_state,
            editor_view,
            minibuffer_mode: MinibufferMode::Idle,
            minibuffer_input: String::new(),
            minibuffer_selected: 0,
            vault_dropdown_visible: false,
            _subscriptions: vec![editor_sub],
        }
    }

    fn activate_note_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer_mode = MinibufferMode::NoteSearch;
        self.minibuffer_input.clear();
        self.minibuffer_selected = 0;
        cx.notify();
    }

    fn dismiss_minibuffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer_mode = MinibufferMode::Idle;
        self.minibuffer_input.clear();
        self.minibuffer_selected = 0;
        self.editor_state.read(cx).focus(window);
        cx.notify();
    }

    fn handle_minibuffer_key(
        &mut self,
        key: &str,
        ctrl: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match key {
            "escape" => {
                self.dismiss_minibuffer(window, cx);
            }
            "enter" => {
                let results = self.search_notes(&self.minibuffer_input);
                let idx = self.minibuffer_selected;
                if let Some((_, path)) = results.get(idx) {
                    let p = path.clone();
                    self.open_note_by_path(p, window, cx);
                } else if !self.minibuffer_input.is_empty() {
                    // Create new note
                    let title = self.minibuffer_input.clone();
                    self.create_note_by_title(&title, window, cx);
                }
                self.dismiss_minibuffer(window, cx);
            }
            "backspace" => {
                self.minibuffer_input.pop();
                self.minibuffer_selected = 0;
                cx.notify();
            }
            "up" => {
                if self.minibuffer_selected > 0 {
                    self.minibuffer_selected -= 1;
                }
                cx.notify();
            }
            "down" => {
                let results = self.search_notes(&self.minibuffer_input);
                if self.minibuffer_selected + 1 < results.len() {
                    self.minibuffer_selected += 1;
                }
                cx.notify();
            }
            "tab" => {
                // Insert selected candidate text into minibuffer (like vertico-insert)
                let results = self.search_notes(&self.minibuffer_input);
                if let Some((title, _)) = results.get(self.minibuffer_selected) {
                    self.minibuffer_input = title.clone();
                }
                cx.notify();
            }
            _ if ctrl => {
                if key == "u" {
                    self.minibuffer_input.clear();
                    self.minibuffer_selected = 0;
                    cx.notify();
                } else if key == "n" {
                    // Ctrl+N = next (like Emacs)
                    let results = self.search_notes(&self.minibuffer_input);
                    if self.minibuffer_selected + 1 < results.len() {
                        self.minibuffer_selected += 1;
                    }
                    cx.notify();
                } else if key == "p" {
                    // Ctrl+P in minibuffer = previous
                    if self.minibuffer_selected > 0 {
                        self.minibuffer_selected -= 1;
                    }
                    cx.notify();
                }
            }
            _ => {
                if key.len() == 1 {
                    self.minibuffer_input.push_str(key);
                    self.minibuffer_selected = 0;
                    cx.notify();
                }
            }
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
        self.vault_dropdown_visible = false;
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

    fn render_mode_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let es = self.editor_state.read(cx);
        let vim_enabled = es.vim.enabled;
        let mode = es.mode;
        let cursor = es.cursor;
        let content = es.content();
        drop(es);

        // Compute line:col from cursor
        let before = &content[..cursor.min(content.len())];
        let line_num = before.matches('\n').count() + 1;
        let col_num = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;

        let vault_name = self.state.vault_name();
        let note_title = self.state.current_title();
        let dirty = self.state.dirty;
        let dirty_indicator = if dirty { " ●" } else { "" };

        // Mode badge (left)
        let mode_badge = if vim_enabled {
            let (label, bg) = match mode {
                EditorMode::Normal => ("NOR", rgb(0x89B4FA)),
                EditorMode::Insert => ("INS", rgb(0xA6E3A1)),
                EditorMode::Visual => ("VIS", rgb(0xCBA6F7)),
                EditorMode::VisualLine => ("V-L", rgb(0xCBA6F7)),
                EditorMode::Command => ("CMD", rgb(0xF9E2AF)),
            };
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(bg)
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x1E1E2E))
                        .child(label),
                )
        } else {
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(rgb(0xA6E3A1))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x1E1E2E))
                        .child("EDT"),
                )
        };

        h_flex()
            .w_full()
            .h(px(24.))
            .bg(rgb(0x313244))
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
                            .text_color(rgb(0xBAC2DE))
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
                            .text_color(rgb(0x6C7086))
                            .child(format!("L{} C{}", line_num, col_num)),
                    ),
            )
    }

    /// Render the minibuffer area. Always visible like emacs.
    /// Shows: status messages (idle), vim command line (:), or note search with vertico candidates.
    fn render_minibuffer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let es = self.editor_state.read(cx);
        let vim_command_mode = es.vim.enabled && es.mode == EditorMode::Command;
        let command_line = es.command_line.clone();
        let status_msg = es.status_message.clone();
        drop(es);

        let base = v_flex()
            .w_full()
            .bg(rgb(0x1E1E2E));

        match &self.minibuffer_mode {
            MinibufferMode::NoteSearch => {
                let results = self.search_notes(&self.minibuffer_input);
                let has_exact = results
                    .iter()
                    .any(|(t, _)| t.to_lowercase() == self.minibuffer_input.to_lowercase());
                let show_create = !self.minibuffer_input.is_empty() && !has_exact;
                let selected = self.minibuffer_selected;

                let mut items = v_flex().w_full();

                for (i, (title, _path)) in results.iter().enumerate() {
                    let is_selected = i == selected;
                    let bg_color = if is_selected {
                        rgb(0x45475A)
                    } else {
                        rgb(0x1E1E2E)
                    };
                    items = items.child(
                        div()
                            .id(ElementId::Name(format!("mb-result-{}", i).into()))
                            .w_full()
                            .px(px(8.))
                            .py(px(2.))
                            .bg(bg_color)
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(if is_selected {
                                        rgb(0xCDD6F4)
                                    } else {
                                        rgb(0xA6ADC8)
                                    })
                                    .child(title.clone()),
                            ),
                    );
                }

                if show_create {
                    let is_selected = selected == results.len();
                    let bg_color = if is_selected {
                        rgb(0x45475A)
                    } else {
                        rgb(0x1E1E2E)
                    };
                    items = items.child(
                        div()
                            .id("mb-create")
                            .w_full()
                            .px(px(8.))
                            .py(px(2.))
                            .bg(bg_color)
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(0xA6E3A1))
                                    .child(format!(
                                        "+ Create \"{}\"",
                                        self.minibuffer_input
                                    )),
                            ),
                    );
                }

                base
                    .border_t_1()
                    .border_color(rgb(0x45475A))
                    // Prompt line
                    .child(
                        h_flex()
                            .w_full()
                            .px(px(8.))
                            .py(px(3.))
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(0x89B4FA))
                                    .child("Find note:"),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(0xCDD6F4))
                                    .child(format!("{}█", self.minibuffer_input)),
                            ),
                    )
                    // Candidates (vertico-style vertical list)
                    .child(items)
            }
            _ if vim_command_mode => {
                // Vim command line
                base
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(22.))
                            .px(px(8.))
                            .py(px(3.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_family("FiraCode Nerd Font Mono")
                                    .text_color(rgb(0xCDD6F4))
                                    .child(format!(":{}█", command_line)),
                            ),
                    )
            }
            _ => {
                // Idle — show status message or empty minibuffer
                let msg = status_msg.unwrap_or_default();
                base
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(22.))
                            .px(px(8.))
                            .py(px(3.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(0x6C7086))
                                    .child(msg),
                            ),
                    )
            }
        }
    }

    fn render_vault_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let vault_paths = self.state.registry.vault_paths();

        let mut dropdown = v_flex()
            .absolute()
            .bottom(px(40.))
            .left(px(12.))
            .w(px(250.))
            .bg(rgb(0x313244))
            .rounded(px(6.))
            .p(px(4.))
            .border_1()
            .border_color(rgb(0x45475A))
            .shadow_md();

        for (i, vault_path) in vault_paths.into_iter().enumerate() {
            let name = vault_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("vault")
                .to_string();

            let path = vault_path.clone();
            dropdown = dropdown.child(
                div()
                    .id(ElementId::Name(format!("vault-{}", i).into()))
                    .w_full()
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x45475A)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                            this.open_vault_by_path(path.clone(), window, cx);
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0xBAC2DE))
                            .child(name),
                    ),
            );
        }

        dropdown
    }
}

impl Render for Memex {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let minibuffer_active = self.minibuffer_mode == MinibufferMode::NoteSearch;

        let mut root = v_flex()
            .id("memex-root")
            .size_full()
            .bg(rgb(0xF8F8F8))
            .font_family("FiraCode Nerd Font")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                // If minibuffer is active, route keys there
                if this.minibuffer_mode == MinibufferMode::NoteSearch {
                    let key = e.keystroke.key.as_str();
                    let ctrl = e.keystroke.modifiers.control;
                    this.handle_minibuffer_key(key, ctrl, window, cx);
                    return;
                }

                if e.keystroke.modifiers.control && e.keystroke.key == "p" {
                    this.activate_note_search(window, cx);
                } else if e.keystroke.modifiers.control && e.keystroke.key == "s" {
                    this.save(window, cx);
                }
            }))
            // Editor canvas
            .child(div().flex_1().w_full().child(self.editor_view.clone()))
            // Mode line (always visible, like emacs mode-line)
            .child(self.render_mode_line(cx))
            // Minibuffer area (below mode line, like emacs)
            .child(self.render_minibuffer(cx));

        // Vault dropdown overlay
        if self.vault_dropdown_visible {
            root = root.child(self.render_vault_dropdown(cx));
        }

        // Dim overlay when minibuffer is active (optional)
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
