use gpui::*;
use gpui_component::{h_flex, v_flex};

mod command_registry;
mod minibuffer_controller;
mod minibuffer_view;
mod mode_line;
mod resource;
mod title_bar;
mod ui_helpers;
mod workspace_controller;

use crate::command::Command;
use crate::document::Document;
use crate::editor::{EditorEvent, EditorState, EditorView, EditorViewEvent};
use crate::minibuffer::{DelegateKind, Minibuffer};
use crate::pane::{ActiveItem, CommandOutcome, ItemAction, VimSnapshot};
use crate::state::AppState;
use crate::theme::{self, Theme};
use crate::workspace::{BufferId, Workspace, WorkspaceDisplay, WorkspaceFocus};

use resource::{
    BufferContent, ResourceKey, SecondaryContent, is_diagram_link, is_pdf_path, parse_pdf_link,
    unique_attachment_path,
};

const MAX_RESULTS: usize = 15;

// App-wide actions. Registered as gpui actions so they work regardless of
// which view has focus. Keybindings are wired up in `src/main.rs`.
actions!(
    memex,
    [
        Save,
        FindNote,
        CommandPalette,
        ToggleVim,
        FocusLeftPane,
        FocusRightPane,
        SearchContent,
        ToggleBacklinks,
        ToggleSecondaryMaximize
    ]
);

pub struct Memex {
    state: AppState,
    editor_state: Entity<EditorState>,
    editor_view: Entity<EditorView>,
    workspace: Workspace<ResourceKey, BufferContent>,
    editor_item: ActiveItem,
    secondary: Option<SecondaryContent>,
    editor_buffer: BufferId,
    minibuffer: Minibuffer,
    minibuffer_focus: FocusHandle,
    global_commands: Vec<Command>,
    /// Filesystem watcher for the currently-open vault. Reseated on
    /// every vault switch; a polling task spawned in `new` drains its
    /// event channel and calls `refresh` on the active vault.
    vault_watcher: Option<crate::vault::VaultWatcher>,
    theme: Theme,
    _subscriptions: Vec<Subscription>,
}

impl Memex {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut state = AppState::new();

        let initial_document = state.restore_document().unwrap_or_else(|| {
            Document::scratch(
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
                    .to_string(),
            )
        });

        let editor_state = cx.new(|cx| EditorState::from_document(initial_document, cx));
        if let Some(vault) = state.vault.as_ref() {
            let titles = vault.index.wikilink_titles();
            editor_state.update(cx, |editor, cx| editor.set_wikilink_titles(titles, cx));
        }
        let theme = theme::by_id(&state.config.theme).unwrap_or(theme::SOLARIZED_LIGHT);
        let editor_width = state.config.editor_width;
        let editor_view =
            cx.new(|cx| EditorView::new(editor_state.clone(), theme, editor_width, cx));

        // The editor owns its own keymap and dispatches keys internally.
        // It only emits events for things the app shell must handle:
        // commands that open the minibuffer, item actions that need clipboard /
        // minibuffer access, and vim-state changes that refresh the mode-line.
        let editor_key_sub = cx.subscribe_in(
            &editor_view,
            window,
            |this, _view, ev: &EditorViewEvent, window, cx| match ev {
                EditorViewEvent::Command(cmd_id, count) => {
                    this.execute_command(cmd_id, "", *count, window, cx);
                }
                EditorViewEvent::ItemActions(actions) => {
                    this.process_item_actions(actions.clone(), window, cx);
                }
                EditorViewEvent::VimStateChanged => {
                    cx.notify();
                }
            },
        );

        let editor_sub = cx.subscribe_in(
            &editor_state,
            window,
            |this, _entity, ev: &EditorEvent, window, cx| {
                match ev {
                    EditorEvent::Changed => {
                        // Clear stale minibuffer messages on editor activity
                        this.minibuffer.message = None;
                        cx.notify();
                    }
                    EditorEvent::RequestSave => {
                        this.save(window, cx);
                        this.minibuffer.set_message("Written");
                    }
                    EditorEvent::RequestQuit => {
                        this.close_window(window, cx);
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
                    EditorEvent::WikilinkClicked(title) => {
                        this.follow_wikilink(title.clone(), window, cx);
                    }
                    EditorEvent::WikilinkAutocomplete => {
                        this.activate_wikilink_autocomplete(window, cx);
                    }
                }
            },
        );

        let editor_resource = editor_state
            .read(cx)
            .document_path()
            .map(ResourceKey::Markdown)
            .unwrap_or_else(|| {
                ResourceKey::Scratch(
                    state
                        .vault
                        .as_ref()
                        .map(|vault| vault.path.clone())
                        .unwrap_or_default(),
                )
            });
        let editor_item = ActiveItem::Editor {
            state: editor_state.clone(),
            view: editor_view.clone(),
        };
        let workspace = Workspace::new(
            editor_resource,
            BufferContent::Markdown(editor_state.read(cx).buffer.clone()),
        );
        let editor_buffer = workspace.editor_buffer;

        let mut this = Self {
            state,
            editor_state: editor_state.clone(),
            editor_view: editor_view.clone(),
            workspace,
            editor_item,
            secondary: None,
            editor_buffer,
            minibuffer: Minibuffer::new(),
            minibuffer_focus: cx.focus_handle(),
            global_commands: command_registry::global_commands(),
            vault_watcher: None,
            theme,
            _subscriptions: vec![editor_sub, editor_key_sub],
        };

        // If a vault was restored, start watching it.
        this.start_vault_watcher();

        // Drain watcher events every 250ms on the foreground executor.
        // try_recv is non-blocking so this stays cheap.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let should_refresh = cx
                    .update(|cx| {
                        this.update(cx, |memex, _| {
                            let Some(watcher) = memex.vault_watcher.as_ref() else {
                                return false;
                            };
                            let mut got_any = false;
                            while let Ok(_batch) = watcher.events.try_recv() {
                                got_any = true;
                            }
                            got_any
                        })
                        .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if should_refresh {
                    let _ = cx.update(|cx| {
                        this.update(cx, |memex, cx| {
                            let titles = if let Some(v) = memex.state.vault.as_mut() {
                                let _ = v.refresh();
                                Some(v.index.wikilink_titles())
                            } else {
                                None
                            };
                            if let Some(titles) = titles {
                                memex.editor_state.update(cx, |editor, cx| {
                                    editor.set_wikilink_titles(titles, cx);
                                });
                            }
                            cx.notify();
                        })
                        .ok();
                    });
                }
            }
        })
        .detach();

        this
    }

    /// Create a read-only snapshot of keymap state for item dispatch.
    /// Reads from the editor view — the editor owns the vim state.
    fn vim_snapshot(&self, cx: &App) -> VimSnapshot {
        self.active_editor_view().read(cx).vim_snapshot()
    }

    /// Returns whether vim mode is enabled. Editor-owned.
    fn vim_enabled(&self, cx: &App) -> bool {
        self.active_editor_view().read(cx).keymap.vim_enabled
    }

    fn current_document_path(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.active_editor_state().read(cx).document_path()
    }

    fn current_document_title(&self, cx: &App) -> String {
        self.state
            .document_title(self.current_document_path(cx).as_deref())
    }

    fn current_document_dirty(&self, cx: &App) -> bool {
        self.active_editor_state().read(cx).is_dirty()
    }

    fn focused_item(&self) -> &ActiveItem {
        if self.workspace.focus == WorkspaceFocus::Secondary {
            if let Some(SecondaryContent::Item { item, .. }) = &self.secondary {
                return item;
            }
        }
        &self.editor_item
    }

    fn active_editor_state(&self) -> Entity<EditorState> {
        self.editor_state.clone()
    }

    fn active_editor_view(&self) -> Entity<EditorView> {
        self.editor_view.clone()
    }

    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.focus = WorkspaceFocus::Editor;
        self.editor_item.focus(window, cx);
        cx.notify();
    }

    fn focus_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.secondary.is_none() {
            return;
        }
        self.workspace.focus = WorkspaceFocus::Secondary;
        if let Some(SecondaryContent::Item { item, .. }) = &self.secondary {
            item.focus(window, cx);
        }
        cx.notify();
    }

    fn render_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = div()
            .id("editor-slot")
            .flex_1()
            .size_full()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_editor(window, cx)),
            )
            .child(self.editor_item.view_element());
        let secondary = self.secondary.as_ref().map(|content| {
            let child = match content {
                SecondaryContent::Item { item, .. } => item.view_element().into_any_element(),
            };
            div()
                .id("secondary-slot")
                .flex_1()
                .size_full()
                .overflow_hidden()
                .border_l_1()
                .border_color(rgb(self.theme.border))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.focus_secondary(window, cx)),
                )
                .child(child)
                .into_any_element()
        });
        match (self.workspace.display, secondary) {
            (WorkspaceDisplay::EditorOnly, _) | (_, None) => editor.into_any_element(),
            (WorkspaceDisplay::SideBySide, Some(secondary)) => h_flex()
                .flex_1()
                .size_full()
                .overflow_hidden()
                .child(editor)
                .child(secondary)
                .into_any_element(),
            (WorkspaceDisplay::SecondaryOnly, Some(secondary)) => secondary,
        }
    }

    fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        self.editor_item.set_theme(theme, cx);
        if let Some(SecondaryContent::Item { item, .. }) = &self.secondary {
            item.set_theme(theme, cx);
        }
        self.minibuffer
            .set_message(format!("Theme: {}", theme.name));
        cx.notify();
    }

    fn select_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        match crate::config::save_theme(theme.id) {
            Ok(path) => {
                self.state.config.theme = theme.id.to_string();
                self.apply_theme(theme, cx);
                self.minibuffer
                    .set_message(format!("Theme: {} ({})", theme.name, path.display()));
            }
            Err(error) => {
                self.minibuffer
                    .set_message(format!("Failed to save theme: {error}"));
                cx.notify();
            }
        }
    }

    /// Open or create today's journal note at `journal/YYYY-MM-DD.md`.
    fn open_or_create_journal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vault = match self.state.vault.as_ref() {
            Some(v) => v,
            None => {
                self.minibuffer.set_message("No vault open");
                cx.notify();
                return;
            }
        };
        let layout = vault.layout();
        // ISO date prefix: first 10 chars of `YYYY-MM-DDTHH:MM:SSZ`.
        let iso = crate::vault::id::iso_now();
        let date = &iso[..10];
        let path = layout.journal_path(date);

        if !path.exists() {
            let mut fm = crate::vault::Frontmatter::default();
            fm.id = Some(crate::vault::id::generate());
            fm.title = Some(date.to_string());
            fm.created = Some(iso.clone());
            let body = format!("# {}\n\n", date);
            let content = match crate::vault::frontmatter::write(&fm, &body) {
                Ok(c) => c,
                Err(e) => {
                    self.minibuffer
                        .set_message(format!("journal write failed: {}", e));
                    cx.notify();
                    return;
                }
            };
            if let Err(e) = crate::fs::save_note(&path, &content) {
                self.minibuffer
                    .set_message(format!("journal create failed: {}", e));
                cx.notify();
                return;
            }
            // Reflect the new file in the index.
            if let Some(v) = self.state.vault.as_mut() {
                let _ = v.refresh();
            }
        }

        self.open_note_by_path(path, window, cx);
    }

    /// Rename the current note's title. Updates `title:` in frontmatter
    /// and appends the previous title to `aliases:` so existing
    /// `[[old title]]` wikilinks keep resolving. The filename does not
    /// change — IDs stay stable.
    fn rename_current_note(
        &mut self,
        new_title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.current_document_path(cx) else {
            self.minibuffer.set_message("No note open");
            cx.notify();
            return;
        };

        // Use the editor's in-memory content (may have unsaved edits).
        let editor = self.active_editor_state();
        let content = editor.read(cx).content();
        let parsed = match crate::vault::frontmatter::parse(&content) {
            Ok(p) => p,
            Err(e) => {
                self.minibuffer.set_message(format!("rename: {}", e));
                cx.notify();
                return;
            }
        };
        let mut fm = parsed.frontmatter.unwrap_or_default();
        let old_title = fm.title.clone();
        if let Some(old) = &old_title {
            if !old.is_empty() && old != new_title && !fm.aliases.iter().any(|a| a == old) {
                fm.aliases.push(old.clone());
            }
        }
        fm.title = Some(new_title.to_string());

        let new_content = match crate::vault::frontmatter::write(&fm, &parsed.body) {
            Ok(s) => s,
            Err(e) => {
                self.minibuffer.set_message(format!("rename: {}", e));
                cx.notify();
                return;
            }
        };

        // Write to disk and update the live editor buffer.
        if let Err(e) = crate::fs::save_note(&path, &new_content) {
            self.minibuffer.set_message(format!("rename: {}", e));
            cx.notify();
            return;
        }
        editor.update(cx, |state, cx| {
            state.set_content(new_content.clone(), window, cx);
        });
        if let Some(v) = self.state.vault.as_mut() {
            let _ = v.refresh();
            let titles = v.index.wikilink_titles();
            editor.update(cx, |state, cx| state.set_wikilink_titles(titles, cx));
        }
        self.minibuffer
            .set_message(format!("Renamed to '{}'", new_title));
        cx.notify();
    }

    /// Read the system clipboard. If it contains an image, save it under
    /// `attachments/{timestamp}.{ext}` and insert `![[filename]]` at the
    /// cursor. If it contains a file path that exists, copy the file into
    /// `attachments/` and link to it. Otherwise shows an error message.
    fn attach_from_clipboard(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.state.vault.as_ref() else {
            self.minibuffer.set_message("No vault open");
            cx.notify();
            return;
        };
        let attachments_dir = vault.layout().attachments;
        if let Err(e) = std::fs::create_dir_all(&attachments_dir) {
            self.minibuffer.set_message(format!("attach: {}", e));
            cx.notify();
            return;
        }

        let Some(item) = cx.read_from_clipboard() else {
            self.minibuffer.set_message("Clipboard is empty");
            cx.notify();
            return;
        };

        let timestamp = crate::vault::id::iso_now()
            .replace(':', "")
            .replace('-', "");
        // Trim the trailing Z so the filename doesn't carry timezone noise.
        let ts_clean = timestamp.trim_end_matches('Z');

        // Try image first. Fall back to treating string content as a path.
        let (filename, bytes) = if let Some(image) = item.entries().iter().find_map(|e| {
            if let ClipboardEntry::Image(img) = e {
                Some(img.clone())
            } else {
                None
            }
        }) {
            let ext = match image.format {
                ImageFormat::Png => "png",
                ImageFormat::Jpeg => "jpg",
                ImageFormat::Webp => "webp",
                ImageFormat::Gif => "gif",
                ImageFormat::Bmp => "bmp",
                ImageFormat::Tiff => "tiff",
                ImageFormat::Svg => "svg",
            };
            (format!("{}.{}", ts_clean, ext), image.bytes.clone())
        } else if let Some(text) = item.text() {
            let src = std::path::PathBuf::from(text.trim());
            if src.is_file() {
                let bytes = match std::fs::read(&src) {
                    Ok(b) => b,
                    Err(e) => {
                        self.minibuffer.set_message(format!("attach: {}", e));
                        cx.notify();
                        return;
                    }
                };
                let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin");
                let stem = src
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("attachment");
                (format!("{}-{}.{}", ts_clean, stem, ext), bytes)
            } else {
                self.minibuffer
                    .set_message("Clipboard has no image or file path");
                cx.notify();
                return;
            }
        } else {
            self.minibuffer.set_message("Clipboard has no image");
            cx.notify();
            return;
        };

        let dest = attachments_dir.join(&filename);
        if let Err(e) = std::fs::write(&dest, &bytes) {
            self.minibuffer
                .set_message(format!("attach write failed: {}", e));
            cx.notify();
            return;
        }

        // Insert at the cursor. Use embed syntax `![[…]]` so eventual
        // inline-image rendering can pick it up.
        let snippet = format!("![[{}]]", filename);
        self.active_editor_state().update(cx, |state, cx| {
            state.edit_text(&snippet, cx);
        });

        // Refresh the vault so the attachment shows in contents.
        if let Some(v) = self.state.vault.as_mut() {
            let _ = v.refresh();
        }
        self.minibuffer
            .set_message(format!("Attached {}", filename));
        cx.notify();
    }

    fn attach_dropped_files(
        &mut self,
        paths: &[std::path::PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.state.vault.as_ref() else {
            self.minibuffer.set_message("No vault open");
            cx.notify();
            return;
        };
        let attachments_dir = vault.layout().attachments;
        if let Err(e) = std::fs::create_dir_all(&attachments_dir) {
            self.minibuffer.set_message(format!("drop attach: {}", e));
            cx.notify();
            return;
        }

        let mut copied = Vec::new();
        let mut skipped = 0usize;
        let canonical_attachments_dir = attachments_dir.canonicalize().ok();
        for src in paths {
            if !src.is_file() || !is_pdf_path(src) {
                skipped += 1;
                continue;
            }

            if let (Some(attachments_dir), Ok(src)) =
                (&canonical_attachments_dir, src.canonicalize())
                && src.starts_with(attachments_dir)
            {
                copied.push(src);
                continue;
            }

            let Some(filename) = src.file_name() else {
                skipped += 1;
                continue;
            };
            let dest = unique_attachment_path(&attachments_dir, filename);
            if let Err(e) = std::fs::copy(src, &dest) {
                self.minibuffer.set_message(format!("drop attach: {}", e));
                cx.notify();
                return;
            }
            copied.push(dest);
        }

        if copied.is_empty() {
            let message = if skipped == 0 {
                "No files dropped".to_string()
            } else {
                "Drop a PDF to add it to attachments".to_string()
            };
            self.minibuffer.set_message(message);
            cx.notify();
            return;
        }

        if let Some(vault) = self.state.vault.as_mut() {
            let _ = vault.refresh();
        }

        let open_path = copied.last().expect("copied is non-empty").clone();
        let count = copied.len();
        let name = open_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("PDF")
            .to_string();
        self.open_pdf(open_path, window, cx);
        self.minibuffer.set_message(if count == 1 {
            format!("Attached {}", name)
        } else {
            format!("Attached {} PDFs, opened {}", count, name)
        });
        cx.notify();
    }

    /// Insert a bullet list of wikilinks to every note with the given tag.
    /// The MOC helper — lets you build hub pages without hunting titles.
    fn insert_links_by_tag(&mut self, tag: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.state.vault.as_ref() else {
            self.minibuffer.set_message("No vault open");
            cx.notify();
            return;
        };
        let ids = vault.index.notes_with_tag(tag).to_vec();
        if ids.is_empty() {
            self.minibuffer
                .set_message(format!("No notes tagged #{}", tag));
            cx.notify();
            return;
        }
        let mut titles: Vec<String> = ids
            .iter()
            .filter_map(|id| vault.index.get(id).map(|m| m.title.clone()))
            .collect();
        titles.sort();
        let block = titles
            .iter()
            .map(|t| format!("- [[{}]]\n", t))
            .collect::<String>();

        // Insert at the cursor via the editor's edit API.
        self.active_editor_state().update(cx, |state, cx| {
            state.edit_text(&block, cx);
        });
        let _ = window;
        self.minibuffer
            .set_message(format!("Inserted {} links", titles.len()));
        cx.notify();
    }

    /// Follow a [[wikilink]]: open the note if it exists, create it otherwise.
    fn follow_wikilink(&mut self, title: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((filename, target)) = parse_pdf_link(&title) {
            let path = self.state.vault.as_ref().and_then(|vault| {
                vault
                    .contents
                    .attachments
                    .iter()
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.eq_ignore_ascii_case(filename))
                    })
                    .cloned()
            });
            match path {
                Some(path) => self.open_pdf_target(path, target, window, cx),
                None => {
                    self.minibuffer
                        .set_message(format!("PDF not found: {}", filename));
                    cx.notify();
                }
            }
            return;
        }
        if is_diagram_link(&title) {
            let path = self
                .state
                .vault
                .as_ref()
                .map(|vault| vault.layout().diagram_path(&title));
            match path {
                Some(path) if path.exists() => self.open_diagram(path, window, cx),
                _ => {
                    self.minibuffer
                        .set_message(format!("Diagram not found: {}", title));
                    cx.notify();
                }
            }
            return;
        }
        // Search for a matching note in the vault
        if let Some(vault) = &self.state.vault {
            if let Some(crate::vault::ResolveHit::Unique(id)) = vault.index.resolve_link(&title)
                && let Some(note) = vault.index.get(id)
            {
                let path = note.path.clone();
                self.open_note_by_path(path, window, cx);
                return;
            }
        }
        // No match — create the note
        self.create_note_by_title(&title, window, cx);
        self.minibuffer
            .set_message(format!("Created \"{}\"", title));
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
                ItemAction::SetMessage(msg) => self.minibuffer.set_message(msg),
                ItemAction::ActivateDelegate {
                    id,
                    prompt,
                    highlight_input: _,
                } => {
                    let vim = self.vim_enabled(cx);
                    self.minibuffer
                        .activate(DelegateKind::Item(id), &prompt, vim);
                    self.minibuffer_focus.focus(window);
                }
                ItemAction::Dismiss => {
                    self.dismiss_minibuffer(window, cx);
                }
                ItemAction::WriteClipboard(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                ItemAction::Yank(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    self.editor_state
                        .update(cx, |state, _| state.set_yank_register(text.clone()));
                }
                ItemAction::SetVimMode(mode) => {
                    self.active_editor_view().update(cx, |view, cx| {
                        view.keymap.set_vim_mode(mode);
                        view.state
                            .update(cx, |s, cx| s.on_vim_mode_changed(mode, cx));
                        view.sync_state_vim_flags(cx);
                        cx.emit(EditorViewEvent::VimStateChanged);
                    });
                }
                ItemAction::PushTransient(transient) => {
                    self.active_editor_view().update(cx, |view, _cx| {
                        view.keymap.push_transient(transient);
                    });
                }
                ItemAction::SetVimEnabled(enabled) => {
                    self.active_editor_view().update(cx, |view, cx| {
                        view.set_vim_enabled(enabled, cx);
                    });
                }
                ItemAction::SyncVimFlags => {
                    self.active_editor_view().update(cx, |view, cx| {
                        view.sync_state_vim_flags(cx);
                    });
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
                self.close_window(window, cx);
            }
            "wq" => {
                self.save(window, cx);
                self.close_window(window, cx);
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
            "diagram-new" | "new-diagram" => {
                self.new_diagram(window, cx);
            }
            "backlinks" => {
                self.show_backlinks(window, cx);
            }
            "toggle-secondary-maximize" | "secondary-maximize" => {
                if self.workspace.toggle_secondary_maximized() {
                    cx.notify();
                }
            }
            "close-secondary" => {
                self.close_secondary(window, cx);
            }
            "today" | "daily" | "journal" => {
                self.open_or_create_journal(window, cx);
            }
            "tags" => {
                self.activate_tag_list(window, cx);
            }
            "tag" => {
                // `:tag foo` drills straight in; bare `:tag` opens the picker.
                let arg = raw_input.strip_prefix("tag ").unwrap_or("").trim();
                if arg.is_empty() {
                    self.activate_tag_list(window, cx);
                } else {
                    self.activate_tag_notes(arg, window, cx);
                }
            }
            "orphans" => {
                self.activate_orphans(window, cx);
            }
            "search-content" | "search" | "grep" => {
                self.activate_content_search(window, cx);
            }
            "theme" => {
                self.activate_theme_picker(window, cx);
            }
            "rename" | "rn" => {
                let arg = raw_input
                    .strip_prefix("rename ")
                    .or_else(|| raw_input.strip_prefix("rn "))
                    .unwrap_or("")
                    .trim();
                if arg.is_empty() {
                    self.minibuffer.set_message("usage: :rename <new title>");
                } else {
                    self.rename_current_note(arg, window, cx);
                }
            }
            "insert-links-by-tag" | "moc" => {
                let arg = raw_input
                    .strip_prefix("insert-links-by-tag ")
                    .or_else(|| raw_input.strip_prefix("moc "))
                    .unwrap_or("")
                    .trim();
                if arg.is_empty() {
                    self.minibuffer.set_message("usage: :moc <tag>");
                } else {
                    self.insert_links_by_tag(arg, window, cx);
                }
            }
            "toggle-backlinks" | "backlinks-panel" => {
                if matches!(
                    &self.secondary,
                    Some(SecondaryContent::Item { item }) if item.is_backlinks()
                ) {
                    self.close_secondary(window, cx);
                } else {
                    self.show_backlinks(window, cx);
                }
            }
            "attach" | "paste-image" => {
                self.attach_from_clipboard(window, cx);
            }
            "vault-forget" | "forget-vault" => {
                let arg = raw_input
                    .strip_prefix("vault-forget ")
                    .or_else(|| raw_input.strip_prefix("forget-vault "))
                    .unwrap_or("")
                    .trim();
                if arg.is_empty() {
                    self.minibuffer.set_message("usage: :vault-forget <path>");
                } else {
                    let path = std::path::PathBuf::from(arg);
                    let removed = self.state.registry.forget_vault(&path);
                    let _ = self.state.registry.save();
                    if removed {
                        self.minibuffer
                            .set_message(format!("Forgot vault: {}", arg));
                    } else {
                        self.minibuffer
                            .set_message(format!("Not in registry: {}", arg));
                    }
                    cx.notify();
                }
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
                let vim = self.vim_snapshot(cx);

                let focused_is_editor = self.focused_item().is_editor();
                let outcome = self
                    .focused_item()
                    .execute_command(cmd_id, (vw, vh), vim, cx);
                if let CommandOutcome::Handled(actions) = outcome {
                    self.process_item_actions(actions, window, cx);
                } else if focused_is_editor {
                    // Editor commands that need window access (editing, motions, etc.)
                    let vim = self.vim_snapshot(cx);
                    let editor = self
                        .focused_item()
                        .editor_state()
                        .expect("focused editor item must expose editor state");
                    let item_actions = editor.update(cx, |state, ecx| {
                        state.execute_command_by_id(cmd_id, count, vim, window, ecx)
                    });
                    self.process_item_actions(item_actions, window, cx);
                    if let Some(msg) = editor.read(cx).status_message.clone() {
                        self.minibuffer.set_message(msg);
                    }
                }
            }
        }
        cx.notify();
    }
}

impl Render for Memex {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let minibuffer_active = self.minibuffer.active;

        let mut root = v_flex()
            .id("memex-root")
            .size_full()
            .bg(rgb(self.theme.background))
            .font_family("FiraCode Nerd Font")
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.attach_dropped_files(paths.paths(), window, cx);
            }))
            // App-wide actions — work regardless of which view has focus.
            .on_action(cx.listener(|this, _: &Save, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.save(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindNote, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.activate_note_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.activate_command_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleVim, _window, cx| {
                if this.minibuffer.active {
                    return;
                }
                let new_enabled = !this.vim_enabled(cx);
                let editor_view = this.active_editor_view();
                editor_view.update(cx, |view, cx| {
                    view.set_vim_enabled(new_enabled, cx);
                });
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusLeftPane, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.focus_editor(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusRightPane, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.focus_secondary(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SearchContent, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.activate_content_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleBacklinks, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                if matches!(
                    &this.secondary,
                    Some(SecondaryContent::Item { item }) if item.is_backlinks()
                ) {
                    this.close_secondary(window, cx);
                } else {
                    this.show_backlinks(window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &ToggleSecondaryMaximize, _window, cx| {
                    if this.minibuffer.active {
                        return;
                    }
                    if this.workspace.toggle_secondary_maximized() {
                        cx.notify();
                    }
                }),
            )
            // Custom title bar with drag + window controls
            .child(self.render_title_bar(cx))
            .child(self.render_workspace(cx))
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
                    .bg(rgba(0x00000000)) // transparent — click-to-dismiss only, no dimming
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
