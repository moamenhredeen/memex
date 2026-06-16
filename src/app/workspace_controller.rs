use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;

use super::resource::{
    BufferContent, PdfLinkTarget, ResourceKey, SecondaryContent, is_diagram_path, is_pdf_path,
};
use super::{MAX_RESULTS, Memex};
use crate::backlinks::{BacklinksState, BacklinksView, BacklinksViewEvent};
use crate::diagram::{DiagramState, DiagramView, DiagramViewEvent, ExcalidrawFile};
use crate::document::Document;
use crate::editor::{EditorBuffer, EditorEvent, EditorState, EditorView, EditorViewEvent};
use crate::graph::{GraphEvent, GraphState, GraphView, GraphViewEvent};
use crate::pane::ActiveItem;
use crate::pdf::{PdfState, PdfView, PdfViewEvent};
use crate::workspace::{BufferId, WorkspaceFocus};

impl Memex {
    pub(super) fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_editor_state();
        if let Err(e) = editor.update(cx, |state, _| state.save_document()) {
            eprintln!("save error: {}", e);
        }
        cx.notify();
    }

    pub(super) fn open_note_by_path(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_pdf_path(&path) {
            self.open_pdf(path, window, cx);
            return;
        }
        if is_diagram_path(&path) {
            self.open_diagram(path, window, cx);
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

    fn create_editor_item(
        &mut self,
        buffer: EditorBuffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ActiveItem {
        let editor_state = cx.new(|cx| EditorState::from_buffer(buffer, cx));
        if let Some(vault) = self.state.vault.as_ref() {
            let titles = vault.index.wikilink_titles();
            editor_state.update(cx, |editor, cx| editor.set_wikilink_titles(titles, cx));
        }
        let theme = self.theme;
        let editor_width = self.state.config.editor_width;
        let editor_view =
            cx.new(|cx| EditorView::new(editor_state.clone(), theme, editor_width, cx));
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

        self.workspace.show_editor(buffer_id);
        self.editor_buffer = buffer_id;
        self.editor_state = editor_state;
        self.editor_view = editor_view;
        self.editor_item = item;
        self.focus_editor(window, cx);
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

    pub(super) fn open_pdf(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((buffer, item)) = self.prepare_pdf(path, window, cx) else {
            return;
        };
        self.show_secondary_item(buffer, item, window, cx);
    }

    pub(super) fn open_pdf_target(
        &mut self,
        path: std::path::PathBuf,
        target: PdfLinkTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(SecondaryContent::Item { item, .. }) = &self.secondary {
            if let Some(state) = item.pdf_state()
                && state.read(cx).path == path
            {
                let found = state.update(cx, |state, _| match &target {
                    PdfLinkTarget::Page(page) => {
                        if *page <= state.page_count {
                            state.goto_page_number(*page);
                            true
                        } else {
                            false
                        }
                    }
                    PdfLinkTarget::Annotation(id) => state.goto_annotation(id),
                });
                if found {
                    self.focus_secondary(window, cx);
                    return;
                }
            }
        }
        let Some((buffer, item)) = self.prepare_pdf(path, window, cx) else {
            return;
        };
        let Some(state) = item.pdf_state() else {
            return;
        };
        let found = state.update(cx, |state, _| match &target {
            PdfLinkTarget::Page(page) => {
                if *page <= state.page_count {
                    state.goto_page_number(*page);
                    true
                } else {
                    false
                }
            }
            PdfLinkTarget::Annotation(id) => state.goto_annotation(id),
        });
        if !found {
            let message = match target {
                PdfLinkTarget::Page(page) => format!("PDF page not found: {}", page),
                PdfLinkTarget::Annotation(id) => format!("PDF annotation not found: {}", id),
            };
            self.minibuffer.set_message(message);
            cx.notify();
            return;
        }
        self.show_secondary_item(buffer, item, window, cx);
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
        let item = self.create_pdf_item(&pdf_path, window, cx)?;
        Some((buffer, item))
    }

    fn show_secondary_item(
        &mut self,
        buffer: BufferId,
        item: ActiveItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.show_secondary(Some(buffer));
        self.secondary = Some(SecondaryContent::Item { item });
        self.focus_secondary(window, cx);
    }

    pub(super) fn open_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.show_secondary_item(buffer, graph_item, window, cx);
    }

    fn create_graph_item(
        &mut self,
        vault_path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ActiveItem {
        let graph_state = cx.new(|cx| {
            let mut graph_state = GraphState::new(cx);
            if let Some(vault) = &self.state.vault
                && vault.path == vault_path
            {
                graph_state.build_from_vault(vault);
            }
            if let Some(current) = self.current_document_path(cx) {
                graph_state.set_local_root_by_path(&current);
            }
            graph_state
        });
        let theme = self.theme;
        let graph_view = cx.new(|cx| GraphView::new(graph_state.clone(), theme, cx));

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

        let obs = cx.observe(&graph_state, |_, _, cx| cx.notify());
        self._subscriptions.push(obs);

        ActiveItem::Graph {
            state: graph_state,
            view: graph_view,
        }
    }

    // ─── Diagram ────────────────────────────────────────────────────────

    fn create_diagram_item(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ActiveItem> {
        let file = match ExcalidrawFile::load(path) {
            Ok(file) => file,
            Err(e) => {
                self.minibuffer
                    .set_message(format!("Failed to open diagram: {}", e));
                cx.notify();
                return None;
            }
        };
        let diagram_state = cx.new(|cx| DiagramState::new(path, file, cx));
        // Frame the content on open (assumed pane size; user can pan/zoom).
        diagram_state.update(cx, |s, _| s.fit_to_content(800.0, 600.0));
        let theme = self.theme;
        let diagram_view = cx.new(|cx| DiagramView::new(diagram_state.clone(), theme, cx));
        let obs = cx.observe(&diagram_state, |_, _, cx| cx.notify());
        self._subscriptions.push(obs);
        let key_sub = cx.subscribe_in(
            &diagram_view,
            window,
            |this, _view, ev: &DiagramViewEvent, window, cx| match ev {
                DiagramViewEvent::Command(cmd) => {
                    this.execute_command(cmd, "", 1, window, cx);
                }
            },
        );
        self._subscriptions.push(key_sub);

        Some(ActiveItem::Diagram {
            state: diagram_state,
            view: diagram_view,
        })
    }

    fn prepare_diagram(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(BufferId, ActiveItem)> {
        let key = ResourceKey::Diagram(path.clone());
        let buffer = if let Some(buffer) = self.workspace.buffers.id_for_resource(&key) {
            buffer
        } else {
            self.workspace
                .buffers
                .open_with(key, || BufferContent::Diagram(path.clone()))
        };
        let diagram_path = match self
            .workspace
            .buffers
            .get(buffer)
            .expect("opened diagram buffer must exist")
        {
            BufferContent::Diagram(path) => path.clone(),
            _ => unreachable!("diagram resource must contain diagram buffer"),
        };
        let item = self.create_diagram_item(&diagram_path, window, cx)?;
        Some((buffer, item))
    }

    pub(super) fn open_diagram(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reuse an already-open diagram in the secondary slot.
        if let Some(SecondaryContent::Item { item, .. }) = &self.secondary
            && let Some(state) = item.diagram_state()
            && state.read(cx).path == path
        {
            self.focus_secondary(window, cx);
            return;
        }
        let Some((buffer, item)) = self.prepare_diagram(path, window, cx) else {
            return;
        };
        self.show_secondary_item(buffer, item, window, cx);
    }

    /// Create a new empty diagram in the vault's `diagrams/` folder, insert a
    /// `[[name.excalidraw]]` link at the editor cursor, and open it.
    pub(super) fn new_diagram(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.state.vault.as_ref() else {
            self.minibuffer.set_message("No vault open");
            cx.notify();
            return;
        };
        let layout = vault.layout();
        if let Err(e) = std::fs::create_dir_all(&layout.diagrams) {
            self.minibuffer
                .set_message(format!("Failed to create diagrams folder: {}", e));
            cx.notify();
            return;
        }

        // Pick a unique `diagram-N.excalidraw` name.
        let mut file_name = String::new();
        for ix in 1.. {
            let candidate = format!("diagram-{}.excalidraw", ix);
            if !layout.diagram_path(&candidate).exists() {
                file_name = candidate;
                break;
            }
        }
        let path = layout.diagram_path(&file_name);

        if let Err(e) = ExcalidrawFile::empty().save(&path) {
            self.minibuffer
                .set_message(format!("Failed to create diagram: {}", e));
            cx.notify();
            return;
        }

        // Insert the link into the current note at the cursor.
        let snippet = format!("[[{}]]", file_name);
        self.active_editor_state().update(cx, |state, cx| {
            state.edit_text(&snippet, cx);
        });

        // Refresh the vault so the new diagram is visible to scans/links.
        if let Some(vault) = self.state.vault.as_mut() {
            let _ = vault.refresh();
        }

        self.open_diagram(path, window, cx);
        self.minibuffer
            .set_message(format!("Created {}", file_name));
        cx.notify();
    }

    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.focus == WorkspaceFocus::Secondary && self.secondary.is_some() {
            self.close_secondary(window, cx);
        } else {
            cx.quit();
        }
    }

    pub(super) fn close_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.secondary = None;
        self.workspace.close_secondary();
        self.focus_editor(window, cx);
    }

    pub(super) fn show_backlinks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = ResourceKey::Backlinks(
            self.state
                .vault
                .as_ref()
                .map(|vault| vault.path.clone())
                .unwrap_or_default(),
        );
        let buffer = if let Some(buffer) = self.workspace.buffers.id_for_resource(&key) {
            buffer
        } else {
            self.workspace
                .buffers
                .open_with(key, || BufferContent::Backlinks)
        };
        let item = self.create_backlinks_item(window, cx);
        self.show_secondary_item(buffer, item, window, cx);
    }

    fn create_backlinks_item(&mut self, window: &mut Window, cx: &mut Context<Self>) -> ActiveItem {
        let current_title = self.current_document_title(cx);
        let backlinks = self
            .state
            .vault
            .as_ref()
            .filter(|_| !current_title.is_empty())
            .map(|v| v.find_backlinks(&current_title))
            .unwrap_or_default();
        let backlinks_state = cx.new(|cx| BacklinksState::new(current_title, backlinks, cx));
        let theme = self.theme;
        let backlinks_view = cx.new(|cx| BacklinksView::new(backlinks_state.clone(), theme, cx));
        let view_sub = cx.subscribe_in(
            &backlinks_view,
            window,
            |this, _view, ev: &BacklinksViewEvent, window, cx| match ev {
                BacklinksViewEvent::OpenPath(path) => {
                    this.open_note_by_path(path.clone(), window, cx);
                }
            },
        );
        self._subscriptions.push(view_sub);
        ActiveItem::Backlinks {
            state: backlinks_state,
            view: backlinks_view,
        }
    }

    pub(super) fn create_note_by_title(
        &mut self,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn open_vault_by_path(
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

    pub(super) fn start_vault_watcher(&mut self) {
        self.vault_watcher = None;
        let Some(vault) = self.state.vault.as_ref() else {
            return;
        };
        match crate::vault::VaultWatcher::start(&vault.path) {
            Ok(w) => self.vault_watcher = Some(w),
            Err(e) => eprintln!("vault watcher failed to start: {}", e),
        }
    }

    pub(super) fn search_notes(&self, query: &str) -> Vec<(String, std::path::PathBuf)> {
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
}
