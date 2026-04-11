use freya::prelude::*;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

use crate::state::AppState;

const OVERLAY_BG: (u8, u8, u8) = (20, 20, 35);
const INPUT_BG: (u8, u8, u8) = (40, 40, 65);
const INPUT_TEXT: (u8, u8, u8) = (220, 220, 240);
const ITEM_TEXT: (u8, u8, u8) = (180, 180, 200);
const ITEM_HOVER: (u8, u8, u8) = (50, 50, 80);
const CREATE_COLOR: (u8, u8, u8) = (137, 220, 137);
const MAX_RESULTS: usize = 15;

/// Command bar overlay (Ctrl+P).
/// Searches notes in the current vault, with "create if not exists."
#[derive(PartialEq)]
pub struct CommandBar {
    pub app_state: State<AppState>,
    pub visible: State<bool>,
}

impl Component for CommandBar {
    fn render(&self) -> impl IntoElement {
        let mut app_state = self.app_state;
        let mut visible = self.visible;

        // Always call hooks regardless of visibility (consistent hook ordering)
        let mut query = use_state(String::new);
        let mut selected_index = use_state(|| 0usize);

        let is_visible = *visible.read();

        if !is_visible {
            // Return a zero-size rect that's always in the tree
            return rect().width(Size::px(0.)).height(Size::px(0.));
        }

        let query_text = query.read().clone();
        let results = search_notes(&app_state.read(), &query_text);
        let has_exact_match = results
            .iter()
            .any(|(title, _)| title.to_lowercase() == query_text.to_lowercase());
        let show_create = !query_text.is_empty() && !has_exact_match;
        let total_items = results.len() + if show_create { 1 } else { 0 };

        // Clamp selected index
        let sel = {
            let s = *selected_index.read();
            if total_items == 0 {
                0
            } else {
                s.min(total_items - 1)
            }
        };

        let on_key_down = move |e: Event<KeyboardEventData>| {
            match &e.key {
                Key::Named(NamedKey::Escape) => {
                    visible.set(false);
                    query.set(String::new());
                    selected_index.set(0);
                }
                Key::Named(NamedKey::ArrowDown) => {
                    let current = *selected_index.read();
                    if total_items > 0 && current < total_items - 1 {
                        selected_index.set(current + 1);
                    }
                }
                Key::Named(NamedKey::ArrowUp) => {
                    let current = *selected_index.read();
                    if current > 0 {
                        selected_index.set(current - 1);
                    }
                }
                Key::Named(NamedKey::Enter) => {
                    let query_text = query.read().clone();
                    let results = search_notes(&app_state.read(), &query_text);
                    let has_exact = results
                        .iter()
                        .any(|(t, _)| t.to_lowercase() == query_text.to_lowercase());
                    let show_create = !query_text.is_empty() && !has_exact;

                    let current = *selected_index.read();
                    if current < results.len() {
                        // Open existing note
                        let (_, path) = &results[current];
                        if let Err(e) = app_state.write().open_note(path.clone()) {
                            eprintln!("failed to open note: {}", e);
                        }
                    } else if show_create {
                        // Create new note
                        if let Err(e) = app_state.write().create_note(&query_text) {
                            eprintln!("failed to create note: {}", e);
                        }
                    }
                    visible.set(false);
                    query.set(String::new());
                    selected_index.set(0);
                }
                Key::Named(NamedKey::Backspace) => {
                    let mut q = query.read().clone();
                    q.pop();
                    query.set(q);
                    selected_index.set(0);
                }
                Key::Character(c) => {
                    let mut q = query.read().clone();
                    q.push_str(c);
                    query.set(q);
                    selected_index.set(0);
                }
                _ => {}
            }
            e.stop_propagation();
        };

        // Build result items
        let mut items_container = rect().width(Size::fill());

        for (i, (title, _path)) in results.iter().enumerate() {
            let is_selected = i == sel;
            let bg = if is_selected { ITEM_HOVER } else { OVERLAY_BG };

            items_container = items_container.child(
                rect()
                    .width(Size::fill())
                    .padding((6., 12., 6., 12.))
                    .background(bg)
                    .corner_radius(4.)
                    .child(label().text(title.clone()).font_size(14.).color(ITEM_TEXT)),
            );
        }

        // "Create" option
        if show_create {
            let is_selected = sel == results.len();
            let bg = if is_selected { ITEM_HOVER } else { OVERLAY_BG };

            items_container = items_container.child(
                rect()
                    .width(Size::fill())
                    .padding((6., 12., 6., 12.))
                    .background(bg)
                    .corner_radius(4.)
                    .child(
                        label()
                            .text(format!("+ Create \"{}\"", query_text))
                            .font_size(14.)
                            .color(CREATE_COLOR),
                    ),
            );
        }

        // Overlay
        rect()
            .width(Size::fill())
            .height(Size::fill())
            .position(Position::new_absolute())
            .background((0, 0, 0, 120))
            .on_global_key_down(on_key_down)
            .child(
                rect()
                    .width(Size::px(500.))
                    .max_height(Size::px(400.))
                    .margin((80., 0., 0., 0.))
                    .background(OVERLAY_BG)
                    .corner_radius(8.)
                    .padding(8.)
                    .overflow(Overflow::Clip)
                    .child(
                        // Search input display
                        rect()
                            .width(Size::fill())
                            .padding((10., 12., 10., 12.))
                            .background(INPUT_BG)
                            .corner_radius(6.)
                            .child(
                                label()
                                    .text(if query_text.is_empty() {
                                        "Search notes...".to_string()
                                    } else {
                                        query_text.clone()
                                    })
                                    .font_size(15.)
                                    .color(if query_text.is_empty() {
                                        (100, 100, 120)
                                    } else {
                                        INPUT_TEXT
                                    }),
                            ),
                    )
                    .child(
                        // Results list
                        ScrollView::new().child(items_container),
                    ),
            )
    }
}

/// Fuzzy-search note titles in the current vault.
fn search_notes(state: &AppState, query: &str) -> Vec<(String, std::path::PathBuf)> {
    let vault = match &state.vault {
        Some(v) => v,
        None => return Vec::new(),
    };

    let titles = vault.note_titles();

    if query.is_empty() {
        // Show all notes (up to limit)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_notes_empty_vault() {
        let state = AppState::new();
        let results = search_notes(&state, "test");
        assert!(results.is_empty());
    }
}
