use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;
use gpui_component::{Icon, IconName, h_flex, v_flex};

use crate::command::Command;
use crate::document::Document;
use crate::editor::{EditorBuffer, EditorEvent, EditorState, EditorView, EditorViewEvent};
use crate::graph::{GraphEvent, GraphState, GraphView, GraphViewEvent};
use crate::minibuffer::{Candidate, DelegateKind, Minibuffer, MinibufferAction, MinibufferVimMode};
use crate::pane::{ActiveItem, CommandOutcome, ItemAction, VimSnapshot};
use crate::pdf::{PdfState, PdfView, PdfViewEvent};
use crate::state::AppState;
use crate::theme::{self, Theme};
use crate::workspace::{BufferId, SplitAxis, WindowId, WindowLayout, WindowStore, Workspace};

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
        ToggleBacklinks
    ]
);

pub struct Memex {
    state: AppState,
    editor_state: Entity<EditorState>,
    editor_view: Entity<EditorView>,
    workspace: Workspace<ResourceKey, BufferContent>,
    window_items: WindowStore<ActiveItem>,
    editor_buffer: BufferId,
    editor_window: WindowId,
    minibuffer: Minibuffer,
    minibuffer_focus: FocusHandle,
    global_commands: Vec<Command>,
    /// Filesystem watcher for the currently-open vault. Reseated on
    /// every vault switch; a polling task spawned in `new` drains its
    /// event channel and calls `refresh` on the active vault.
    vault_watcher: Option<crate::vault::VaultWatcher>,
    /// Whether the backlinks panel is visible below the editor.
    show_backlinks: bool,
    theme: Theme,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResourceKey {
    Scratch(std::path::PathBuf),
    Markdown(std::path::PathBuf),
    Pdf(std::path::PathBuf),
    Graph(std::path::PathBuf),
}

#[derive(Clone)]
enum BufferContent {
    Markdown(EditorBuffer),
    Pdf(std::path::PathBuf),
    Graph(std::path::PathBuf),
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
        let theme = theme::by_id(&state.config.theme).unwrap_or(theme::SOLARIZED_LIGHT);
        let editor_view = cx.new(|cx| EditorView::new(editor_state.clone(), theme, cx));

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
        let editor_buffer = workspace.focused_buffer();
        let editor_window = workspace.focused_window;
        let mut window_items = WindowStore::default();
        window_items.insert(editor_window, editor_item);

        let mut this = Self {
            state,
            editor_state: editor_state.clone(),
            editor_view: editor_view.clone(),
            workspace,
            window_items,
            editor_buffer,
            editor_window,
            minibuffer: Minibuffer::new(),
            minibuffer_focus: cx.focus_handle(),
            global_commands: Self::global_commands(),
            vault_watcher: None,
            show_backlinks: false,
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
                            if let Some(v) = memex.state.vault.as_mut() {
                                let _ = v.refresh();
                                cx.notify();
                            }
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

    fn item_for_window(&self, window: WindowId) -> &ActiveItem {
        self.window_items
            .get(window)
            .expect("live window must have presentation state")
    }

    fn focused_item(&self) -> &ActiveItem {
        self.item_for_window(self.workspace.focused_window)
    }

    fn active_editor_state(&self) -> Entity<EditorState> {
        self.focused_item()
            .editor_state()
            .unwrap_or_else(|| self.editor_state.clone())
    }

    fn active_editor_view(&self) -> Entity<EditorView> {
        self.focused_item()
            .editor_view()
            .unwrap_or_else(|| self.editor_view.clone())
    }

    fn secondary_window(&self) -> Option<WindowId> {
        self.workspace
            .layout
            .window_ids()
            .into_iter()
            .find(|id| *id != self.editor_window)
    }

    fn focus_workspace_window(
        &mut self,
        id: WindowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.focus(id) {
            self.item_for_window(id).focus(window, cx);
            cx.notify();
        }
    }

    fn render_window_layout(&self, layout: &WindowLayout, cx: &mut Context<Self>) -> AnyElement {
        match layout {
            WindowLayout::Window(workspace_window) => {
                let id = workspace_window.id;
                let view = self.item_for_window(id).view_element();
                div()
                    .id(ElementId::Name(format!("workspace-window-{:?}", id).into()))
                    .flex_1()
                    .size_full()
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                            this.focus_workspace_window(id, window, cx);
                        }),
                    )
                    .child(view)
                    .into_any_element()
            }
            WindowLayout::Split { axis, children, .. } => {
                let children = children
                    .iter()
                    .map(|child| self.render_window_layout(child, cx))
                    .collect::<Vec<_>>();
                match axis {
                    SplitAxis::Horizontal => h_flex()
                        .flex_1()
                        .size_full()
                        .overflow_hidden()
                        .children(children)
                        .into_any_element(),
                    SplitAxis::Vertical => v_flex()
                        .flex_1()
                        .size_full()
                        .overflow_hidden()
                        .children(children)
                        .into_any_element(),
                }
            }
        }
    }

    fn activate_note_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::NoteSearch, "Find note:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_split_note_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::SplitNoteSearch, "Split open:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_vault_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::VaultSwitch, "Switch vault:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_vault_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::VaultOpen, "Open vault:", vim);
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
        let vim = self.vim_enabled(cx);
        let prompt = if vim { ":" } else { "M-x" };
        self.minibuffer.activate(DelegateKind::Command, prompt, vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_wikilink_autocomplete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::WikilinkAutocomplete, "Link to:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_backlinks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::Backlinks, "Backlinks:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_tag_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer.activate(DelegateKind::TagList, "Tag:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_tag_notes(&mut self, tag: &str, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer.activate(
            DelegateKind::TagNotes(tag.to_string()),
            &format!("#{}:", tag),
            vim,
        );
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_orphans(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::Orphans, "Orphans:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_content_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer
            .activate(DelegateKind::ContentSearch, "Search:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn activate_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vim = self.vim_enabled(cx);
        self.minibuffer.activate(DelegateKind::Theme, "Theme:", vim);
        self.minibuffer_focus.focus(window);
        cx.notify();
    }

    fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        for item in self.window_items.values() {
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
        // Search for a matching note in the vault
        if let Some(vault) = &self.state.vault {
            let titles = vault.note_titles();
            let target_lower = title.to_lowercase();
            if let Some((_, path)) = titles
                .iter()
                .find(|(t, _)| t.to_lowercase() == target_lower)
            {
                let path = path.clone();
                self.open_note_by_path(path, window, cx);
                return;
            }
        }
        // No match — create the note
        self.create_note_by_title(&title, window, cx);
        self.minibuffer
            .set_message(format!("Created \"{}\"", title));
    }

    fn dismiss_minibuffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.minibuffer.dismiss();
        self.focused_item().focus(window, cx);
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
        let action = self
            .minibuffer
            .handle_key(key, ctrl, shift, candidates.len());

        match action {
            MinibufferAction::Updated => {
                // Notify active item of input changes for item-owned delegates
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
            DelegateKind::Command => self.palette_candidates(&self.minibuffer.input),
            DelegateKind::NoteSearch => self.get_note_candidates(),
            DelegateKind::SplitNoteSearch => self.get_note_candidates(),
            DelegateKind::WikilinkAutocomplete => self.get_wikilink_candidates(),
            DelegateKind::Backlinks => self.get_backlink_candidates(cx),
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
            DelegateKind::SplitNoteSearch => {
                if let Some(candidate) = candidates.get(selected) {
                    if !candidate.is_action {
                        let path = std::path::PathBuf::from(&candidate.data);
                        self.dismiss_minibuffer(window, cx);
                        self.open_in_split_by_path(path, window, cx);
                    }
                }
            }
            DelegateKind::WikilinkAutocomplete => {
                if let Some(candidate) = candidates.get(selected) {
                    let title = candidate.data.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.insert_wikilink_completion(&title, window, cx);
                } else if !input.is_empty() {
                    // Use typed text as-is
                    let title = input.clone();
                    self.dismiss_minibuffer(window, cx);
                    self.insert_wikilink_completion(&title, window, cx);
                }
            }
            DelegateKind::Backlinks => {
                if let Some(candidate) = candidates.get(selected) {
                    let path = std::path::PathBuf::from(&candidate.data);
                    self.dismiss_minibuffer(window, cx);
                    self.open_note_by_path(path, window, cx);
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
                ItemAction::ActivateLayer(layer_id) => {
                    // Editor-owned layers now — route into the editor view.
                    self.active_editor_view().update(cx, |view, cx| {
                        view.keymap.stack.activate_layer(layer_id);
                        view.state
                            .update(cx, |s, cx| s.on_layer_activated(layer_id, cx));
                        view.sync_state_vim_flags(cx);
                        cx.emit(EditorViewEvent::VimStateChanged);
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
            "split" => {
                self.split_focused_window(SplitAxis::Vertical, window, cx);
            }
            "vsplit" => {
                self.split_focused_window(SplitAxis::Horizontal, window, cx);
            }
            "split-open" => {
                self.activate_split_note_search(window, cx);
            }
            "only" | "only-window" => {
                if self.workspace.only_focused() {
                    let focused = self.workspace.focused_window;
                    self.window_items.retain_only(focused);
                    cx.notify();
                }
            }
            "backlinks" => {
                self.activate_backlinks(window, cx);
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
                self.show_backlinks = !self.show_backlinks;
                cx.notify();
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

    /// Build candidates for wikilink autocomplete — vault note titles only (no PDFs, no "create").
    fn get_wikilink_candidates(&self) -> Vec<Candidate> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let query = &self.minibuffer.input;
        let matcher = SkimMatcherV2::default();

        // Titles — canonical names.
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
                // Try aliases before giving up on this note.
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
            // Prefer the canonical title but allow alias hints in detail.
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

    /// Build candidates for backlinks — notes that link to the current note.
    fn get_backlink_candidates(&self, cx: &App) -> Vec<Candidate> {
        let current_title = self.current_document_title(cx);
        if current_title.is_empty() {
            return vec![];
        }
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return vec![],
        };
        let backlinks = vault.find_backlinks(&current_title);
        let query = &self.minibuffer.input;

        if query.is_empty() {
            return backlinks
                .into_iter()
                .map(|(title, path)| Candidate {
                    label: title,
                    detail: None,
                    is_action: false,
                    data: path.to_string_lossy().to_string(),
                })
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, String, std::path::PathBuf)> = backlinks
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
            .map(|(_, title, path)| Candidate {
                label: title,
                detail: None,
                is_action: false,
                data: path.to_string_lossy().to_string(),
            })
            .collect()
    }

    /// List all tags in the vault with counts. Fuzzy-filtered by input.
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

    /// Notes carrying a specific tag. Fuzzy-filtered by input on the title.
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

    /// Notes with neither incoming nor outgoing links — the "lonely" list.
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

    /// Full-text search across all note bodies. Returns matches sorted
    /// by (hit count, title). Each candidate's `detail` shows a single
    /// snippet of the first hit.
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
            let snippet = extract_snippet(&body, &needle, 60);
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

    /// Insert a wikilink completion at the editor cursor.
    /// Replaces the `[[` already typed with `[[title]]`.
    fn insert_wikilink_completion(
        &mut self,
        title: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.active_editor_state();
        editor.update(cx, |state, cx| {
            // The cursor should be right after the `[[` that triggered autocomplete.
            // Replace `[[` with `[[title]]`
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

        // When input is empty, show recent vaults for quick switching
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
                // Mark registered vaults
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

    /// Global commands available in every item context.
    fn global_commands() -> Vec<Command> {
        vec![
            Command {
                id: "write",
                name: "Save",
                description: "Save current note to disk",
                aliases: &["w", "save"],
                binding: Some(":w"),
            },
            Command {
                id: "quit",
                name: "Close Pane",
                description: "Close the current pane, or quit if it is the last pane",
                aliases: &["q", "exit"],
                binding: Some(":q"),
            },
            Command {
                id: "wq",
                name: "Save and Quit",
                description: "Save current note and quit",
                aliases: &["x"],
                binding: Some(":wq"),
            },
            Command {
                id: "vault-switch",
                name: "Switch Vault",
                description: "Switch to a recent vault",
                aliases: &["vault", "vaults", "switch-vault"],
                binding: Some(":vault-switch"),
            },
            Command {
                id: "vault-open",
                name: "Open Vault",
                description: "Browse filesystem to open a vault",
                aliases: &["open-vault"],
                binding: Some(":vault-open"),
            },
            Command {
                id: "notes",
                name: "Find Note",
                description: "Search and open a note in current vault",
                aliases: &["find-note", "find", "note"],
                binding: Some("Ctrl+P"),
            },
            Command {
                id: "edit",
                name: "Edit File",
                description: "Open a file by path",
                aliases: &["e", "open"],
                binding: Some(":e <path>"),
            },
            Command {
                id: "set",
                name: "Set Option",
                description: "Set an editor option",
                aliases: &[],
                binding: Some(":set <option>"),
            },
            Command {
                id: "set-vim",
                name: "Enable Vim Mode",
                description: "Enable vim keybindings",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "set-novim",
                name: "Disable Vim Mode",
                description: "Disable vim keybindings",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "nohlsearch",
                name: "Clear Search Highlighting",
                description: "Remove search result highlighting",
                aliases: &["noh"],
                binding: Some(":noh"),
            },
            Command {
                id: "toggle-vim",
                name: "Toggle Vim Mode",
                description: "Toggle vim mode on/off",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "open-graph",
                name: "Open Graph",
                description: "Open the vault graph in the current pane",
                aliases: &["graph"],
                binding: None,
            },
            Command {
                id: "split",
                name: "Split Pane",
                description: "Split the current pane horizontally",
                aliases: &["sp"],
                binding: Some(":sp"),
            },
            Command {
                id: "vsplit",
                name: "Vertical Split",
                description: "Split the current pane vertically",
                aliases: &["vs"],
                binding: Some(":vs"),
            },
            Command {
                id: "split-open",
                name: "Split Open",
                description: "Open a note or PDF in the right split panel",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "only-window",
                name: "Only Window",
                description: "Make the focused window occupy the workspace",
                aliases: &["only"],
                binding: Some(":only"),
            },
            Command {
                id: "backlinks",
                name: "Backlinks",
                description: "Show notes that link to the current note",
                aliases: &["bl", "references"],
                binding: None,
            },
            Command {
                id: "today",
                name: "Today's Journal",
                description: "Open or create today's journal note",
                aliases: &["daily", "journal"],
                binding: None,
            },
            Command {
                id: "tags",
                name: "Tags",
                description: "List all tags in the vault",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "tag",
                name: "Tag Search",
                description: "Notes with a specific tag",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "orphans",
                name: "Orphan Notes",
                description: "Notes with no incoming or outgoing links",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "search-content",
                name: "Search Content",
                description: "Full-text search across notes",
                aliases: &["search", "grep"],
                binding: Some("Ctrl+Shift+F"),
            },
            Command {
                id: "rename",
                name: "Rename Note",
                description: "Update the current note's title (no file rename — IDs stay stable)",
                aliases: &["rn"],
                binding: Some(":rename <title>"),
            },
            Command {
                id: "insert-links-by-tag",
                name: "Insert Links by Tag",
                description: "Insert wikilinks to all notes with a tag (MOC helper)",
                aliases: &["moc"],
                binding: Some(":moc <tag>"),
            },
            Command {
                id: "vault-forget",
                name: "Forget Vault",
                description: "Remove a vault from the recent-vaults list",
                aliases: &["forget-vault"],
                binding: Some(":vault-forget <path>"),
            },
            Command {
                id: "toggle-backlinks",
                name: "Toggle Backlinks Panel",
                description: "Show or hide the backlinks panel below the editor",
                aliases: &["backlinks-panel"],
                binding: Some("Ctrl+Shift+B"),
            },
            Command {
                id: "attach",
                name: "Attach from Clipboard",
                description: "Save clipboard image to attachments/ and insert a link",
                aliases: &["paste-image"],
                binding: None,
            },
            Command {
                id: "theme",
                name: "Theme",
                description: "Select an application theme",
                aliases: &["themes", "color-scheme"],
                binding: Some(":theme"),
            },
        ]
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

    /// Fuzzy-filter commands for the palette: item commands + global commands.
    fn palette_candidates(&self, query: &str) -> Vec<Candidate> {
        let item_cmds = self.focused_item().commands();
        let global_cmds = &self.global_commands;

        let all_cmds: Vec<&Command> = item_cmds.iter().chain(global_cmds.iter()).collect();

        if query.is_empty() {
            return all_cmds
                .iter()
                .take(MAX_RESULTS)
                .map(|c| command_to_candidate(c))
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
            .map(|(_, c)| command_to_candidate(c))
            .collect()
    }

    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_editor_state();
        if let Err(e) = editor.update(cx, |state, _| state.save_document()) {
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
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            self.open_pdf(path, window, cx);
            return;
        }

        let document = match self.state.open_document(path) {
            Ok(document) => document,
            Err(e) => {
                eprintln!("failed to open note: {}", e);
                return;
            }
        };
        let path = document
            .path()
            .expect("opened document must have a path")
            .to_path_buf();
        self.open_document_buffer(ResourceKey::Markdown(path), document, window, cx);
        cx.notify();
    }

    /// Open a note or PDF in a new right split pane.
    fn open_in_split_by_path(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            let Some((buffer, item)) = self.prepare_pdf(path, window, cx) else {
                return;
            };
            self.show_item_in_new_split(buffer, item, SplitAxis::Horizontal, window, cx);
        } else {
            let key = ResourceKey::Markdown(path.clone());
            let buffer = if let Some(buffer) = self.workspace.buffers.id_for_resource(&key) {
                buffer
            } else {
                let document = match Document::open(path) {
                    Ok(document) => document,
                    Err(e) => {
                        self.minibuffer
                            .set_message(format!("Failed to read: {}", e));
                        return;
                    }
                };
                self.workspace
                    .buffers
                    .open_with(key, || BufferContent::Markdown(EditorBuffer::new(document)))
            };
            let editor_buffer = match self
                .workspace
                .buffers
                .get(buffer)
                .expect("opened markdown buffer must exist")
            {
                BufferContent::Markdown(buffer) => buffer.clone(),
                _ => unreachable!("markdown resource must contain markdown buffer"),
            };
            let item = self.create_editor_item(editor_buffer, window, cx);
            self.show_item_in_new_split(buffer, item, SplitAxis::Horizontal, window, cx);
        }
    }

    fn create_editor_item(
        &mut self,
        buffer: EditorBuffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ActiveItem {
        let editor_state = cx.new(|cx| EditorState::from_buffer(buffer, cx));
        let theme = self.theme;
        let editor_view = cx.new(|cx| EditorView::new(editor_state.clone(), theme, cx));
        let key_sub = cx.subscribe_in(
            &editor_view,
            window,
            |this, _view, ev: &EditorViewEvent, window, cx| match ev {
                EditorViewEvent::Command(cmd_id, count) => {
                    this.execute_command(cmd_id, "", *count, window, cx);
                }
                EditorViewEvent::ItemActions(actions) => {
                    this.process_item_actions(actions.clone(), window, cx);
                }
                EditorViewEvent::VimStateChanged => cx.notify(),
            },
        );
        let state_sub = cx.subscribe_in(
            &editor_state,
            window,
            |this, _entity, ev: &EditorEvent, window, cx| match ev {
                EditorEvent::Changed => {
                    this.minibuffer.message = None;
                    cx.notify();
                }
                EditorEvent::RequestSave => this.save(window, cx),
                EditorEvent::RequestQuit => this.close_window(window, cx),
                EditorEvent::RequestOpen(path) => {
                    this.open_note_by_path(path.into(), window, cx);
                }
                EditorEvent::RequestVaultSwitch => this.activate_vault_switch(window, cx),
                EditorEvent::RequestVaultOpen => this.activate_vault_open(window, cx),
                EditorEvent::RequestNoteSearch => this.activate_note_search(window, cx),
                EditorEvent::RequestCommand => this.activate_command_palette(window, cx),
                EditorEvent::WikilinkClicked(title) => {
                    this.follow_wikilink(title.clone(), window, cx);
                }
                EditorEvent::WikilinkAutocomplete => {
                    this.activate_wikilink_autocomplete(window, cx);
                }
            },
        );
        self._subscriptions.extend([key_sub, state_sub]);
        ActiveItem::Editor {
            state: editor_state,
            view: editor_view,
        }
    }

    fn show_markdown_buffer_in_editor(
        &mut self,
        buffer_id: BufferId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = match self
            .workspace
            .buffers
            .get(buffer_id)
            .expect("opened markdown buffer must exist")
        {
            BufferContent::Markdown(buffer) => buffer.clone(),
            _ => unreachable!("markdown resource must contain markdown buffer"),
        };
        let item = self.create_editor_item(buffer, window, cx);
        let editor_state = item
            .editor_state()
            .expect("new markdown item must expose editor state");
        let editor_view = item
            .editor_view()
            .expect("new markdown item must expose editor view");

        let focused = self.workspace.focused_window;
        self.workspace.switch_window_buffer(focused, buffer_id);
        self.window_items.insert(focused, item);
        self.editor_window = focused;
        self.editor_buffer = buffer_id;
        self.editor_state = editor_state;
        self.editor_view = editor_view;
        self.focus_workspace_window(focused, window, cx);
    }

    fn open_document_buffer(
        &mut self,
        resource: ResourceKey,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = self.workspace.buffers.open_with(resource, || {
            BufferContent::Markdown(EditorBuffer::new(document))
        });
        self.show_markdown_buffer_in_editor(buffer, window, cx);
    }

    /// Create a PDF ActiveItem from a path. Returns None on error (sets minibuffer message).
    fn create_pdf_item(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ActiveItem> {
        let raw_bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.minibuffer
                    .set_message(format!("Failed to read PDF: {}", e));
                cx.notify();
                return None;
            }
        };
        if let Err(e) = mupdf::Document::from_bytes(&raw_bytes, "") {
            self.minibuffer.set_message(format!("Invalid PDF: {:?}", e));
            cx.notify();
            return None;
        }

        let pdf_state = cx.new(|cx| PdfState::new(path, cx).expect("PDF already validated"));
        pdf_state.update(cx, |s, cx| s.extract_text_cache(cx));
        let theme = self.theme;
        let pdf_view = cx.new(|cx| PdfView::new(pdf_state.clone(), theme, cx));
        let pdf_sub = cx.observe(&pdf_state, |_, _, cx| cx.notify());
        self._subscriptions.push(pdf_sub);
        // Route PDF-local keybindings through the existing item-command dispatch.
        let key_sub = cx.subscribe_in(
            &pdf_view,
            window,
            |this, _view, ev: &PdfViewEvent, window, cx| match ev {
                PdfViewEvent::Command(cmd) => {
                    this.execute_command(cmd, "", 1, window, cx);
                }
            },
        );
        self._subscriptions.push(key_sub);

        Some(ActiveItem::Pdf {
            state: pdf_state,
            view: pdf_view,
        })
    }

    fn open_pdf(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((buffer, item)) = self.prepare_pdf(path, window, cx) else {
            return;
        };
        self.show_item_in_focused(buffer, item, window, cx);
    }

    fn prepare_pdf(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(BufferId, ActiveItem)> {
        let key = ResourceKey::Pdf(path.clone());
        let buffer = if let Some(buffer) = self.workspace.buffers.id_for_resource(&key) {
            buffer
        } else {
            self.workspace
                .buffers
                .open_with(key, || BufferContent::Pdf(path.clone()))
        };
        let pdf_path = match self
            .workspace
            .buffers
            .get(buffer)
            .expect("opened PDF buffer must exist")
        {
            BufferContent::Pdf(path) => path.clone(),
            _ => unreachable!("PDF resource must contain PDF buffer"),
        };
        let item = match self.create_pdf_item(&pdf_path, window, cx) {
            Some(item) => item,
            None => return None,
        };
        Some((buffer, item))
    }

    fn show_item_in_focused(
        &mut self,
        buffer: BufferId,
        item: ActiveItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.workspace.focused_window;
        self.workspace.switch_window_buffer(focused, buffer);
        self.window_items.insert(focused, item);
        self.focus_workspace_window(focused, window, cx);
    }

    fn show_item_in_new_split(
        &mut self,
        buffer: BufferId,
        item: ActiveItem,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let split = self
            .workspace
            .split_focused(axis, buffer)
            .expect("focused window must be splittable");
        self.window_items.insert(split, item);
        self.focus_workspace_window(split, window, cx);
    }

    fn open_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = ResourceKey::Graph(
            self.state
                .vault
                .as_ref()
                .map(|vault| vault.path.clone())
                .unwrap_or_default(),
        );
        let buffer = if let Some(buffer) = self.workspace.buffers.id_for_resource(&key) {
            buffer
        } else {
            let path = match &key {
                ResourceKey::Graph(path) => path.clone(),
                _ => unreachable!(),
            };
            self.workspace
                .buffers
                .open_with(key, || BufferContent::Graph(path))
        };
        let graph_path = match self
            .workspace
            .buffers
            .get(buffer)
            .expect("opened graph buffer must exist")
        {
            BufferContent::Graph(path) => path.clone(),
            _ => unreachable!("graph resource must contain graph buffer"),
        };
        let graph_item = self.create_graph_item(&graph_path, window, cx);
        self.show_item_in_focused(buffer, graph_item, window, cx);
    }

    fn create_graph_item(
        &mut self,
        vault_path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ActiveItem {
        // Build graph from vault
        let graph_state = cx.new(|cx| {
            let mut gs = GraphState::new(cx);
            if let Some(vault) = &self.state.vault {
                if vault.path == vault_path {
                    gs.build_from_vault(vault);
                }
            }
            // Set local root to current note if one is open
            if let Some(current) = self.current_document_path(cx) {
                gs.set_local_root_by_path(&current);
            }
            gs
        });
        let theme = self.theme;
        let graph_view = cx.new(|cx| GraphView::new(graph_state.clone(), theme, cx));

        // Subscribe to graph events (node clicks)
        let graph_sub = cx.subscribe_in(
            &graph_state,
            window,
            |this, _entity, ev: &GraphEvent, window, cx| match ev {
                GraphEvent::OpenNote(path) => {
                    this.open_note_by_path(path.clone(), window, cx);
                }
            },
        );
        self._subscriptions.push(graph_sub);

        // Route graph-local keybindings through the existing item-command dispatch.
        let key_sub = cx.subscribe_in(
            &graph_view,
            window,
            |this, _view, ev: &GraphViewEvent, window, cx| match ev {
                GraphViewEvent::Command(cmd) => {
                    this.execute_command(cmd, "", 1, window, cx);
                }
            },
        );
        self._subscriptions.push(key_sub);

        // Observe so we re-render on physics ticks
        let obs = cx.observe(&graph_state, |_, _, cx| cx.notify());
        self._subscriptions.push(obs);

        ActiveItem::Graph {
            state: graph_state,
            view: graph_view,
        }
    }

    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let closed = self.workspace.focused_window;
        if self.workspace.close_focused() {
            self.window_items.remove(closed);
            let focused = self.workspace.focused_window;
            self.item_for_window(focused).focus(window, cx);
            cx.notify();
        } else {
            cx.quit();
        }
    }

    fn split_focused_window(
        &mut self,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = self.workspace.focused_buffer();
        let Some(item) = self.create_item_for_buffer(buffer, window, cx) else {
            return;
        };
        self.show_item_in_new_split(buffer, item, axis, window, cx);
    }

    fn create_item_for_buffer(
        &mut self,
        buffer: BufferId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ActiveItem> {
        let content = self
            .workspace
            .buffers
            .get(buffer)
            .expect("live window buffer must exist")
            .clone();
        match content {
            BufferContent::Markdown(buffer) => Some(self.create_editor_item(buffer, window, cx)),
            BufferContent::Pdf(path) => self.create_pdf_item(&path, window, cx),
            BufferContent::Graph(path) => Some(self.create_graph_item(&path, window, cx)),
        }
    }

    fn create_note_by_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.state.create_note(title) {
            Ok(document) => {
                let path = document
                    .path()
                    .expect("created document must have a path")
                    .to_path_buf();
                self.open_document_buffer(ResourceKey::Markdown(path), document, window, cx);
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
        let (resource, document) = match self.state.open_vault(path) {
            Ok(Some(document)) => {
                let path = document
                    .path()
                    .expect("vault document must have a path")
                    .to_path_buf();
                (ResourceKey::Markdown(path), document)
            }
            Ok(None) => {
                let vault_path = self
                    .state
                    .vault
                    .as_ref()
                    .map(|vault| vault.path.clone())
                    .unwrap_or_default();
                (
                    ResourceKey::Scratch(vault_path),
                    Document::scratch(String::new()),
                )
            }
            Err(e) => {
                eprintln!("failed to open vault: {}", e);
                return;
            }
        };
        self.open_document_buffer(resource, document, window, cx);
        self.start_vault_watcher();
        cx.notify();
    }

    /// Start (or restart) the filesystem watcher for the current vault.
    /// The polling loop in `Memex::new` drains events from the stored
    /// watcher and triggers `Vault::refresh` — this just reseats the
    /// source.
    fn start_vault_watcher(&mut self) {
        // Drop the previous watcher first so its channel closes.
        self.vault_watcher = None;
        let Some(vault) = self.state.vault.as_ref() else {
            return;
        };
        match crate::vault::VaultWatcher::start(&vault.path) {
            Ok(w) => self.vault_watcher = Some(w),
            Err(e) => eprintln!("vault watcher failed to start: {}", e),
        }
    }
    fn search_notes(&self, query: &str) -> Vec<(String, std::path::PathBuf)> {
        let vault = match &self.state.vault {
            Some(v) => v,
            None => return Vec::new(),
        };

        let titles = vault.openable_titles();

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
            let t = self.current_document_title(cx);
            if t.is_empty() { "Memex".to_string() } else { t }
        };
        let dirty = self.current_document_dirty(cx);
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
            // Left: spacer for symmetry — part of the drag area
            .child(
                div()
                    .w(px(72.))
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    }),
            )
            // Center: title — part of the drag area
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .justify_center()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(self.theme.text))
                            .child(title_text),
                    ),
            )
            // Right: window controls — NOT inside the drag area, so Close hitbox wins
            .child(h_flex().gap(px(0.)).child(self.title_bar_close_button(cx)))
    }

    fn title_bar_close_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .hover(|s| s.text_color(rgba(0x00000010)).bg(rgba(0xFF000040)))
            // Register close hitbox for Windows hit testing
            .window_control_area(WindowControlArea::Close)
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.quit();
            }))
            .child(Icon::new(IconName::WindowClose))
    }

    /// Backlinks panel — always-visible strip below the editor listing
    /// every note that links to the current one. Toggled with
    /// Ctrl+Shift+B. Empty state shows a subtle nudge.
    fn render_backlinks_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current_title = self.current_document_title(cx);
        let backlinks = self
            .state
            .vault
            .as_ref()
            .filter(|_| !current_title.is_empty())
            .map(|v| v.find_backlinks(&current_title))
            .unwrap_or_default();

        let mut header = h_flex()
            .w_full()
            .px(px(8.))
            .py(px(3.))
            .bg(rgb(self.theme.surface))
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(self.theme.text_strong))
                    .child("Backlinks"),
            );
        header = header.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(self.theme.text_muted))
                .child(format!("{}", backlinks.len())),
        );

        let mut list = v_flex()
            .w_full()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(self.theme.background));

        if backlinks.is_empty() {
            list = list.child(
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(self.theme.text_muted))
                    .child(if current_title.is_empty() {
                        "No note open."
                    } else {
                        "No backlinks yet. Link to this note from another note with [[…]]."
                    }),
            );
        } else {
            for (title, path) in backlinks {
                let path_for_click = path.clone();
                list = list.child(
                    div()
                        .id(ElementId::Name(
                            format!("bl-{}", path.to_string_lossy()).into(),
                        ))
                        .w_full()
                        .px(px(10.))
                        .py(px(3.))
                        .text_size(px(12.))
                        .text_color(rgb(self.theme.accent))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(0x00000010)))
                        .child(title)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                                this.open_note_by_path(path_for_click.clone(), window, cx);
                            }),
                        ),
                );
            }
        }

        v_flex()
            .w_full()
            .h(px(160.))
            .border_t_1()
            .border_color(rgb(self.theme.border))
            .child(header)
            .child(list)
    }

    fn render_mode_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_view = self.active_editor_view();
        let ev = editor_view.read(cx);
        let vim_enabled = ev.keymap.vim_enabled;
        let vim_state = ev.keymap.active_vim_state().map(|s| s.to_string());

        let vault_name = self.state.vault_name();
        let note_title = self.current_document_title(cx);
        let dirty = self.current_document_dirty(cx);
        let dirty_indicator = if dirty { " ●" } else { "" };

        // Position info depends on the focused window.
        let focused_item = self.focused_item();
        let position_text = focused_item.position_text(600.0, cx);

        // Mode badge (left) — show focused item's badge
        let show_non_editor = focused_item.is_pdf() || focused_item.is_graph();
        let mode_badge = if show_non_editor {
            let (label, color) = focused_item.mode_badge();
            div().px(px(6.)).py(px(1.)).bg(rgb(color)).child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(self.theme.background))
                    .child(label),
            )
        } else if vim_enabled {
            let (label, bg) = match vim_state.as_deref() {
                Some("NORMAL") => ("NORMAL", rgb(self.theme.accent)),
                Some("INSERT") => ("INSERT", rgb(self.theme.success)),
                Some("VISUAL") => ("VISUAL", rgb(self.theme.violet)),
                Some("V-LINE") => ("V-LINE", rgb(self.theme.violet)),
                _ => ("NOR", rgb(self.theme.accent)),
            };
            div().px(px(6.)).py(px(1.)).bg(bg).child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(self.theme.background))
                    .child(label),
            )
        } else {
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(rgb(self.theme.success))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(self.theme.background))
                        .child("EDT"),
                )
        };

        h_flex()
            .w_full()
            .h(px(24.))
            .bg(rgb(self.theme.surface))
            .items_center()
            .gap(px(0.))
            .child(mode_badge)
            // Vault + file
            .child(
                div().px(px(8.)).child(
                    div()
                        .text_size(px(14.))
                        .text_color(rgb(self.theme.text_strong))
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
                div().px(px(8.)).child(
                    div()
                        .text_size(px(14.))
                        .text_color(rgb(self.theme.text_muted))
                        .child(position_text),
                ),
            )
    }

    /// Render the minibuffer area — unified, single rendering path.
    /// Always visible like emacs: shows echo area messages when idle,
    /// prompt + input + vertico candidates when active.
    fn render_minibuffer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = v_flex().w_full().bg(rgb(self.theme.background));

        if !self.minibuffer.active {
            // Idle — echo area: show message or status from editor
            let msg = self
                .minibuffer
                .message
                .clone()
                .or_else(|| self.active_editor_state().read(cx).status_message.clone())
                .unwrap_or_default();
            return base.child(
                h_flex().w_full().h(px(22.)).px(px(8.)).py(px(3.)).child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(self.theme.text_muted))
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
                rgb(self.theme.selection)
            } else {
                rgb(self.theme.background)
            };

            let text_color = if candidate.is_action {
                rgb(self.theme.success)
            } else if is_selected {
                rgb(self.theme.text_strong)
            } else {
                rgb(self.theme.text)
            };

            let label_element = if matches!(self.minibuffer.delegate_kind, DelegateKind::Item(ref id) if self.focused_item().highlight_input(id))
                && !self.minibuffer.input.is_empty()
            {
                // Highlight the search term within the candidate label
                render_highlighted_label(
                    &candidate.label,
                    &self.minibuffer.input,
                    text_color,
                    self.theme.warning,
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
                        .text_color(rgb(self.theme.text_muted))
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
            .border_color(rgb(self.theme.border))
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
                            .text_color(rgb(self.theme.accent))
                            .child(self.minibuffer.prompt.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(self.theme.text_strong))
                            .child(format!("{}{}{}", before_cursor, cursor_char, after_cursor)),
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
            .bg(rgb(self.theme.background))
            .font_family("FiraCode Nerd Font")
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
                this.focus_workspace_window(this.editor_window, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusRightPane, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                if let Some(secondary) = this.secondary_window() {
                    this.focus_workspace_window(secondary, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SearchContent, window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.activate_content_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleBacklinks, _window, cx| {
                if this.minibuffer.active {
                    return;
                }
                this.show_backlinks = !this.show_backlinks;
                cx.notify();
            }))
            // Custom title bar with drag + window controls
            .child(self.render_title_bar(cx))
            .child(self.render_window_layout(&self.workspace.layout, cx))
            // Optional backlinks panel below the content (Ctrl+Shift+B)
            .children(if self.show_backlinks {
                Some(self.render_backlinks_panel(cx).into_any_element())
            } else {
                None
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

/// Render a label with the search term highlighted in a distinct color.
fn render_highlighted_label(
    label: &str,
    query: &str,
    base_color: impl Into<Hsla> + Copy,
    highlight: u32,
) -> Div {
    let highlight_color = rgb(highlight);
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
                    div()
                        .text_color(base_hsla)
                        .child(label[pos..abs_start].to_string()),
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
            container =
                container.child(div().text_color(base_hsla).child(label[pos..].to_string()));
            break;
        }
    }

    container
}

/// Snap byte index to a valid char boundary.
fn snap_to_char(s: &str, idx: usize, ceil: bool) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    if s.is_char_boundary(idx) {
        return idx;
    }
    if ceil {
        let mut i = idx;
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        i
    } else {
        let mut i = idx;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Extract a single-line snippet around the first match of `needle` in
/// `body`, truncated to roughly `radius` chars on each side. Case-
/// insensitive match. Returns `"…"` if nothing matches.
fn extract_snippet(body: &str, needle: &str, radius: usize) -> String {
    let lower = body.to_lowercase();
    let Some(pos) = lower.find(needle) else {
        return "…".to_string();
    };
    // Align to char boundaries in the original body.
    let start = body[..pos]
        .char_indices()
        .rev()
        .take(radius)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end_target = pos + needle.len() + radius;
    let mut end = end_target.min(body.len());
    while end < body.len() && !body.is_char_boundary(end) {
        end += 1;
    }
    let slice = &body[start..end];
    let slice = slice.lines().next().unwrap_or(slice);
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < body.len() { "…" } else { "" };
    format!("{}{}{}", prefix, slice.trim(), suffix)
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
