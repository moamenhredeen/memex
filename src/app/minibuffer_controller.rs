use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;

use super::{MAX_RESULTS, Memex, command_registry, ui_helpers};
use crate::command::Command;
use crate::editor::EditorEvent;
use crate::minibuffer::{Candidate, DelegateKind, MinibufferAction};
use crate::theme;

impl Memex {
    pub(super) fn activate_note_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::NoteSearch, "Find note:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_vault_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::VaultSwitch, "Switch vault:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_vault_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::VaultOpen, "Open vault:", vim);
        if let Some(home) = dirs::home_dir() {
            let seed = format!("{}/", home.to_string_lossy());
            self.minibuffer.input = seed.clone();
            self.minibuffer.cursor = seed.len();
        }
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        let prompt = if vim { ":" } else { "M-x" };
        self.minibuffer.activate(DelegateKind::Command, prompt, vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_wikilink_autocomplete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::WikilinkAutocomplete, "Link to:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_tag_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer.activate(DelegateKind::TagList, "Tag:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_tag_notes(
        &mut self,
        tag: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let vim = self.vim_enabled(cx);
        self.minibuffer.activate(
            DelegateKind::TagNotes(tag.to_string()),
            &format!("#{}:", tag),
            vim,
        );
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_orphans(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::Orphans, "Orphans:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_content_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::ContentSearch, "Search:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn activate_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer.activate(DelegateKind::Theme, "Theme:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    pub(super) fn dismiss_minibuffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer.dismiss();
        self.focused_item().focus(window, cx);
        cx.notify();
    }

    pub(super) fn handle_minibuffer_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let candidates = self.get_candidates(cx);
        let action = self
            .minibuffer
            .handle_key(key, ctrl, shift, candidates.len());

        match action {
            MinibufferAction::Updated => {
                if let DelegateKind::Item(ref id) = self.minibuffer.delegate_kind {
                    let input = self.minibuffer.input.clone();
                    self.focused_item().on_input_changed(id, &input, cx);
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
                        let path = format!("{}/", c.data);
                        self.minibuffer.input = path.clone();
                        self.minibuffer.cursor = path.len();
                    } else {
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

    pub(super) fn get_candidates(&self, cx: &App) -> Vec<Candidate> {
        match &self.minibuffer.delegate_kind {
            DelegateKind::Command => self.palette_candidates(&self.minibuffer.input),
            DelegateKind::NoteSearch => self.get_note_candidates(),
            DelegateKind::WikilinkAutocomplete => self.get_wikilink_candidates(),
            DelegateKind::VaultSwitch => self.get_vault_switch_candidates(),
            DelegateKind::VaultOpen => self.get_vault_open_candidates(),
            DelegateKind::TagList => self.get_tag_list_candidates(),
            DelegateKind::TagNotes(tag) => self.get_tag_notes_candidates(tag),
            DelegateKind::Orphans => self.get_orphans_candidates(),
            DelegateKind::ContentSearch => self.get_content_search_candidates(),
            DelegateKind::Theme => self.get_theme_candidates(),
            DelegateKind::Item(id) => {
                self.focused_item()
                    .get_candidates(id, &self.minibuffer.input, cx)
            }
        }
    }

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
                    self.dismiss_minibuffer(window, cx);
                    let vim = self.vim_snapshot(cx);
                    let editor = self.active_editor_state();
                    let actions = editor.update(cx, |state, cx| {
                        state.execute_ex_command(&input, vim, window, cx)
                    });
                    self.process_item_actions(actions, window, cx);
                }
            }
            DelegateKind::NoteSearch => {
                if let Some(candidate) = candidates.get(selected) {
                    if candidate.is_action {
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
            DelegateKind::WikilinkAutocomplete => {
                if let Some(candidate) = candidates.get(selected) {
                    let title = candidate.data.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.insert_wikilink_completion(&title, window, cx);
                } else if !input.is_empty() {
                    let title = input.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.insert_wikilink_completion(&title, window, cx);
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
                        self.minibuffer
                            .set_message(format!("Not a directory: {}", candidate.data));
                    }
                } else if !input.is_empty() {
                    let path = std::path::PathBuf::from(&input);
                    if path.is_dir() {
                        self.dismiss_minibuffer(window, cx);
                        self.open_vault_by_path(path, window, cx);
                        self.activate_note_search(window, cx);
                    } else {
                        self.minibuffer
                            .set_message(format!("Not a directory: {}", input));
                    }
                }
            }
            DelegateKind::TagList => {
                if let Some(candidate) = candidates.get(selected) {
                    let tag = candidate.data.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.activate_tag_notes(&tag, window, cx);
                }
            }
            DelegateKind::TagNotes(_) => {
                if let Some(candidate) = candidates.get(selected) {
                    let path = std::path::PathBuf::from(&candidate.data);
                    self.dismiss_minibuffer(window, cx);
                    self.open_note_by_path(path, window, cx);
                }
            }
            DelegateKind::Orphans => {
                if let Some(candidate) = candidates.get(selected) {
                    let path = std::path::PathBuf::from(&candidate.data);
                    self.dismiss_minibuffer(window, cx);
                    self.open_note_by_path(path, window, cx);
                }
            }
            DelegateKind::ContentSearch => {
                if let Some(candidate) = candidates.get(selected) {
                    let path = std::path::PathBuf::from(&candidate.data);
                    self.dismiss_minibuffer(window, cx);
                    self.open_note_by_path(path, window, cx);
                }
            }
            DelegateKind::Theme => {
                if let Some(candidate) = candidates.get(selected)
                    && let Some(theme) = theme::by_id(&candidate.data)
                {
                    self.dismiss_minibuffer(window, cx);
                    self.select_theme(theme, cx);
                }
            }
            DelegateKind::Item(ref id) => {
                let candidate = candidates.get(selected);
                let id = id.clone();
                let actions = self
                    .focused_item()
                    .handle_confirm(&id, &input, candidate, cx);
                self.process_item_actions(actions, window, cx);
            }
        }
    }

    fn get_note_candidates(&self) -> Vec<Candidate> {
        let results = self.search_notes(&self.minibuffer.input);
        let has_exact = results
            .iter()
            .any(|(t, _)| t.to_lowercase() == self.minibuffer.input.to_lowercase());
        let show_create = !self.minibuffer.input.is_empty() && !has_exact;

        let mut candidates: Vec<Candidate> = results
            .into_iter()
            .map(|(title, path)| {
                let is_pdf = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
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

    fn get_wikilink_candidates(&self) -> Vec<Candidate> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let query = &self.minibuffer.input;
        let matcher = SkimMatcherV2::default();

        let mut entries: Vec<(i64, String, Option<String>, String)> = Vec::new();
        for note in vault
            .contents
            .notes
            .iter()
            .chain(vault.contents.journal.iter())
        {
            if note.path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                continue;
            }
            let score = if query.is_empty() {
                0
            } else if let Some(s) = matcher.fuzzy_match(&note.title, query) {
                s
            } else {
                let mut best = None;
                for a in &note.aliases {
                    if let Some(s) = matcher.fuzzy_match(a, query) {
                        best = Some(best.map_or(s, |b: i64| b.max(s)));
                    }
                }
                match best {
                    Some(s) => s,
                    None => continue,
                }
            };
            entries.push((score, note.title.clone(), None, note.title.clone()));
            for alias in &note.aliases {
                let alias_score = if query.is_empty() {
                    0
                } else {
                    matcher.fuzzy_match(alias, query).unwrap_or(i64::MIN / 2)
                };
                entries.push((
                    alias_score,
                    alias.clone(),
                    Some(format!("alias of {}", note.title)),
                    note.title.clone(),
                ));
            }
        }

        if !query.is_empty() {
            entries.sort_by(|a, b| b.0.cmp(&a.0));
        }

        entries
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, label, detail, data)| Candidate {
                label,
                detail,
                is_action: false,
                data,
            })
            .collect()
    }

    fn get_tag_list_candidates(&self) -> Vec<Candidate> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let query = &self.minibuffer.input;
        let tags = vault.index.all_tags();

        let build = |tag: &str, count: usize| Candidate {
            label: format!("#{}", tag),
            detail: Some(format!(
                "{} note{}",
                count,
                if count == 1 { "" } else { "s" }
            )),
            is_action: false,
            data: tag.to_string(),
        };

        if query.is_empty() {
            return tags
                .into_iter()
                .take(MAX_RESULTS)
                .map(|(t, c)| build(&t, c))
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, String, usize)> = tags
            .into_iter()
            .filter_map(|(t, c)| matcher.fuzzy_match(&t, query).map(|s| (s, t, c)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, t, c)| build(&t, c))
            .collect()
    }

    fn get_tag_notes_candidates(&self, tag: &str) -> Vec<Candidate> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let ids = vault.index.notes_with_tag(tag);
        let query = &self.minibuffer.input;

        let entries: Vec<(String, std::path::PathBuf)> = ids
            .iter()
            .filter_map(|id| vault.index.get(id))
            .map(|m| (m.title.clone(), m.path.clone()))
            .collect();

        let build = |title: &str, path: &std::path::Path| Candidate {
            label: title.to_string(),
            detail: None,
            is_action: false,
            data: path.to_string_lossy().to_string(),
        };

        if query.is_empty() {
            return entries
                .iter()
                .take(MAX_RESULTS)
                .map(|(t, p)| build(t, p))
                .collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &String, &std::path::PathBuf)> = entries
            .iter()
            .filter_map(|(t, p)| matcher.fuzzy_match(t, query).map(|s| (s, t, p)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, t, p)| build(t, p))
            .collect()
    }

    fn get_orphans_candidates(&self) -> Vec<Candidate> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let query = &self.minibuffer.input;
        let orphans: Vec<&str> = vault.index.orphans();

        let build_from = |id: &str| -> Option<Candidate> {
            let m = vault.index.get(id)?;
            Some(Candidate {
                label: m.title.clone(),
                detail: None,
                is_action: false,
                data: m.path.to_string_lossy().to_string(),
            })
        };

        if query.is_empty() {
            return orphans
                .iter()
                .take(MAX_RESULTS)
                .filter_map(|id| build_from(id))
                .collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &str)> = orphans
            .into_iter()
            .filter_map(|id| {
                let m = vault.index.get(id)?;
                matcher.fuzzy_match(&m.title, query).map(|s| (s, id))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .filter_map(|(_, id)| build_from(id))
            .collect()
    }

    fn get_content_search_candidates(&self) -> Vec<Candidate> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let query = self.minibuffer.input.trim();
        if query.is_empty() {
            return vec![];
        }
        let needle = query.to_lowercase();

        let mut hits: Vec<(usize, String, std::path::PathBuf, String)> = Vec::new();
        for note in vault
            .contents
            .notes
            .iter()
            .chain(vault.contents.journal.iter())
        {
            let Ok(body) = std::fs::read_to_string(&note.path) else {
                continue;
            };
            let count = body.to_lowercase().matches(&needle).count();
            if count == 0 {
                continue;
            }
            let snippet = ui_helpers::extract_snippet(&body, &needle, 60);
            hits.push((count, note.title.clone(), note.path.clone(), snippet));
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        hits.into_iter()
            .take(MAX_RESULTS)
            .map(|(count, title, path, snippet)| Candidate {
                label: format!(
                    "{}{}",
                    title,
                    if count > 1 {
                        format!(" ({}×)", count)
                    } else {
                        String::new()
                    }
                ),
                detail: Some(snippet),
                is_action: false,
                data: path.to_string_lossy().to_string(),
            })
            .collect()
    }

    fn insert_wikilink_completion(
        &mut self,
        title: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.active_editor_state();
        editor.update(cx, |state, cx| {
            let cursor = state.cursor;
            if cursor >= 2 {
                let content = state.content();
                if content.get(cursor - 2..cursor) == Some("[[") {
                    let range = (cursor - 2)..cursor;
                    let replacement = format!("[[{}]]", title);
                    let old_text = "[[".to_string();
                    let cursor_before = state.cursor;
                    let selection_before = state.selected_range.clone();

                    state.rope_replace(range.clone(), &replacement);
                    let new_cursor = range.start + replacement.len();

                    state.buffer.record_edit(
                        crate::editor::undo::EditOp {
                            range,
                            old_text,
                            new_text: replacement,
                            cursor_before,
                            cursor_after: new_cursor,
                        },
                        selection_before,
                    );

                    state.cursor = new_cursor;
                    state.selected_range = new_cursor..new_cursor;
                    cx.emit(EditorEvent::Changed);
                    cx.notify();
                }
            }
        });
    }

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

    fn get_vault_open_candidates(&self) -> Vec<Candidate> {
        let input = &self.minibuffer.input;

        if input.is_empty() {
            let recent = self.state.registry.recent_vaults(None);
            return recent
                .into_iter()
                .take(MAX_RESULTS)
                .map(|entry| {
                    let name = std::path::Path::new(&entry.path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("vault");
                    let is_current = self
                        .state
                        .vault
                        .as_ref()
                        .map(|v| v.path.to_string_lossy().to_string() == entry.path)
                        .unwrap_or(false);
                    let suffix = if is_current { "  (current)" } else { "" };
                    Candidate {
                        label: format!("{}{}", name, suffix),
                        detail: Some(entry.path.clone()),
                        is_action: false,
                        data: entry.path.clone(),
                    }
                })
                .collect();
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
                    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
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
                let is_registered = self
                    .state
                    .registry
                    .vault_paths()
                    .iter()
                    .any(|vp| *vp == path);
                let suffix = if is_registered { "  ★" } else { "" };
                candidates.push(Candidate {
                    label: format!("{}/{}", name, suffix),
                    detail: Some(path.to_string_lossy().to_string()),
                    is_action: false,
                    data: path.to_string_lossy().to_string(),
                });
            }
        }

        candidates
    }

    fn get_theme_candidates(&self) -> Vec<Candidate> {
        let query = self.minibuffer.input.to_lowercase();
        theme::THEMES
            .iter()
            .filter(|candidate| {
                query.is_empty()
                    || candidate.name.to_lowercase().contains(&query)
                    || candidate.id.contains(&query)
            })
            .map(|candidate| Candidate {
                label: candidate.name.to_string(),
                detail: Some(if candidate.id == self.theme.id {
                    format!("{} (active)", candidate.description)
                } else {
                    candidate.description.to_string()
                }),
                is_action: candidate.id != self.theme.id,
                data: candidate.id.to_string(),
            })
            .collect()
    }

    fn palette_candidates(&self, query: &str) -> Vec<Candidate> {
        let item_cmds = self.focused_item().commands();
        let global_cmds = &self.global_commands;

        let all_cmds: Vec<&Command> = item_cmds.iter().chain(global_cmds.iter()).collect();

        if query.is_empty() {
            return all_cmds
                .iter()
                .take(MAX_RESULTS)
                .map(|c| command_registry::command_to_candidate(c))
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &Command)> = all_cmds
            .iter()
            .filter_map(|c| {
                let scores = [
                    matcher.fuzzy_match(c.name, query),
                    matcher.fuzzy_match(c.description, query),
                    matcher.fuzzy_match(c.id, query),
                ];
                let alias_score = c
                    .aliases
                    .iter()
                    .filter_map(|a| matcher.fuzzy_match(a, query))
                    .max();
                let best = scores.into_iter().flatten().chain(alias_score).max();
                best.map(|score| (score, *c))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, c)| command_registry::command_to_candidate(c))
            .collect()
    }
}
