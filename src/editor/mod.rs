mod blink;
mod buffer;
mod code_edit;
pub mod commands;
mod display_map;
mod element;
mod input;
mod movement;
pub mod outline;
mod table;
pub mod undo;
mod view;

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

use crate::command::Command;
use crate::document::Document;
use crate::minibuffer::Candidate;
use crate::pane::{CommandOutcome, ItemAction};
use gpui::*;

pub use blink::BlinkCursor;
pub use buffer::EditorBuffer;
pub use view::{EditorView, EditorViewEvent};

actions!(editor, [TabAction, ShiftTabAction]);

pub const DIAGRAM_EMBED_HEIGHT_PX: f32 = 320.0;

pub struct EditorState {
    pub buffer: EditorBuffer,
    pub cursor: usize,
    pub selected_range: Range<usize>,
    pub selection_reversed: bool,
    pub marked_range: Option<Range<usize>>,
    pub focus_handle: FocusHandle,
    pub blink_cursor: Entity<BlinkCursor>,
    pub scroll_offset: Pixels,
    pub last_line_layouts: Vec<LinePaintInfo>,
    pub last_bounds: Option<Bounds<Pixels>>,
    pub grammar: crate::keymap::VimGrammar,
    pub display_map: display_map::DisplayMap,
    pub outline: outline::OutlineState,
    /// Status message shown briefly after command execution
    pub status_message: Option<String>,
    /// Whether vim is enabled (mirrored from KeymapSystem for input handler)
    pub vim_enabled: bool,
    /// Whether insert mode is active (mirrored from KeymapSystem for input handler)
    pub insert_mode: bool,
    /// Scrollbar drag state (survives across frames).
    pub drag_state: Option<crate::ui::DragState>,
    /// Viewport height from last frame, used by follow-cursor scrolling.
    pub viewport_height: Pixels,
    pub wrap_width: Pixels,
    /// Set by any cursor-moving operation; cleared by `EditorElement` after
    /// it scrolls the cursor into view.
    pub needs_scroll_to_cursor: bool,
    pub wikilink_titles: HashMap<String, String>,
    pub diagram_dir: Option<PathBuf>,
    vim_edit_group_active: bool,
    visual_line_anchor: Option<usize>,
    _blink_sub: Subscription,
}

#[derive(Clone)]
pub struct LinePaintInfo {
    pub content_offset: usize,
    pub shaped_line: WrappedLine,
    pub origin_x: Pixels,
    pub source_len: usize,
    pub source_to_display: Vec<usize>,
    pub display_to_source: Vec<usize>,
    pub y: Pixels,
    pub row_height: Pixels,
    pub line_height: Pixels,
}

#[derive(Clone, Debug)]
pub struct DiagramEmbed {
    pub target: String,
    pub path: PathBuf,
}

impl LinePaintInfo {
    pub fn display_offset(&self, source_offset: usize) -> usize {
        self.source_to_display[source_offset.min(self.source_len)]
    }

    pub fn source_offset(&self, display_offset: usize) -> usize {
        self.display_to_source[display_offset.min(self.shaped_line.len())]
    }

    pub fn display_position(&self, source_offset: usize) -> Point<Pixels> {
        self.shaped_line
            .position_for_index(self.display_offset(source_offset), self.row_height)
            .unwrap_or_default()
    }
}

fn diagram_link_target(line_text: &str) -> Option<String> {
    let trimmed = line_text.trim();
    let inner = trimmed.strip_prefix("[[")?.strip_suffix("]]")?;
    let target = inner.split('|').next().unwrap_or(inner).trim();
    if target.to_ascii_lowercase().ends_with(".diagram") {
        Some(target.to_string())
    } else {
        None
    }
}

impl EditorState {
    pub fn set_yank_register(&mut self, text: String) {
        self.grammar.register_content = text;
    }

    #[allow(dead_code)]
    pub fn new(content: String, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::from_document(Document::scratch(content), cx)
    }

    pub fn from_document(document: Document, cx: &mut Context<Self>) -> Self {
        Self::from_buffer(EditorBuffer::new(document), cx)
    }

    /// Create independent window state for an existing shared buffer.
    pub fn from_buffer(buffer: EditorBuffer, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let blink_cursor = cx.new(|_cx| BlinkCursor::new());
        let _blink_sub = cx.observe(&blink_cursor, |_, _, cx| cx.notify());

        let mut display = display_map::DisplayMap::new(px(24.));
        let content = buffer.content();
        display.update(&content);

        Self {
            cursor: 0,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            buffer,
            focus_handle,
            blink_cursor,
            scroll_offset: px(0.),
            last_line_layouts: Vec::new(),
            last_bounds: None,
            grammar: crate::keymap::VimGrammar::new(),
            display_map: display,
            outline: outline::OutlineState::new(),
            status_message: None,
            vim_enabled: true,
            insert_mode: false,
            drag_state: None,
            viewport_height: px(0.),
            wrap_width: px(0.),
            needs_scroll_to_cursor: false,
            wikilink_titles: HashMap::new(),
            diagram_dir: None,
            vim_edit_group_active: false,
            visual_line_anchor: None,
            _blink_sub,
        }
    }

    pub fn set_wikilink_titles(&mut self, titles: HashMap<String, String>, cx: &mut Context<Self>) {
        self.wikilink_titles = titles;
        cx.notify();
    }

    pub fn set_diagram_dir(&mut self, diagram_dir: Option<PathBuf>, cx: &mut Context<Self>) {
        self.diagram_dir = diagram_dir;
        cx.notify();
    }

    pub fn diagram_embed_for_line(&self, line_text: &str) -> Option<DiagramEmbed> {
        let target = diagram_link_target(line_text)?;
        let path = self.diagram_dir.as_ref()?.join(&target);
        Some(DiagramEmbed { target, path })
    }

    pub fn offset_is_diagram_embed_line(&self, offset: usize) -> bool {
        let content = self.content();
        let line_idx = self.display_map.line_for_offset(offset);
        self.line_is_diagram_embed(line_idx, &content)
    }

    pub(crate) fn line_is_diagram_embed(&self, line_idx: usize, content: &str) -> bool {
        self.line_text_range(line_idx, content)
            .and_then(|range| content.get(range))
            .and_then(|line| self.diagram_embed_for_line(line))
            .is_some()
    }

    pub(crate) fn line_text_range(
        &self,
        line_idx: usize,
        content: &str,
    ) -> Option<std::ops::Range<usize>> {
        let line_offset = self.display_map.line_offset(line_idx);
        if line_offset > content.len() || !content.is_char_boundary(line_offset) {
            return None;
        }
        let line_end = content[line_offset..]
            .find('\n')
            .map(|i| line_offset + i)
            .unwrap_or(content.len());
        Some(line_offset..line_end)
    }

    pub(crate) fn prepare_display_layout(
        &mut self,
        wrap_width: Pixels,
        viewport_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let content = self.content();
        self.display_map.update(&content);
        if self.wrap_width != wrap_width {
            self.wrap_width = wrap_width;
            self.display_map.reset_line_heights();
        }

        let kinds = self.display_map.line_kinds();
        let headings = outline::extract_headings(&kinds);
        let line_count = self.display_map.line_count();
        let hidden = self.outline.compute_hidden_lines(&headings, line_count);
        self.display_map.update_visibility(&hidden);

        self.viewport_height = viewport_height;
        self.sync_diagram_embed_line_heights(&content, cx);

        if self.needs_scroll_to_cursor {
            self.scroll_cursor_into_view();
            self.needs_scroll_to_cursor = false;
        }
    }

    fn sync_diagram_embed_line_heights(&mut self, content: &str, cx: &mut Context<Self>) {
        let embed_height = px(DIAGRAM_EMBED_HEIGHT_PX);
        let mut height_updates = Vec::new();
        for line_idx in 0..self.display_map.line_count() {
            let Some(range) = self.line_text_range(line_idx, content) else {
                continue;
            };
            let is_embed = content
                .get(range.clone())
                .and_then(|line| self.diagram_embed_for_line(line))
                .is_some();
            let cursor_on_line = self.cursor >= range.start && self.cursor <= range.end;
            let current = self.display_map.line_height(line_idx);
            if is_embed && !cursor_on_line {
                if current != embed_height {
                    height_updates.push((line_idx, embed_height));
                }
            } else if current == embed_height {
                height_updates.push((
                    line_idx,
                    self.display_map.line_info(line_idx).kind.line_height(),
                ));
            }
        }
        if self.display_map.update_line_heights(&height_updates) {
            cx.notify();
        }
    }

    /// Snapshot the buffer as a String (allocates). Use for read-heavy operations
    /// that need string slicing. Mutations should use the rope API directly.
    pub fn content(&self) -> String {
        self.buffer.content()
    }

    pub fn content_len(&self) -> usize {
        self.buffer.len_bytes()
    }

    pub fn document_path(&self) -> Option<std::path::PathBuf> {
        self.buffer.document_path()
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    pub fn save_document(&mut self) -> Result<(), std::io::Error> {
        self.buffer.save()
    }

    /// Check if a byte offset falls within a [[wikilink]] span.
    /// Returns the wikilink target title if found.
    pub fn wikilink_at_offset(&self, offset: usize) -> Option<String> {
        let content = self.content();
        let line_idx = self.display_map.line_for_offset(offset);
        let line_offset = self.display_map.line_offset(line_idx);
        let info = self.display_map.line_info(line_idx);
        let pos_in_line = offset - line_offset;

        for span in &info.spans {
            if span.kind == crate::markdown::StyleKind::Wikilink
                && pos_in_line >= span.range.start
                && pos_in_line < span.range.end
            {
                let line_end = content[line_offset..]
                    .find('\n')
                    .map(|i| line_offset + i)
                    .unwrap_or(content.len());
                let line_text = &content[line_offset..line_end];
                if let Some(raw) = line_text.get(span.range.clone()) {
                    if let Some(inner) = raw.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
                        let target = inner.split('|').next().unwrap_or(inner).trim();
                        if !target.is_empty() {
                            return Some(target.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if a byte offset falls on a checkbox (`- [ ] ` or `- [x] `).
    /// Returns the byte range of the checkbox marker (`[ ]` or `[x]`) if found.
    pub fn checkbox_at_offset(&self, offset: usize) -> Option<std::ops::Range<usize>> {
        let content = self.content();
        let line_idx = self.display_map.line_for_offset(offset);
        let line_offset = self.display_map.line_offset(line_idx);
        let line_end = content[line_offset..]
            .find('\n')
            .map(|i| line_offset + i)
            .unwrap_or(content.len());
        let line_text = &content[line_offset..line_end];
        let trimmed_start = line_text.len() - line_text.trim_start().len();

        // Check for `- [ ] ` or `- [x] ` or `- [X] ` at the start of the line
        let trimmed = line_text.trim_start();
        if trimmed.starts_with("- [ ] ")
            || trimmed.starts_with("- [x] ")
            || trimmed.starts_with("- [X] ")
        {
            // The checkbox bracket range within the line: `[`, ` ` or `x`, `]`
            let bracket_start = line_offset + trimmed_start + 2; // skip "- "
            let bracket_end = bracket_start + 3; // "[ ]" or "[x]"
            // Allow clicking anywhere on the checkbox prefix "- [ ] "
            if offset >= line_offset + trimmed_start && offset < line_offset + trimmed_start + 6 {
                return Some(bracket_start..bracket_end);
            }
        }
        None
    }

    /// Toggle a checkbox at the given byte range between `[ ]` and `[x]`.
    pub fn toggle_checkbox(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let content = self.content();
        if let Some(marker) = content.get(range.clone()) {
            let new_marker = if marker == "[ ]" { "[x]" } else { "[ ]" };
            let cursor_before = self.cursor;
            let selection_before = self.selected_range.clone();

            self.rope_replace(range.clone(), new_marker);

            self.buffer.record_edit(
                crate::editor::undo::EditOp {
                    range: range.clone(),
                    old_text: marker.to_string(),
                    new_text: new_marker.to_string(),
                    cursor_before,
                    cursor_after: cursor_before,
                },
                selection_before,
            );

            cx.emit(EditorEvent::Changed);
            cx.notify();
        }
    }

    pub fn set_content(&mut self, content: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.replace_content(content.clone());
        self.cursor = 0;
        self.selected_range = 0..0;
        self.marked_range = None;
        self.display_map.update(&content);
        cx.notify();
    }

    #[allow(dead_code)]
    pub fn set_document(
        &mut self,
        document: Document,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = document.content();
        self.buffer.replace_document(document);
        self.cursor = 0;
        self.selected_range = 0..0;
        self.marked_range = None;
        self.display_map.update(&content);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    /// Replace a byte range in the rope buffer with new text. O(log n).
    pub(crate) fn rope_replace(&mut self, range: Range<usize>, new_text: &str) {
        self.buffer.replace_range(range, new_text);
        self.display_map.invalidate();
    }

    /// Internal text mutation — bypasses OS input guard.
    /// Used by all commands that need to modify buffer content programmatically.
    pub(crate) fn edit_text(&mut self, new_text: &str, cx: &mut Context<Self>) {
        let range = self
            .marked_range
            .clone()
            .unwrap_or(self.selected_range.clone());
        self.edit_text_with_cursor(new_text, range.start + new_text.len(), cx);
    }

    pub(crate) fn edit_text_with_cursor(
        &mut self,
        new_text: &str,
        cursor_after: usize,
        cx: &mut Context<Self>,
    ) {
        let range = self
            .marked_range
            .clone()
            .unwrap_or(self.selected_range.clone());

        let old_text = self.buffer.slice_bytes(range.clone());
        let cursor_before = self.cursor;
        let selection_before = self.selected_range.clone();

        self.rope_replace(range.clone(), new_text);

        self.buffer.record_edit(
            undo::EditOp {
                range: range.clone(),
                old_text,
                new_text: new_text.to_string(),
                cursor_before,
                cursor_after,
            },
            selection_before,
        );

        self.selected_range = cursor_after..cursor_after;
        self.cursor = cursor_after;
        self.marked_range.take();
        self.needs_scroll_to_cursor = true;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn apply_code_edit(&mut self, replacement: code_edit::CodeEdit, cx: &mut Context<Self>) {
        if replacement.range.is_empty() && replacement.text.is_empty() {
            self.move_to(replacement.cursor_after, cx);
            return;
        }
        self.selected_range = replacement.range;
        self.marked_range = None;
        self.edit_text_with_cursor(&replacement.text, replacement.cursor_after, cx);
    }

    fn try_code_insert(&mut self, text: &str, range: Range<usize>, cx: &mut Context<Self>) -> bool {
        let content = self.content();
        if let Some(edit) = code_edit::smart_insert(&content, range, text) {
            self.apply_code_edit(edit, cx);
            true
        } else {
            false
        }
    }

    fn try_code_backspace(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.selected_range.is_empty() || self.marked_range.is_some() {
            return false;
        }
        let content = self.content();
        if let Some(edit) = code_edit::smart_backspace(&content, self.cursor_offset()) {
            self.apply_code_edit(edit, cx);
            true
        } else {
            false
        }
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Move the cursor to `offset`. If the target sits on a folded
    /// (hidden) line, snap to the nearest visible line in the direction
    /// of travel — forward when the user moved forward, backward when
    /// the user moved backward. This is what makes `j` jump past a fold
    /// instead of bouncing back to the heading.
    pub fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let prefer_forward = offset > self.cursor;
        self.move_to_inner(offset, prefer_forward, cx);
    }

    /// Explicit "prefer forward on hidden line" version. Kept for callers
    /// (e.g. `next_grapheme`) that want to force forward regardless of
    /// the numeric comparison above.
    pub fn move_to_forward(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to_inner(offset, true, cx);
    }

    fn move_to_inner(&mut self, offset: usize, prefer_forward: bool, cx: &mut Context<Self>) {
        let mut offset = self.snap_to_char_boundary(offset);
        // If the target is on a hidden line, snap to nearest visible line
        if self.display_map.is_offset_hidden(offset) {
            let line = self.display_map.line_for_offset(offset);
            let (first, second) = if prefer_forward {
                (true, false)
            } else {
                (false, true)
            };
            if let Some(vis) = self.display_map.next_visible_line(line, first) {
                offset = self.display_map.line_offset(vis);
            } else if let Some(vis) = self.display_map.next_visible_line(line, second) {
                offset = self.display_map.line_offset(vis);
            }
        }
        self.selected_range = offset..offset;
        self.cursor = offset;
        self.needs_scroll_to_cursor = true;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    /// Snap a byte offset to the nearest valid UTF-8 char boundary (rounding down).
    fn snap_to_char_boundary(&self, offset: usize) -> usize {
        let content = self.content();
        let mut p = offset.min(content.len());
        while p > 0 && !content.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    /// Return the selection range in (start, end) order.
    fn ordered_selection(&self) -> (usize, usize) {
        let s = self.selected_range.start;
        let e = self.selected_range.end;
        if s <= e { (s, e) } else { (e, s) }
    }

    pub fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.snap_to_char_boundary(offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.cursor = self.cursor_offset();
        cx.notify();
    }

    /// If the cursor is on a hidden (folded) line, move it to the nearest
    /// visible line above (the heading that folded it).
    pub fn ensure_cursor_visible(&mut self, cx: &mut Context<Self>) {
        if !self.display_map.is_offset_hidden(self.cursor) {
            return;
        }
        let line = self.display_map.line_for_offset(self.cursor);
        // Move to the nearest visible line above
        if let Some(vis) = self.display_map.next_visible_line(line, false) {
            let offset = self.display_map.line_offset(vis);
            self.move_to(offset, cx);
        } else if let Some(vis) = self.display_map.next_visible_line(line, true) {
            let offset = self.display_map.line_offset(vis);
            self.move_to(offset, cx);
        }
    }

    /// Recompute outline fold visibility and update display map.
    pub fn refresh_outline_visibility(&mut self, cx: &mut Context<Self>) {
        let kinds = self.display_map.line_kinds();
        let headings = outline::extract_headings(&kinds);
        let line_count = self.display_map.line_count();
        let hidden = self.outline.compute_hidden_lines(&headings, line_count);
        self.display_map.update_visibility(&hidden);
        self.ensure_cursor_visible(cx);
        cx.notify();
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let txn = match self.buffer.undo() {
            Some(t) => t,
            None => return,
        };

        // Apply inverse operations in reverse order
        for inv_op in txn.inverse_ops() {
            self.rope_replace(inv_op.range.clone(), &inv_op.new_text);
        }

        // Restore cursor/selection to before the transaction
        self.selected_range = txn.selection_before.clone();
        self.cursor = if self.selection_reversed {
            txn.selection_before.start
        } else {
            txn.selection_before.end
        };
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        let txn = match self.buffer.redo() {
            Some(t) => t,
            None => return,
        };

        // Re-apply all operations in forward order
        for op in &txn.ops {
            self.rope_replace(op.range.clone(), &op.new_text);
        }

        // Restore cursor/selection to after the transaction
        self.selected_range = txn.selection_after.clone();
        self.cursor = if self.selection_reversed {
            txn.selection_after.start
        } else {
            txn.selection_after.end
        };
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Execute an editor command. This is the central dispatch point
    /// for all editor operations.
    pub fn dispatch(
        &mut self,
        cmd: commands::EditorCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use commands::EditorCommand::*;
        match cmd {
            MoveLeft => {
                if self.selected_range.is_empty() {
                    self.move_to(self.prev_grapheme(self.cursor_offset()), cx);
                } else {
                    self.move_to(self.selected_range.start, cx);
                }
            }
            MoveRight => {
                if self.selected_range.is_empty() {
                    self.move_to_forward(self.next_grapheme(self.cursor_offset()), cx);
                } else {
                    self.move_to_forward(self.selected_range.end, cx);
                }
            }
            MoveUp => {
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let current_line = self.display_map.line_for_offset(pos);
                let before = &content[..pos];
                let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                let mut target_line = self.display_map.next_visible_line(current_line, false);
                while let Some(line) = target_line {
                    if !self.line_is_diagram_embed(line, &content) {
                        break;
                    }
                    target_line = self.display_map.next_visible_line(line, false);
                }
                if let Some(tl) = target_line {
                    let tl_start = self.display_map.line_offset(tl);
                    let tl_end = content[tl_start..]
                        .find('\n')
                        .map(|p| tl_start + p)
                        .unwrap_or(content.len());
                    let tl_len = tl_end - tl_start;
                    self.move_to(tl_start + col.min(tl_len), cx);
                }
                // else: no visible line above — stay put
            }
            MoveDown => {
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let before = &content[..pos];
                let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                let current_line = self.display_map.line_for_offset(pos);
                let mut target_line = self.display_map.next_visible_line(current_line, true);
                while let Some(line) = target_line {
                    if !self.line_is_diagram_embed(line, &content) {
                        break;
                    }
                    target_line = self.display_map.next_visible_line(line, true);
                }
                if let Some(tl) = target_line {
                    let tl_start = self.display_map.line_offset(tl);
                    let rest = &content[tl_start..];
                    let tl_len = rest.find('\n').unwrap_or(rest.len());
                    self.move_to(tl_start + col.min(tl_len), cx);
                }
                // else: no visible line below — stay put
            }
            MoveLineStart => {
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.move_to(line_start, cx);
            }
            MoveLineEnd => {
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_end = content[pos..]
                    .find('\n')
                    .map(|p| pos + p)
                    .unwrap_or(content.len());
                self.move_to(line_end, cx);
            }
            MoveToOffset(offset) => {
                self.move_to(offset, cx);
            }
            SelectLeft => {
                self.select_to(self.prev_grapheme(self.cursor_offset()), cx);
            }
            SelectRight => {
                self.select_to(self.next_grapheme(self.cursor_offset()), cx);
            }
            SelectUp => {
                // Extend selection upward (same logic as MoveUp but with select_to)
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor_offset());
                let before = &content[..pos];
                let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                if line_start == 0 {
                    self.select_to(0, cx);
                } else {
                    let prev_end = line_start - 1;
                    let prev_start = content[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let prev_len = prev_end - prev_start;
                    self.select_to(prev_start + col.min(prev_len), cx);
                }
            }
            SelectDown => {
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor_offset());
                let before = &content[..pos];
                let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                let after = &content[pos..];
                if let Some(nl) = after.find('\n') {
                    let next_start = pos + nl + 1;
                    let rest = &content[next_start..];
                    let next_len = rest.find('\n').unwrap_or(rest.len());
                    self.select_to(next_start + col.min(next_len), cx);
                } else {
                    self.select_to(content.len(), cx);
                }
            }
            SelectToOffset(offset) => {
                self.select_to(offset, cx);
            }
            SelectAll => {
                self.selected_range = 0..self.content_len();
                self.selection_reversed = false;
                self.cursor = self.content_len();
                cx.notify();
            }
            DeleteSelection => {
                if !self.selected_range.is_empty() {
                    self.edit_text("", cx);
                }
            }
            YankSelection => {
                if !self.selected_range.is_empty() {
                    let text = self.buffer.slice_bytes(self.selected_range.clone());
                    self.grammar.register_content = text;
                    // Collapse selection
                    let pos = self.selected_range.start;
                    self.move_to(pos, cx);
                }
            }
            YankText(text) => {
                self.grammar.register_content = text;
            }
            DeleteBackward => {
                if self.try_code_backspace(cx) {
                    return;
                }
                if self.selected_range.is_empty() {
                    self.select_to(self.prev_grapheme(self.cursor_offset()), cx);
                }
                self.edit_text("", cx);
            }
            DeleteForward => {
                if self.selected_range.is_empty() {
                    self.select_to(self.next_grapheme(self.cursor_offset()), cx);
                }
                self.edit_text("", cx);
            }
            DeleteRange(range) => {
                self.selected_range = range;
                self.edit_text("", cx);
            }
            InsertNewline => {
                let range = self.selected_range.clone();
                if !self.try_code_insert("\n", range, cx) {
                    self.edit_text("\n", cx);
                }
            }
            InsertTab => {
                self.edit_text("    ", cx);
            }
            InsertChar(ch) => {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                let range = self.selected_range.clone();
                if !self.try_code_insert(s, range, cx) {
                    self.edit_text(s, cx);
                }
            }
            InsertText(text) => {
                self.edit_text(&text, cx);
            }
            Undo => self.undo(cx),
            Redo => self.redo(cx),
            TableNextCell => {
                self.handle_table_tab(true, cx);
            }
            TablePrevCell => {
                self.handle_table_tab(false, cx);
            }
            EnterMode(_) => {
                // Handled via execute_grammar_result / execute_command_by_id
                // which have access to the keymap
            }
            ToggleVimMode => {
                // Handled via execute_command_by_id which has access to the keymap
            }
            IndentSelection => {
                let (start, end) = self.ordered_selection();
                if start < end {
                    let content = self.content();
                    let line_start = content[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let line_end = content[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    // Indent each line in selection
                    let mut offset = 0usize;
                    let mut pos = line_start;
                    while pos <= line_end {
                        let insert_at = pos + offset;
                        self.selected_range = insert_at..insert_at;
                        self.edit_text("    ", cx);
                        offset += 4;
                        if let Some(nl) = content[pos..].find('\n') {
                            pos = pos + nl + 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            DedentSelection => {
                let (start, end) = self.ordered_selection();
                if start < end {
                    let content = self.content();
                    let line_start = content[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let mut pos = line_start;
                    let mut offset: isize = 0;
                    while pos <= end.saturating_add_signed(offset) {
                        let actual = (pos as isize + offset) as usize;
                        let current = self.content();
                        let spaces = current[actual..]
                            .chars()
                            .take_while(|c| *c == ' ')
                            .count()
                            .min(4);
                        if spaces > 0 {
                            self.selected_range = actual..actual + spaces;
                            self.edit_text("", cx);
                            offset -= spaces as isize;
                        }
                        if let Some(nl) = content[pos..].find('\n') {
                            pos = pos + nl + 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            JoinSelection => {
                let (start, end) = self.ordered_selection();
                let content = self.content();
                // Find all newlines in selection and replace with space
                let selected = &content[start..end];
                let joined = selected.replace('\n', " ");
                self.selected_range = start..end;
                self.edit_text(&joined, cx);
            }
            ToggleCaseSelection => {
                let (start, end) = self.ordered_selection();
                let content = self.content();
                let selected = &content[start..end];
                let toggled: String = selected
                    .chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().next().unwrap_or(c)
                        } else {
                            c.to_uppercase().next().unwrap_or(c)
                        }
                    })
                    .collect();
                self.selected_range = start..end;
                self.edit_text(&toggled, cx);
            }
            UppercaseSelection => {
                let (start, end) = self.ordered_selection();
                let content = self.content();
                let upper = content[start..end].to_uppercase();
                self.selected_range = start..end;
                self.edit_text(&upper, cx);
            }
            LowercaseSelection => {
                let (start, end) = self.ordered_selection();
                let content = self.content();
                let lower = content[start..end].to_lowercase();
                self.selected_range = start..end;
                self.edit_text(&lower, cx);
            }
            OutlineCycleFold => {
                let content = self.content();
                self.display_map.update(&content);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                let line_count = self.display_map.line_count();
                // Find heading at or above cursor
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let heading = outline::heading_for_line(cursor_line, &headings);
                if let Some(hi) = heading {
                    let hl = hi.line_idx;
                    self.outline.cycle_heading(hl, &headings, line_count);
                    self.refresh_outline_visibility(cx);
                } else {
                    // Not on/under a heading — fall through to insert tab
                    self.dispatch(commands::EditorCommand::InsertTab, window, cx);
                }
            }
            OutlineGlobalCycle => {
                // Table shift-tab takes priority over global cycle
                if self.handle_table_tab(false, cx) {
                    return;
                }
                let content = self.content();
                self.display_map.update(&content);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                self.outline.global_cycle(&headings);
                self.refresh_outline_visibility(cx);
            }
            OutlinePromote => {
                let content = self.content();
                self.display_map.update(&content);
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                if let Some(hi) = headings.iter().find(|h| h.line_idx == cursor_line) {
                    if hi.level > 1 {
                        let line_start = self.display_map.line_offset(cursor_line);
                        // Remove one '#' from the heading prefix
                        let line_end = content[line_start..]
                            .find('\n')
                            .map(|p| line_start + p)
                            .unwrap_or(content.len());
                        let line_text = &content[line_start..line_end];
                        if line_text.contains(' ') {
                            // Remove first '#'
                            self.selected_range = line_start..line_start + 1;
                            self.edit_text("", cx);
                        }
                    }
                }
            }
            OutlineDemote => {
                let content = self.content();
                self.display_map.update(&content);
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                if let Some(hi) = headings.iter().find(|h| h.line_idx == cursor_line) {
                    if hi.level < 6 {
                        let line_start = self.display_map.line_offset(cursor_line);
                        // Add one '#' at the start of the heading
                        self.selected_range = line_start..line_start;
                        self.edit_text("#", cx);
                    }
                }
            }
            OutlineMoveUp => {
                let content = self.content();
                self.display_map.update(&content);
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                let line_count = self.display_map.line_count();
                if let Some(hi_idx) = headings.iter().position(|h| h.line_idx == cursor_line) {
                    if let Some(prev) = outline::prev_sibling(cursor_line, &headings) {
                        let section_end = outline::section_end_line(hi_idx, &headings, line_count);
                        let my_start = self.display_map.line_offset(cursor_line);
                        let my_end = if section_end < line_count {
                            self.display_map.line_offset(section_end)
                        } else {
                            content.len()
                        };
                        let prev_start = self.display_map.line_offset(prev.line_idx);
                        let my_text = content[my_start..my_end].to_string();
                        let prev_text = content[prev_start..my_start].to_string();
                        // Swap: replace [prev_start..my_end] with [my_text + prev_text]
                        self.selected_range = prev_start..my_end;
                        let swapped = format!("{}{}", my_text, prev_text);
                        self.edit_text(&swapped, cx);
                        // Move cursor to the new position of the heading
                        self.move_to(prev_start, cx);
                    }
                }
            }
            OutlineMoveDown => {
                let content = self.content();
                self.display_map.update(&content);
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                let line_count = self.display_map.line_count();
                if let Some(hi_idx) = headings.iter().position(|h| h.line_idx == cursor_line) {
                    if let Some(next) = outline::next_sibling(cursor_line, &headings) {
                        let next_idx = headings
                            .iter()
                            .position(|h| h.line_idx == next.line_idx)
                            .unwrap();
                        let _my_section_end =
                            outline::section_end_line(hi_idx, &headings, line_count);
                        let next_section_end =
                            outline::section_end_line(next_idx, &headings, line_count);
                        let my_start = self.display_map.line_offset(cursor_line);
                        let next_start = self.display_map.line_offset(next.line_idx);
                        let next_end = if next_section_end < line_count {
                            self.display_map.line_offset(next_section_end)
                        } else {
                            content.len()
                        };
                        let my_text = content[my_start..next_start].to_string();
                        let next_text = content[next_start..next_end].to_string();
                        // Swap: replace [my_start..next_end] with [next_text + my_text]
                        self.selected_range = my_start..next_end;
                        let swapped = format!("{}{}", next_text, my_text);
                        let new_cursor = my_start + next_text.len();
                        self.edit_text(&swapped, cx);
                        self.move_to(new_cursor, cx);
                    }
                }
            }
            OutlineNextHeading => {
                let content = self.content();
                self.display_map.update(&content);
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                if let Some(hi) = outline::next_heading(cursor_line, &headings) {
                    let offset = self.display_map.line_offset(hi.line_idx);
                    self.move_to(offset, cx);
                }
            }
            OutlinePrevHeading => {
                let content = self.content();
                self.display_map.update(&content);
                let cursor_line = self.display_map.line_for_offset(self.cursor);
                let kinds = self.display_map.line_kinds();
                let headings = outline::extract_headings(&kinds);
                // If cursor is on a heading, go to the previous one
                // If cursor is on body text, go to the heading above
                if let Some(hi) = outline::prev_heading(cursor_line, &headings) {
                    let offset = self.display_map.line_offset(hi.line_idx);
                    self.move_to(offset, cx);
                }
            }
        }
    }

    /// Execute a GrammarResult from the keymap system.
    pub fn execute_grammar_result(
        &mut self,
        result: crate::keymap::GrammarResult,
        count: usize,
        vim: crate::pane::VimSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        use crate::keymap::GrammarResult;
        let mut actions = Vec::new();
        match result {
            GrammarResult::MoveCursor(offset) => {
                if vim.visual_active {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
            GrammarResult::InsertChar(ch) => {
                self.dispatch(commands::EditorCommand::InsertChar(ch), window, cx);
            }
            GrammarResult::DeleteRange {
                range,
                yanked,
                enter_insert,
            } => {
                self.grammar.register_content = yanked;
                if enter_insert {
                    self.begin_vim_edit_group();
                }
                let target = range.start;
                self.selected_range = range;
                self.edit_text("", cx);
                self.move_to(target.min(self.content_len()), cx);
                if enter_insert {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                    self.insert_mode = true;
                    self.buffer.break_undo_coalescing();
                    cx.notify();
                }
            }
            GrammarResult::Yank(text) => {
                self.grammar.register_content = text;
            }
            GrammarResult::IndentRange { line_start, text } => {
                let at = line_start;
                self.selected_range = at..at;
                self.edit_text(&text, cx);
            }
            GrammarResult::DedentRange { range } => {
                self.selected_range = range;
                self.edit_text("", cx);
            }
            GrammarResult::ExecuteCommand(cmd_id) => {
                let sub_actions = self.execute_command_by_id(cmd_id, count, vim, window, cx);
                actions.extend(sub_actions);
            }
            GrammarResult::Batch(results) => {
                for r in results {
                    let sub_actions = self.execute_grammar_result(r, count, vim, window, cx);
                    actions.extend(sub_actions);
                }
            }
            GrammarResult::SetVimMode(mode) => {
                self.on_vim_mode_changed(mode, cx);
                actions.push(ItemAction::SetVimMode(mode));
                actions.push(ItemAction::SyncVimFlags);
            }
            GrammarResult::PushTransient(transient) => {
                actions.push(ItemAction::PushTransient(transient));
            }
            GrammarResult::ReplaceChar { ch, count } => {
                let content = self.content();
                let pos = self.cursor;
                let mut end = pos;
                for _ in 0..count {
                    if end < content.len() {
                        end = {
                            let mut p = end + 1;
                            while p < content.len() && !content.is_char_boundary(p) {
                                p += 1;
                            }
                            p
                        };
                    }
                }
                if end > pos {
                    let replacement: String = std::iter::repeat(ch).take(count).collect();
                    self.selected_range = pos..end;
                    self.edit_text(&replacement, cx);
                }
            }
            GrammarResult::Pending | GrammarResult::Noop => {}
        }
        actions
    }

    /// Handle Vim mode side effects.
    pub fn on_vim_mode_changed(&mut self, mode: crate::keymap::VimMode, cx: &mut Context<Self>) {
        match mode {
            crate::keymap::VimMode::Insert => {
                self.begin_vim_edit_group();
            }
            crate::keymap::VimMode::Normal => {
                self.end_vim_edit_group();
                self.visual_line_anchor = None;
                if !self.selected_range.is_empty() {
                    let pos = self.selected_range.start;
                    self.selected_range = pos..pos;
                    self.cursor = pos;
                }
                self.buffer.break_undo_coalescing();
            }
            crate::keymap::VimMode::Visual => {
                self.visual_line_anchor = None;
                if self.selected_range.is_empty() {
                    let pos = self.cursor;
                    let end = self.next_grapheme(pos);
                    self.selected_range = pos..end;
                    self.selection_reversed = false;
                }
            }
            crate::keymap::VimMode::VisualLine => {
                let content = self.content();
                let line_start = line_start_at(&content, self.cursor);
                let line_end = line_end_including_newline(&content, line_start);
                self.visual_line_anchor = Some(line_start);
                self.selected_range = line_start..line_end;
                self.selection_reversed = false;
                self.cursor = line_start;
            }
            crate::keymap::VimMode::Operator => {}
        }
        cx.notify();
    }

    fn begin_vim_edit_group(&mut self) {
        if !self.vim_edit_group_active {
            self.buffer.break_undo_coalescing();
            self.buffer.begin_edit_group(self.selected_range.clone());
            self.vim_edit_group_active = true;
        }
    }

    fn end_vim_edit_group(&mut self) {
        if self.vim_edit_group_active {
            self.buffer.end_edit_group();
            self.vim_edit_group_active = false;
        }
    }

    fn extend_visual_line_selection(&mut self, line_delta: isize, cx: &mut Context<Self>) {
        let content = self.content();
        let anchor = self
            .visual_line_anchor
            .unwrap_or_else(|| line_start_at(&content, self.cursor));
        let current = line_start_at(&content, self.cursor);
        let target = move_line_start(&content, current, line_delta);
        let range = visual_line_range(&content, anchor, target);

        self.visual_line_anchor = Some(anchor);
        self.selected_range = range;
        self.selection_reversed = target < anchor;
        self.cursor = target;
        self.needs_scroll_to_cursor = true;
        cx.notify();
    }

    /// Handle f/t/r transient char captures.
    pub fn handle_transient_capture(
        &mut self,
        transient: crate::keymap::TransientKind,
        ch: char,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = self.content();
        let cursor = self.cursor;
        match transient {
            crate::keymap::TransientKind::ReplaceChar => {
                if cursor < content.len() {
                    let next_char_len =
                        content[cursor..].chars().next().map_or(1, |c| c.len_utf8());
                    let mut new_content = content.clone();
                    new_content.replace_range(cursor..cursor + next_char_len, &ch.to_string());
                    self.set_content(new_content, window, cx);
                }
                cx.notify();
            }
            kind @ (crate::keymap::TransientKind::FindChar
            | crate::keymap::TransientKind::TilChar
            | crate::keymap::TransientKind::FindCharBack
            | crate::keymap::TransientKind::TilCharBack) => {
                let pos = match kind {
                    crate::keymap::TransientKind::FindChar => {
                        crate::keymap::find_char_forward(&content, cursor, ch, count)
                    }
                    crate::keymap::TransientKind::TilChar => {
                        crate::keymap::til_char_forward(&content, cursor, ch, count)
                    }
                    crate::keymap::TransientKind::FindCharBack => {
                        crate::keymap::find_char_backward(&content, cursor, ch, count)
                    }
                    crate::keymap::TransientKind::TilCharBack => {
                        crate::keymap::til_char_backward(&content, cursor, ch, count)
                    }
                    crate::keymap::TransientKind::ReplaceChar => cursor,
                };
                // Map to static str for storage
                let static_kind: &'static str = match kind {
                    crate::keymap::TransientKind::FindChar => "find-char",
                    crate::keymap::TransientKind::TilChar => "til-char",
                    crate::keymap::TransientKind::FindCharBack => "find-char-back",
                    crate::keymap::TransientKind::TilCharBack => "til-char-back",
                    crate::keymap::TransientKind::ReplaceChar => unreachable!(),
                };
                self.grammar.last_char_search = Some((ch, static_kind));
                self.cursor = pos;
                cx.notify();
            }
        }
    }

    /// Process a vim grammar action (Motion, Operator, SelfInsert, etc.).
    /// Returns ItemActions for any keymap state changes needed.
    pub fn process_vim_action(
        &mut self,
        action: crate::keymap::Action,
        key: &str,
        count: usize,
        vim: crate::pane::VimSnapshot,
        registry: &crate::keymap::BindingRegistry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        let content = self.content();
        let cursor = self.cursor;
        let result = self
            .grammar
            .process(action, key, count, &content, cursor, registry);
        let actions = self.execute_grammar_result(result, count, vim, window, cx);
        actions
    }

    /// Sync vim mode flags from keymap state.
    pub fn sync_vim_flags(&mut self, vim_enabled: bool, insert_active: bool) {
        self.vim_enabled = vim_enabled;
        self.insert_mode = insert_active;
    }

    /// Execute a named command (from keymap Command actions).
    pub fn execute_command_by_id(
        &mut self,
        cmd_id: &str,
        count: usize,
        vim: crate::pane::VimSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        use commands::EditorCommand;
        let mut actions = Vec::new();
        match cmd_id {
            // Basic editing
            "delete-backward" => self.dispatch(EditorCommand::DeleteBackward, window, cx),
            "delete-forward" => self.dispatch(EditorCommand::DeleteForward, window, cx),
            "insert-newline" => self.dispatch(EditorCommand::InsertNewline, window, cx),
            "insert-tab" => self.dispatch(EditorCommand::InsertTab, window, cx),
            "table-next-cell" => self.dispatch(EditorCommand::TableNextCell, window, cx),
            "table-prev-cell" => self.dispatch(EditorCommand::TablePrevCell, window, cx),
            // Selection
            "select-left" => self.dispatch(EditorCommand::SelectLeft, window, cx),
            "select-right" => self.dispatch(EditorCommand::SelectRight, window, cx),
            "select-up" => self.dispatch(EditorCommand::SelectUp, window, cx),
            "select-down" => self.dispatch(EditorCommand::SelectDown, window, cx),
            // History
            "undo" => self.dispatch(EditorCommand::Undo, window, cx),
            "redo" => self.dispatch(EditorCommand::Redo, window, cx),
            // Vim mode
            "toggle-vim" => {
                actions.push(ItemAction::SetVimEnabled(!vim.vim_enabled));
                actions.push(ItemAction::SyncVimFlags);
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            // Visual mode operations
            "delete-selection" => {
                if vim.visual_active {
                    let (s, e) = self.ordered_selection();
                    let content = self.content();
                    self.grammar.register_content = content[s..e].to_string();
                }
                self.dispatch(EditorCommand::DeleteSelection, window, cx);
                if vim.visual_active {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Normal));
                    cx.notify();
                }
            }
            "yank-selection" => {
                self.dispatch(EditorCommand::YankSelection, window, cx);
                if vim.visual_active {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Normal));
                    cx.notify();
                }
            }
            "change-selection" => {
                self.begin_vim_edit_group();
                if vim.visual_active {
                    let (s, e) = self.ordered_selection();
                    let content = self.content();
                    self.grammar.register_content = content[s..e].to_string();
                }
                self.dispatch(EditorCommand::DeleteSelection, window, cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "visual-line-down" => self.extend_visual_line_selection(count as isize, cx),
            "visual-line-up" => self.extend_visual_line_selection(-(count as isize), cx),
            "indent-selection" => self.dispatch(EditorCommand::IndentSelection, window, cx),
            "dedent-selection" => self.dispatch(EditorCommand::DedentSelection, window, cx),
            "toggle-case-selection" => {
                self.dispatch(EditorCommand::ToggleCaseSelection, window, cx);
                if vim.visual_active {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Normal));
                    cx.notify();
                }
            }
            "uppercase-selection" => {
                self.dispatch(EditorCommand::UppercaseSelection, window, cx);
                if vim.visual_active {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Normal));
                    cx.notify();
                }
            }
            "lowercase-selection" => {
                self.dispatch(EditorCommand::LowercaseSelection, window, cx);
                if vim.visual_active {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Normal));
                    cx.notify();
                }
            }
            "join-selection" => {
                self.dispatch(EditorCommand::JoinSelection, window, cx);
                if vim.visual_active {
                    actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Normal));
                    cx.notify();
                }
            }
            // Vim normal mode commands
            "delete-char-forward" => {
                // x — delete char under cursor
                let content = self.content();
                let pos = self.cursor;
                if pos < content.len() {
                    let end = {
                        let mut p = pos + 1;
                        while p < content.len() && !content.is_char_boundary(p) {
                            p += 1;
                        }
                        p
                    };
                    self.grammar.register_content = content[pos..end].to_string();
                    self.selected_range = pos..end;
                    self.edit_text("", cx);
                }
            }
            "delete-char-backward" => {
                // X — delete char before cursor
                let pos = self.cursor;
                if pos > 0 {
                    let content = self.content();
                    let start = {
                        let mut p = pos - 1;
                        while p > 0 && !content.is_char_boundary(p) {
                            p -= 1;
                        }
                        p
                    };
                    self.grammar.register_content = content[start..pos].to_string();
                    self.selected_range = start..pos;
                    self.edit_text("", cx);
                }
            }
            "substitute-char" => {
                let content = self.content();
                let pos = self.cursor;
                let mut end = pos;
                for _ in 0..count {
                    if end >= content.len() || content[end..].starts_with('\n') {
                        break;
                    }
                    end = self.next_grapheme(end);
                }
                if end > pos {
                    self.begin_vim_edit_group();
                    self.grammar.register_content = content[pos..end].to_string();
                    self.selected_range = pos..end;
                    self.edit_text("", cx);
                }
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "yank-line" => {
                let content = self.content();
                let line_start = content[..self.cursor]
                    .rfind('\n')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let mut end = line_start;
                for _ in 0..count {
                    end = content[end..]
                        .find('\n')
                        .map(|offset| end + offset + 1)
                        .unwrap_or(content.len());
                    if end == content.len() {
                        break;
                    }
                }
                self.grammar.register_content = content[line_start..end].to_string();
            }
            "append-after" => {
                // a — move right one char and enter insert
                let pos = self.next_grapheme(self.cursor);
                self.move_to(pos, cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "insert-at-line-start" => {
                // I — move to first non-whitespace and enter insert
                let content = self.content();
                let target = crate::keymap::motion_first_non_whitespace(&content, self.cursor, 1);
                self.move_to(target, cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "insert-at-line-end" => {
                // A — move to end of line and enter insert
                let content = self.content();
                let target = crate::keymap::motion_line_end(&content, self.cursor, 1);
                self.move_to(target, cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "open-line-below" => {
                // o — open line below and enter insert
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_end = content[pos..]
                    .find('\n')
                    .map(|p| pos + p)
                    .unwrap_or(content.len());
                self.begin_vim_edit_group();
                self.move_to(line_end, cx);
                self.edit_text("\n", cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "open-line-above" => {
                // O — open line above and enter insert
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.begin_vim_edit_group();
                self.move_to(line_start, cx);
                self.edit_text("\n", cx);
                let new_pos = line_start;
                self.move_to(new_pos, cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "paste-after" => {
                // p — paste after cursor
                let text = self.grammar.register_content.clone();
                if !text.is_empty() {
                    if text.ends_with('\n') {
                        // Line-wise paste: insert after current line
                        let content = self.content();
                        let pos = self.snap_to_char_boundary(self.cursor);
                        let line_end = content[pos..]
                            .find('\n')
                            .map(|p| pos + p + 1)
                            .unwrap_or(content.len());
                        self.move_to(line_end, cx);
                        self.edit_text(&text, cx);
                        self.move_to(line_end, cx);
                    } else {
                        let pos = self.next_grapheme(self.cursor);
                        self.move_to(pos, cx);
                        self.edit_text(&text, cx);
                        // Move cursor to end of pasted text - 1
                        let end = pos + text.len();
                        self.move_to(end.saturating_sub(1).min(self.content_len()), cx);
                    }
                }
            }
            "paste-before" => {
                // P — paste before cursor
                let text = self.grammar.register_content.clone();
                if !text.is_empty() {
                    if text.ends_with('\n') {
                        let content = self.content();
                        let pos = self.snap_to_char_boundary(self.cursor);
                        let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                        self.move_to(line_start, cx);
                        self.edit_text(&text, cx);
                        self.move_to(line_start, cx);
                    } else {
                        let pos = self.cursor;
                        self.edit_text(&text, cx);
                        self.move_to(pos, cx);
                    }
                }
            }
            "join-lines" => {
                // J — join current line with next
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                if let Some(nl) = content[pos..].find('\n') {
                    let nl_pos = pos + nl;
                    // Remove newline and leading whitespace on next line
                    let after = &content[nl_pos + 1..];
                    let ws = after.len() - after.trim_start().len();
                    let range = nl_pos..nl_pos + 1 + ws;
                    self.selected_range = range;
                    self.edit_text(" ", cx);
                    self.move_to(nl_pos, cx);
                }
            }
            "delete-to-end" => {
                // D — delete from cursor to end of line
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_end = content[pos..]
                    .find('\n')
                    .map(|p| pos + p)
                    .unwrap_or(content.len());
                if pos < line_end {
                    self.grammar.register_content = content[pos..line_end].to_string();
                    self.selected_range = pos..line_end;
                    self.edit_text("", cx);
                }
            }
            "change-to-end" => {
                // C — change from cursor to end of line
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_end = content[pos..]
                    .find('\n')
                    .map(|p| pos + p)
                    .unwrap_or(content.len());
                self.begin_vim_edit_group();
                if pos < line_end {
                    self.grammar.register_content = content[pos..line_end].to_string();
                    self.selected_range = pos..line_end;
                    self.edit_text("", cx);
                }
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "change-line" => {
                // S — change entire line
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let line_end = content[pos..]
                    .find('\n')
                    .map(|p| pos + p)
                    .unwrap_or(content.len());
                self.begin_vim_edit_group();
                self.grammar.register_content = content[line_start..line_end].to_string();
                self.selected_range = line_start..line_end;
                self.edit_text("", cx);
                actions.push(ItemAction::SetVimMode(crate::keymap::VimMode::Insert));
                self.buffer.break_undo_coalescing();
                cx.notify();
            }
            "toggle-case" => {
                // ~ — toggle case of char under cursor and advance
                let content = self.content();
                let pos = self.cursor;
                if pos < content.len() {
                    if let Some(ch) = content[pos..].chars().next() {
                        let toggled = if ch.is_uppercase() {
                            ch.to_lowercase().to_string()
                        } else {
                            ch.to_uppercase().to_string()
                        };
                        let end = pos + ch.len_utf8();
                        self.selected_range = pos..end;
                        self.edit_text(&toggled, cx);
                        let new_pos = pos + toggled.len();
                        self.move_to(new_pos.min(self.content_len()), cx);
                    }
                }
            }
            "dot-repeat" => {
                if let Some(transaction) = self.buffer.last_transaction() {
                    let base = transaction
                        .ops
                        .first()
                        .map(|op| op.cursor_before)
                        .unwrap_or(0);
                    let repeat_at = self.cursor;
                    self.buffer.break_undo_coalescing();
                    self.buffer.begin_edit_group(self.selected_range.clone());
                    for _ in 0..count {
                        for op in &transaction.ops {
                            let start = repeat_at
                                .saturating_add_signed(op.range.start as isize - base as isize);
                            let end = start
                                .saturating_add(op.old_text.len())
                                .min(self.content_len());
                            let content = self.content();
                            if start <= end
                                && content.is_char_boundary(start)
                                && content.is_char_boundary(end)
                            {
                                self.selected_range = start..end;
                                self.edit_text(&op.new_text, cx);
                            }
                        }
                    }
                    self.buffer.end_edit_group();
                    self.buffer.break_undo_coalescing();
                }
            }
            "repeat-char-search" => {
                let content = self.content();
                let cursor = self.cursor;
                if let Some(target) =
                    crate::keymap::repeat_char_search(&self.grammar, &content, cursor, count)
                {
                    if vim.visual_active {
                        self.select_to(target, cx);
                    } else {
                        self.move_to(target, cx);
                    }
                }
            }
            "repeat-char-search-reverse" => {
                let content = self.content();
                let cursor = self.cursor;
                if let Some(target) = crate::keymap::repeat_char_search_reverse(
                    &self.grammar,
                    &content,
                    cursor,
                    count,
                ) {
                    if vim.visual_active {
                        self.select_to(target, cx);
                    } else {
                        self.move_to(target, cx);
                    }
                }
            }
            // Scrolling — clamp to `(total - viewport).max(0)` so we don't
            // overscroll past the last line and leave the scrollbar thumb
            // stranded at the bottom.
            "scroll-half-down" => {
                self.scroll_by_amount(self.viewport_height / 2.0, cx);
            }
            "scroll-half-up" => {
                self.scroll_by_amount(-self.viewport_height / 2.0, cx);
            }
            "scroll-page-down" => self.scroll_by_amount(self.viewport_height, cx),
            "scroll-page-up" => self.scroll_by_amount(-self.viewport_height, cx),
            "scroll-line-down" => {
                let line = self.display_map.line_for_offset(self.cursor);
                self.scroll_by_amount(self.display_map.line_height(line) * count as f32, cx);
            }
            "scroll-line-up" => {
                let line = self.display_map.line_for_offset(self.cursor);
                self.scroll_by_amount(-self.display_map.line_height(line) * count as f32, cx);
            }
            // Go-to commands (from g prefix trie)
            "goto-doc-start" => {
                self.move_to(0, cx);
            }
            // App-level commands (forwarded via events)
            "save" => {
                cx.emit(EditorEvent::RequestSave);
            }
            "quit" => {
                cx.emit(EditorEvent::RequestQuit);
            }
            "command-palette" => {
                cx.emit(EditorEvent::RequestCommand);
            }
            "find-note" => {
                cx.emit(EditorEvent::RequestNoteSearch);
            }
            "vault-switch" => {
                cx.emit(EditorEvent::RequestVaultSwitch);
            }
            "vault-open" => {
                cx.emit(EditorEvent::RequestVaultOpen);
            }
            // Outline commands
            "outline-cycle-fold" => self.dispatch(EditorCommand::OutlineCycleFold, window, cx),
            "outline-global-cycle" => self.dispatch(EditorCommand::OutlineGlobalCycle, window, cx),
            "outline-promote" => self.dispatch(EditorCommand::OutlinePromote, window, cx),
            "outline-demote" => self.dispatch(EditorCommand::OutlineDemote, window, cx),
            "outline-move-up" => self.dispatch(EditorCommand::OutlineMoveUp, window, cx),
            "outline-move-down" => self.dispatch(EditorCommand::OutlineMoveDown, window, cx),
            "outline-next-heading" => self.dispatch(EditorCommand::OutlineNextHeading, window, cx),
            "outline-prev-heading" => self.dispatch(EditorCommand::OutlinePrevHeading, window, cx),
            "set" => {
                let (msg, set_actions) = self.handle_set_command("", &vim);
                actions.push(ItemAction::SetMessage(msg));
                actions.extend(set_actions);
            }
            _ => {
                let _ = (cmd_id, window, cx);
            }
        }
        actions
    }

    /// Execute an ex-style command (`:w`, `:q`, `:set`, etc.).
    /// Returns a status message string.
    pub fn execute_ex_command(
        &mut self,
        input: &str,
        vim: crate::pane::VimSnapshot,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        let mut actions = Vec::new();
        let input = input.trim();
        let (cmd, args) = match input.split_once(' ') {
            Some((c, a)) => (c, a.trim()),
            None => (input, ""),
        };

        match cmd {
            "w" | "write" => {
                cx.emit(EditorEvent::RequestSave);
                self.status_message = Some("Saving...".into());
            }
            "q" | "quit" => {
                cx.emit(EditorEvent::RequestQuit);
            }
            "wq" | "x" => {
                cx.emit(EditorEvent::RequestSave);
                cx.emit(EditorEvent::RequestQuit);
            }
            "set" => {
                let (msg, set_actions) = self.handle_set_command(args, &vim);
                actions.push(ItemAction::SetMessage(msg));
                actions.extend(set_actions);
            }
            "noh" | "nohlsearch" => {
                self.status_message = Some("Search highlighting cleared".into());
            }
            "e" | "edit" => {
                if args.is_empty() {
                    self.status_message = Some("Specify a file path".into());
                } else {
                    cx.emit(EditorEvent::RequestOpen(args.to_string()));
                }
            }
            "vault" | "vaults" | "vault-switch" | "switch-vault" => {
                cx.emit(EditorEvent::RequestVaultSwitch);
            }
            "vault-open" | "open-vault" => {
                cx.emit(EditorEvent::RequestVaultOpen);
            }
            "notes" | "find-note" | "find" | "note" => {
                cx.emit(EditorEvent::RequestNoteSearch);
            }
            _ => {
                self.status_message = Some(format!("E492: Not an editor command: {}", cmd));
            }
        }
        actions
    }

    pub fn handle_set_command(
        &mut self,
        args: &str,
        vim: &crate::pane::VimSnapshot,
    ) -> (String, Vec<ItemAction>) {
        if args.is_empty() {
            let mode_label = if vim.visual_active {
                "VIS"
            } else if vim.insert_active {
                "INS"
            } else {
                "NOR"
            };
            return (
                format!("vim={} mode={}", vim.vim_enabled, mode_label),
                vec![],
            );
        }

        match args {
            "vim" => (
                "Vim mode enabled".into(),
                vec![ItemAction::SetVimEnabled(true), ItemAction::SyncVimFlags],
            ),
            "novim" => (
                "Vim mode disabled".into(),
                vec![ItemAction::SetVimEnabled(false), ItemAction::SyncVimFlags],
            ),
            "number" | "nu" => ("Line numbers not yet implemented".into(), vec![]),
            "nonumber" | "nonu" => ("Line numbers not yet implemented".into(), vec![]),
            "wrap" => ("Soft wrap is enabled".into(), vec![]),
            "nowrap" => ("Soft wrap cannot be disabled yet".into(), vec![]),
            _ => (format!("Unknown option: {}", args), vec![]),
        }
    }

    // ─── PaneItem interface ─────────────────────────────────────────────────

    /// Commands for the command palette when the editor is active.
    pub fn commands() -> Vec<Command> {
        vec![
            Command {
                id: "outline-cycle-fold",
                name: "Outline: Toggle Fold",
                description: "Cycle fold state on current heading",
                aliases: &["fold", "toggle-fold"],
                binding: Some("Tab"),
            },
            Command {
                id: "outline-global-cycle",
                name: "Outline: Global Cycle",
                description: "Cycle all headings: overview → children → show all",
                aliases: &["fold-all", "unfold-all"],
                binding: Some("S-Tab"),
            },
            Command {
                id: "outline-promote",
                name: "Outline: Promote Heading",
                description: "Decrease heading level (## → #)",
                aliases: &["promote"],
                binding: Some("M-left"),
            },
            Command {
                id: "outline-demote",
                name: "Outline: Demote Heading",
                description: "Increase heading level (# → ##)",
                aliases: &["demote"],
                binding: Some("M-right"),
            },
            Command {
                id: "outline-move-up",
                name: "Outline: Move Subtree Up",
                description: "Swap heading subtree with previous sibling",
                aliases: &[],
                binding: Some("M-up"),
            },
            Command {
                id: "outline-move-down",
                name: "Outline: Move Subtree Down",
                description: "Swap heading subtree with next sibling",
                aliases: &[],
                binding: Some("M-down"),
            },
            Command {
                id: "outline-next-heading",
                name: "Outline: Next Heading",
                description: "Jump to next heading",
                aliases: &[],
                binding: Some("M-n"),
            },
            Command {
                id: "outline-prev-heading",
                name: "Outline: Previous Heading",
                description: "Jump to previous heading",
                aliases: &[],
                binding: Some("M-p"),
            },
        ]
    }

    /// Execute a command via the PaneItem interface.
    pub fn item_execute_command(
        &mut self,
        cmd_id: &str,
        _viewport: (f32, f32),
        vim: crate::pane::VimSnapshot,
        _cx: &mut Context<Self>,
    ) -> CommandOutcome {
        // Editor needs a window for dispatch — but we don't have one here.
        // Commands that need window go through execute_command_by_id which
        // is called from the key dispatch path. For command palette dispatch,
        // we return status messages as ItemActions.
        //
        // The full execute_command_by_id is called from process_editor_key
        // which has the window reference.
        //
        // For now, handle commands that don't need window + return actions for those that do.
        match cmd_id {
            "set" => {
                let (msg, set_actions) = self.handle_set_command("", &vim);
                let mut actions = vec![ItemAction::SetMessage(msg)];
                actions.extend(set_actions);
                CommandOutcome::handled(actions)
            }
            "toggle-vim" => CommandOutcome::handled(vec![
                ItemAction::SetVimEnabled(!vim.vim_enabled),
                ItemAction::SyncVimFlags,
            ]),
            _ => CommandOutcome::Unhandled,
        }
    }

    /// Get candidates for editor-owned delegates (none currently).
    pub fn item_get_candidates(&self, _delegate_id: &str, _input: &str) -> Vec<Candidate> {
        vec![]
    }

    /// Handle confirm for editor-owned delegates (none currently).
    pub fn item_handle_confirm(
        &mut self,
        _delegate_id: &str,
        _input: &str,
        _candidate: Option<&Candidate>,
    ) -> Vec<ItemAction> {
        vec![]
    }

    fn scroll_by_amount(&mut self, amount: Pixels, cx: &mut Context<Self>) {
        let total = <EditorState as crate::ui::Scrollable>::total_height(self);
        let viewport: f32 = self.viewport_height.into();
        let max = px((total - viewport).max(0.0));
        self.scroll_offset = (self.scroll_offset + amount).clamp(px(0.), max);
        cx.notify();
    }

    /// Adjust `scroll_offset` so the cursor is visible in the viewport.
    /// A small margin keeps the cursor from sitting flush against the edge.
    /// No-op if the viewport height hasn't been recorded yet.
    pub fn scroll_cursor_into_view(&mut self) {
        let viewport: f32 = self.viewport_height.into();
        if viewport <= 0.0 {
            return;
        }
        let cursor_line = self.display_map.line_for_offset(self.cursor);
        let (line_y, line_h) = self
            .last_line_layouts
            .iter()
            .find(|line| {
                self.cursor >= line.content_offset
                    && self.cursor <= line.content_offset + line.source_len
            })
            .map(|line| {
                let position = line.display_position(self.cursor - line.content_offset);
                (
                    f32::from(self.display_map.line_y(cursor_line) + position.y),
                    f32::from(line.row_height),
                )
            })
            .unwrap_or_else(|| {
                (
                    f32::from(self.display_map.line_y(cursor_line)),
                    f32::from(self.display_map.line_height(cursor_line)),
                )
            });
        let total: f32 = f32::from(self.display_map.total_height()) + 48.0;
        let scroll: f32 = self.scroll_offset.into();
        let new_scroll = scroll_cursor_into_view_math(scroll, viewport, total, line_y, line_h);
        self.scroll_offset = px(new_scroll);
    }
}

fn line_start_at(content: &str, offset: usize) -> usize {
    let offset = offset.min(content.len());
    content[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn line_end_including_newline(content: &str, line_start: usize) -> usize {
    content[line_start..]
        .find('\n')
        .map(|offset| line_start + offset + 1)
        .unwrap_or(content.len())
}

fn move_line_start(content: &str, start: usize, delta: isize) -> usize {
    let mut target = line_start_at(content, start);
    if delta >= 0 {
        for _ in 0..delta as usize {
            let next = line_end_including_newline(content, target);
            if next >= content.len() {
                break;
            }
            target = next;
        }
    } else {
        for _ in 0..delta.unsigned_abs() {
            if target == 0 {
                break;
            }
            target = line_start_at(content, target - 1);
        }
    }
    target
}

fn visual_line_range(content: &str, anchor: usize, target: usize) -> Range<usize> {
    let first = line_start_at(content, anchor.min(target));
    let last = line_start_at(content, anchor.max(target));
    first..line_end_including_newline(content, last)
}

/// Pure helper for `scroll_cursor_into_view` — exposed for unit tests.
///
/// - Input `scroll` is the current scroll offset.
/// - `viewport` is the visible area height.
/// - `total` is the total document height (including chrome padding).
/// - `line_y` / `line_h` describe the cursor line in document coordinates.
/// Returns the new scroll offset, with a 2-line margin and clamped to
/// `[0, total - viewport]`.
pub fn scroll_cursor_into_view_math(
    scroll: f32,
    viewport: f32,
    total: f32,
    line_y: f32,
    line_h: f32,
) -> f32 {
    let padding: f32 = 24.0; // matches `EditorElement` top padding.
    let margin = line_h * 2.0;
    let top = line_y - margin;
    let bottom = line_y + line_h + margin;
    let new_scroll = if top < scroll {
        top.max(0.0)
    } else if bottom > scroll + viewport - padding {
        bottom - viewport + padding
    } else {
        scroll
    };
    let max_scroll = (total - viewport).max(0.0);
    new_scroll.clamp(0.0, max_scroll)
}

#[cfg(test)]
mod scroll_tests {
    use super::{move_line_start, scroll_cursor_into_view_math, visual_line_range};

    // Helper: 100-line doc, 20px lines, 500px viewport, 24+24px padding.
    // TOTAL (2048) > VIEWPORT (500) so max_scroll > 0 and we can exercise clamping.
    const LINE_H: f32 = 20.0;
    const VIEWPORT: f32 = 500.0;
    const TOTAL: f32 = 100.0 * 20.0 + 48.0;
    const PADDING: f32 = 24.0;

    #[test]
    fn visual_line_ranges_include_complete_lines() {
        let content = "one\ntwo\nthree";
        assert_eq!(visual_line_range(content, 4, 4), 4..8);
        assert_eq!(visual_line_range(content, 4, 8), 4..13);
        assert_eq!(visual_line_range(content, 4, 0), 0..8);
    }

    #[test]
    fn visual_line_movement_clamps_and_honors_counts() {
        let content = "one\ntwo\nthree";
        assert_eq!(move_line_start(content, 0, 2), 8);
        assert_eq!(move_line_start(content, 8, -2), 0);
        assert_eq!(move_line_start(content, 0, -1), 0);
        assert_eq!(move_line_start(content, 8, 4), 8);
    }

    #[test]
    fn cursor_above_viewport_scrolls_up() {
        // Cursor at line 2 (y = 40), scroll is 300 (viewing lines ~15..40).
        let out = scroll_cursor_into_view_math(300.0, VIEWPORT, TOTAL, 40.0, LINE_H);
        assert!(out < 300.0, "expected scroll to move up, got {}", out);
        // Cursor should fit in view: line_y - 2*line_h must be >= out.
        assert!(40.0 - 2.0 * LINE_H >= out);
    }

    #[test]
    fn cursor_below_viewport_scrolls_down() {
        // Cursor near doc end at y=480 (viewport is 0..476 usable), scroll 0.
        // cursor bottom = 480 + 20 + 40 = 540, which is > 0 + 500 - 24 = 476.
        let out = scroll_cursor_into_view_math(0.0, VIEWPORT, TOTAL, 480.0, LINE_H);
        assert!(out > 0.0, "expected scroll to move down, got {}", out);
        // Cursor bottom must fit: line_y + line_h + 2*line_h must be <= out + viewport - padding.
        let bottom = 480.0 + LINE_H + 2.0 * LINE_H;
        assert!(bottom <= out + VIEWPORT - PADDING + 0.001);
    }

    #[test]
    fn cursor_already_visible_no_change() {
        // Viewport shows 0..500; cursor mid-way at y=200. No change.
        let out = scroll_cursor_into_view_math(0.0, VIEWPORT, TOTAL, 200.0, LINE_H);
        assert_eq!(out, 0.0);
    }

    #[test]
    fn overscroll_clamps_to_max() {
        // Cursor at end of doc, current scroll past max.
        let max = (TOTAL - VIEWPORT).max(0.0);
        let out = scroll_cursor_into_view_math(9999.0, VIEWPORT, TOTAL, TOTAL - LINE_H, LINE_H);
        assert!(out <= max + 0.001);
        assert!(out >= 0.0);
    }

    #[test]
    fn viewport_bigger_than_document_stays_at_zero() {
        let out = scroll_cursor_into_view_math(0.0, 10_000.0, 400.0, 100.0, LINE_H);
        assert_eq!(out, 0.0);
    }
}

impl crate::ui::Scrollable for EditorState {
    fn total_height(&self) -> f32 {
        // Match the chrome added by `EditorElement`: 24px top + 24px bottom padding.
        f32::from(self.display_map.total_height()) + 48.0
    }
    fn scroll_offset(&self) -> Pixels {
        self.scroll_offset
    }
    fn set_scroll_offset(&mut self, offset: Pixels) {
        self.scroll_offset = offset;
    }
    fn drag_state(&self) -> Option<crate::ui::DragState> {
        self.drag_state
    }
    fn set_drag_state(&mut self, drag: Option<crate::ui::DragState>) {
        self.drag_state = drag;
    }
}

pub enum EditorEvent {
    Changed,
    RequestSave,
    RequestQuit,
    RequestOpen(String),
    RequestVaultSwitch,
    RequestVaultOpen,
    RequestNoteSearch,
    RequestCommand,
    /// User clicked a [[wikilink]] — title is the link target.
    WikilinkClicked(String),
    /// User typed [[ — request autocomplete from the app via minibuffer.
    WikilinkAutocomplete,
}

impl EventEmitter<EditorEvent> for EditorState {}
