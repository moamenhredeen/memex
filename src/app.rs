use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::text::TextView;
use gpui_component::{h_flex, v_flex};

use crate::state::AppState;

const MAX_RESULTS: usize = 15;

pub struct Memex {
    state: AppState,
    editor_input: Entity<InputState>,
    command_bar_input: Entity<InputState>,
    command_bar_visible: bool,
    vault_dropdown_visible: bool,
    _subscriptions: Vec<Subscription>,
}

impl Memex {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::new();

        let initial_content = if state.content.is_empty() {
            "# Welcome to Memex\n\nOpen or create a vault to get started.\nUse **Ctrl+P** to search and create notes.\n\n---\n\nSupports *italic*, **bold**, ~~strikethrough~~, `code`, and more.".to_string()
        } else {
            state.content.clone()
        };

        let editor_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Start typing...")
                .default_value(initial_content)
        });

        let command_bar_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search notes...")
        });

        // Track editor changes to mark dirty
        let editor_sub = cx.subscribe_in(&editor_input, window, |this, _entity, ev: &InputEvent, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                this.state.dirty = true;
                cx.notify();
            }
        });

        Self {
            state,
            editor_input,
            command_bar_input,
            command_bar_visible: false,
            vault_dropdown_visible: false,
            _subscriptions: vec![editor_sub],
        }
    }

    fn toggle_command_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_bar_visible = !self.command_bar_visible;
        if self.command_bar_visible {
            self.command_bar_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
            self.command_bar_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor_input.read(cx).value().to_string();
        self.state.content = text;
        if let Err(e) = self.state.save() {
            eprintln!("save error: {}", e);
        }
        cx.notify();
    }

    fn open_note_by_path(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.state.open_note(path) {
            eprintln!("failed to open note: {}", e);
            return;
        }
        let content = self.state.content.clone();
        self.editor_input.update(cx, |input, cx| {
            input.set_value(content, window, cx);
        });
        cx.notify();
    }

    fn create_note_by_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.state.create_note(title) {
            Ok(_) => {
                let content = self.state.content.clone();
                self.editor_input.update(cx, |input, cx| {
                    input.set_value(content, window, cx);
                });
            }
            Err(e) => eprintln!("failed to create note: {}", e),
        }
        cx.notify();
    }

    fn open_vault_by_path(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.state.open_vault(path) {
            eprintln!("failed to open vault: {}", e);
            return;
        }
        let content = self.state.content.clone();
        self.editor_input.update(cx, |input, cx| {
            input.set_value(content, window, cx);
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

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let vault_name = self.state.vault_name();
        let note_title = self.state.current_title();
        let dirty = self.state.dirty;
        let dirty_indicator = if dirty { " ●" } else { "" };

        h_flex()
            .w_full()
            .h(px(32.))
            .bg(rgb(0xEBEBF0))
            .items_center()
            .px(px(12.))
            .gap(px(12.))
            .border_t_1()
            .border_color(rgb(0xD0D0D8))
            .child(
                div()
                    .id("vault-switcher")
                    .px(px(8.))
                    .py(px(4.))
                    .bg(rgb(0xF5F5FA))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                        this.vault_dropdown_visible = !this.vault_dropdown_visible;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0x2864C8))
                            .child(format!("⌂ {}", vault_name)),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(0x50505A))
                    .child(format!("{}{}", note_title, dirty_indicator)),
            )
    }

    fn render_vault_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let vault_paths = self.state.registry.vault_paths();

        let mut dropdown = v_flex()
            .absolute()
            .bottom(px(40.))
            .left(px(12.))
            .w(px(250.))
            .bg(rgb(0xF5F5FA))
            .rounded(px(6.))
            .p(px(4.))
            .border_1()
            .border_color(rgb(0xD0D0D8))
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
                    .hover(|s| s.bg(rgb(0xE1E6F5)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                        this.open_vault_by_path(path.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(0x50505A))
                            .child(name),
                    ),
            );
        }

        dropdown
    }

    fn render_command_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.command_bar_input.read(cx).value().to_string();
        let results = self.search_notes(&query);
        let has_exact = results
            .iter()
            .any(|(t, _)| t.to_lowercase() == query.to_lowercase());
        let show_create = !query.is_empty() && !has_exact;

        let mut results_list = v_flex().id("results-list").w_full().overflow_y_scroll();

        for (i, (title, path)) in results.into_iter().enumerate() {
            let p = path.clone();
            results_list = results_list.child(
                div()
                    .id(ElementId::Name(format!("result-{}", i).into()))
                    .w_full()
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xE1E6F5)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                        this.open_note_by_path(p.clone(), window, cx);
                        this.command_bar_visible = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(0x3C3C46))
                            .child(title),
                    ),
            );
        }

        if show_create {
            let title_for_create = query.clone();
            results_list = results_list.child(
                div()
                    .id("create-note")
                    .w_full()
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xE1E6F5)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                        this.create_note_by_title(&title_for_create, window, cx);
                        this.command_bar_visible = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(0x1E8C32))
                            .child(format!("+ Create \"{}\"", query)),
                    ),
            );
        }

        // Full-screen overlay
        div()
            .id("command-bar-overlay")
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .size_full()
            .bg(rgba(0x00000040))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                this.command_bar_visible = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("command-bar-panel")
                    .w(px(500.))
                    .max_h(px(400.))
                    .mt(px(80.))
                    .mx_auto()
                    .bg(rgb(0xFAFAFC))
                    .rounded(px(8.))
                    .p(px(8.))
                    .border_1()
                    .border_color(rgb(0xD0D0D8))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_e: &MouseDownEvent, _window, _cx| {
                        // Stop propagation — don't close on inner click
                    })
                    .child(
                        div()
                            .w_full()
                            .mb(px(8.))
                            .child(Input::new(&self.command_bar_input).w_full()),
                    )
                    .child(results_list),
            )
    }
}

impl Render for Memex {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.editor_input.read(cx).value().to_string();

        let mut root = v_flex()
            .id("memex-root")
            .size_full()
            .bg(rgb(0xF8F8F8))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.modifiers.control && e.keystroke.key == "p" {
                    this.toggle_command_bar(window, cx);
                } else if e.keystroke.modifiers.control && e.keystroke.key == "s" {
                    this.save(window, cx);
                }
            }))
            // Editor area — markdown preview
            .child(
                div()
                    .id("markdown-preview")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .p(px(24.))
                    .child(
                        TextView::markdown("md-view", content.clone(), window, cx)
                    ),
            )
            // Raw editor input
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .p(px(8.))
                    .border_t_1()
                    .border_color(rgb(0xD0D0D8))
                    .child(Input::new(&self.editor_input).w_full()),
            )
            // Status bar
            .child(self.render_status_bar(cx));

        // Vault dropdown overlay
        if self.vault_dropdown_visible {
            root = root.child(self.render_vault_dropdown(cx));
        }

        // Command bar overlay
        if self.command_bar_visible {
            root = root.child(self.render_command_bar(cx));
        }

        root
    }
}
