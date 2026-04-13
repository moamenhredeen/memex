mod blink;
pub mod commands;
mod display_map;
mod element;
mod input;
mod movement;
pub mod outline;
mod table;
pub mod undo;
mod view;

use std::ops::Range;

use gpui::*;
use ropey::Rope;

use crate::command::Command;
use crate::minibuffer::Candidate;
use crate::pane::ItemAction;

pub use blink::BlinkCursor;
pub use view::EditorView;

actions!(editor, [TabAction, ShiftTabAction]);

pub struct EditorState {
    pub buffer: Rope,
    pub cursor: usize,
    pub selected_range: Range<usize>,
    pub selection_reversed: bool,
    pub marked_range: Option<Range<usize>>,
    pub focus_handle: FocusHandle,
    pub blink_cursor: Entity<BlinkCursor>,
    pub scroll_offset: Pixels,
    pub last_line_layouts: Vec<LinePaintInfo>,
    pub last_bounds: Option<Bounds<Pixels>>,
    pub history: undo::UndoHistory,
    pub grammar: crate::keymap::VimGrammar,
    pub display_map: display_map::DisplayMap,
    pub plugins: crate::plugin::PluginEngine,
    pub outline: outline::OutlineState,
    /// Status message shown briefly after command execution
    pub status_message: Option<String>,
    /// When true, the next OS text input event is suppressed (keymap already handled the key).
    pub suppress_next_input: bool,
    /// Whether vim is enabled (mirrored from KeymapSystem for input handler)
    pub vim_enabled: bool,
    /// Whether insert mode is active (mirrored from KeymapSystem for input handler)
    pub insert_mode: bool,
    _blink_sub: Subscription,
}

#[derive(Clone)]
pub struct LinePaintInfo {
    pub content_offset: usize,
    pub shaped_line: ShapedLine,
    pub y: Pixels,
    pub line_height: Pixels,
}

impl EditorState {
    pub fn new(content: String, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let blink_cursor = cx.new(|_cx| BlinkCursor::new());
        let _blink_sub = cx.observe(&blink_cursor, |_, _, cx| cx.notify());

        let mut display = display_map::DisplayMap::new(px(24.));
        display.update(&content);

        let plugins = crate::plugin::PluginEngine::new();

        Self {
            cursor: 0,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            buffer: Rope::from_str(&content),
            focus_handle,
            blink_cursor,
            scroll_offset: px(0.),
            last_line_layouts: Vec::new(),
            last_bounds: None,
            history: undo::UndoHistory::new(),
            grammar: crate::keymap::VimGrammar::new(),
            display_map: display,
            plugins,
            outline: outline::OutlineState::new(),
            status_message: None,
            suppress_next_input: false,
            vim_enabled: true,
            insert_mode: false,
            _blink_sub,
        }
    }

    /// Snapshot the buffer as a String (allocates). Use for read-heavy operations
    /// that need string slicing. Mutations should use the rope API directly.
    pub fn content(&self) -> String {
        self.buffer.to_string()
    }

    pub fn content_len(&self) -> usize {
        self.buffer.len_bytes()
    }

    pub fn set_content(&mut self, content: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer = Rope::from_str(&content);
        self.cursor = 0;
        self.selected_range = 0..0;
        self.marked_range = None;
        self.history.clear();
        self.display_map.update(&content);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    /// Replace a byte range in the rope buffer with new text. O(log n).
    pub(crate) fn rope_replace(&mut self, range: Range<usize>, new_text: &str) {
        let char_start = self.buffer.byte_to_char(range.start);
        let char_end = self.buffer.byte_to_char(range.end);
        if char_start != char_end {
            self.buffer.remove(char_start..char_end);
        }
        if !new_text.is_empty() {
            self.buffer.insert(char_start, new_text);
        }
        self.display_map.invalidate();
    }

    /// Internal text mutation — bypasses OS input guard.
    /// Used by all commands that need to modify buffer content programmatically.
    pub(crate) fn edit_text(&mut self, new_text: &str, cx: &mut Context<Self>) {
        let range = self.marked_range.clone().unwrap_or(self.selected_range.clone());

        let old_text = {
            let char_start = self.buffer.byte_to_char(range.start);
            let char_end = self.buffer.byte_to_char(range.end);
            self.buffer.slice(char_start..char_end).to_string()
        };
        let cursor_before = self.cursor;
        let selection_before = self.selected_range.clone();

        self.rope_replace(range.clone(), new_text);

        let new_cursor = range.start + new_text.len();

        self.history.record(
            undo::EditOp {
                range: range.clone(),
                old_text,
                new_text: new_text.to_string(),
                cursor_before,
                cursor_after: new_cursor,
            },
            selection_before,
        );

        self.selected_range = new_cursor..new_cursor;
        self.cursor = new_cursor;
        self.marked_range.take();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to_inner(offset, false, cx);
    }

    /// Move cursor, preferring forward direction when landing on hidden line.
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
        let txn = match self.history.undo() {
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
        let txn = match self.history.redo() {
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
                if let Some(tl) = self.display_map.next_visible_line(current_line, false) {
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
                if let Some(tl) = self.display_map.next_visible_line(current_line, true) {
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
                    let prev_start =
                        content[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
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
                    let char_start = self.buffer.byte_to_char(self.selected_range.start);
                    let char_end = self.buffer.byte_to_char(self.selected_range.end);
                    let text = self.buffer.slice(char_start..char_end).to_string();
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
                self.edit_text("\n", cx);
            }
            InsertTab => {
                if !self.handle_table_tab(true, cx) {
                    self.edit_text("    ", cx);
                }
            }
            InsertChar(ch) => {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                self.edit_text(s, cx);
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
                        let spaces = current[actual..].chars().take_while(|c| *c == ' ').count().min(4);
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
                let toggled: String = selected.chars().map(|c| {
                    if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) }
                    else { c.to_uppercase().next().unwrap_or(c) }
                }).collect();
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
                        if let Some(hash_end) = line_text.find(' ') {
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
                        let next_idx = headings.iter().position(|h| h.line_idx == next.line_idx).unwrap();
                        let _my_section_end = outline::section_end_line(hi_idx, &headings, line_count);
                        let next_section_end = outline::section_end_line(next_idx, &headings, line_count);
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
        keymap: &mut crate::keymap::KeymapSystem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::keymap::GrammarResult;
        match result {
            GrammarResult::MoveCursor(offset) => {
                if keymap.is_visual_active() {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
            GrammarResult::InsertChar(ch) => {
                self.dispatch(commands::EditorCommand::InsertChar(ch), window, cx);
            }
            GrammarResult::DeleteRange { range, yanked, enter_insert } => {
                self.grammar.register_content = yanked;
                let target = range.start;
                self.selected_range = range;
                self.edit_text("", cx);
                self.move_to(target.min(self.content_len()), cx);
                if enter_insert {
                    keymap.stack.activate_layer("vim:insert");
                    self.insert_mode = true;
                    self.history.break_coalescing();
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
                self.execute_command_by_id(cmd_id, count, keymap, window, cx);
            }
            GrammarResult::Batch(results) => {
                for r in results {
                    self.execute_grammar_result(r, count, keymap, window, cx);
                }
            }
            GrammarResult::ActivateLayer(layer_id) => {
                // Layer was already activated by grammar; handle side effects
                match layer_id {
                    "vim:insert" => {
                        self.history.break_coalescing();
                    }
                    "vim:normal" => {
                        // Collapse selection when returning to normal mode
                        if !self.selected_range.is_empty() {
                            let pos = self.selected_range.start;
                            self.move_to(pos, cx);
                        }
                        self.history.break_coalescing();
                    }
                    "vim:visual" | "vim:visual-line" => {
                        // Start visual selection at cursor
                        if self.selected_range.is_empty() {
                            let pos = self.cursor;
                            self.selected_range = pos..self.next_grapheme(pos);
                            self.selection_reversed = false;
                        }
                    }
                    _ => {}
                }
                cx.notify();
                // Sync insert_mode flag for input handler
                self.insert_mode = keymap.is_insert_active();
            }
            GrammarResult::PushTransient(_) => {
                // Transient layer was pushed by grammar; nothing to do here
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
            GrammarResult::RunScript(name) => {
                let content = self.content();
                let cursor = self.cursor;
                let sel = (self.selected_range.start, self.selected_range.end);
                if let Some(cmds) = self.plugins.run_command(&name, &content, cursor, sel) {
                    for cmd in cmds {
                        match cmd {
                            commands::EditorCommand::EnterMode(mode_str) => {
                                let layer_id = match mode_str.as_str() {
                                    "insert" => "vim:insert",
                                    "normal" => "vim:normal",
                                    "visual" => "vim:visual",
                                    "visual-line" => "vim:visual-line",
                                    _ => "vim:insert",
                                };
                                keymap.stack.activate_layer(layer_id);
                                self.insert_mode = keymap.is_insert_active();
                                cx.notify();
                            }
                            commands::EditorCommand::ToggleVimMode => {
                                let enabled = !keymap.vim_enabled;
                                keymap.set_vim_enabled(enabled);
                                self.vim_enabled = keymap.vim_enabled;
                                self.insert_mode = keymap.is_insert_active();
                                self.history.break_coalescing();
                                cx.notify();
                            }
                            _ => self.dispatch(cmd, window, cx),
                        }
                    }
                }
            }
            GrammarResult::Pending | GrammarResult::Noop => {}
        }
    }

    /// Execute a named command (from keymap Command actions).
    pub fn execute_command_by_id(
        &mut self,
        cmd_id: &str,
        count: usize,
        keymap: &mut crate::keymap::KeymapSystem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use commands::EditorCommand;
        match cmd_id {
            // Basic editing
            "delete-backward" => self.dispatch(EditorCommand::DeleteBackward, window, cx),
            "delete-forward" => self.dispatch(EditorCommand::DeleteForward, window, cx),
            "insert-newline" => self.dispatch(EditorCommand::InsertNewline, window, cx),
            "insert-tab" => self.dispatch(EditorCommand::InsertTab, window, cx),
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
                let enabled = !keymap.vim_enabled;
                keymap.set_vim_enabled(enabled);
                self.vim_enabled = keymap.vim_enabled;
                self.insert_mode = keymap.is_insert_active();
                self.history.break_coalescing();
                cx.notify();
            }
            // Visual mode operations
            "delete-selection" => {
                if keymap.is_visual_active() {
                    let (s, e) = self.ordered_selection();
                    let content = self.content();
                    self.grammar.register_content = content[s..e].to_string();
                }
                self.dispatch(EditorCommand::DeleteSelection, window, cx);
                if keymap.is_visual_active() {
                    keymap.stack.activate_layer("vim:normal");
                    cx.notify();
                }
            }
            "yank-selection" => {
                self.dispatch(EditorCommand::YankSelection, window, cx);
                if keymap.is_visual_active() {
                    keymap.stack.activate_layer("vim:normal");
                    cx.notify();
                }
            }
            "change-selection" => {
                if keymap.is_visual_active() {
                    let (s, e) = self.ordered_selection();
                    let content = self.content();
                    self.grammar.register_content = content[s..e].to_string();
                }
                self.dispatch(EditorCommand::DeleteSelection, window, cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
                cx.notify();
            }
            "indent-selection" => self.dispatch(EditorCommand::IndentSelection, window, cx),
            "dedent-selection" => self.dispatch(EditorCommand::DedentSelection, window, cx),
            "toggle-case-selection" => {
                self.dispatch(EditorCommand::ToggleCaseSelection, window, cx);
                if keymap.is_visual_active() {
                    keymap.stack.activate_layer("vim:normal");
                    cx.notify();
                }
            }
            "uppercase-selection" => {
                self.dispatch(EditorCommand::UppercaseSelection, window, cx);
                if keymap.is_visual_active() {
                    keymap.stack.activate_layer("vim:normal");
                    cx.notify();
                }
            }
            "lowercase-selection" => {
                self.dispatch(EditorCommand::LowercaseSelection, window, cx);
                if keymap.is_visual_active() {
                    keymap.stack.activate_layer("vim:normal");
                    cx.notify();
                }
            }
            "join-selection" => {
                self.dispatch(EditorCommand::JoinSelection, window, cx);
                if keymap.is_visual_active() {
                    keymap.stack.activate_layer("vim:normal");
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
                        while p < content.len() && !content.is_char_boundary(p) { p += 1; }
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
                        while p > 0 && !content.is_char_boundary(p) { p -= 1; }
                        p
                    };
                    self.grammar.register_content = content[start..pos].to_string();
                    self.selected_range = start..pos;
                    self.edit_text("", cx);
                }
            }
            "append-after" => {
                // a — move right one char and enter insert
                let pos = self.next_grapheme(self.cursor);
                self.move_to(pos, cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
                cx.notify();
            }
            "insert-at-line-start" => {
                // I — move to first non-whitespace and enter insert
                let content = self.content();
                let target = crate::keymap::motion_first_non_whitespace(&content, self.cursor, 1);
                self.move_to(target, cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
                cx.notify();
            }
            "insert-at-line-end" => {
                // A — move to end of line and enter insert
                let content = self.content();
                let target = crate::keymap::motion_line_end(&content, self.cursor, 1);
                self.move_to(target, cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
                cx.notify();
            }
            "open-line-below" => {
                // o — open line below and enter insert
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_end = content[pos..].find('\n').map(|p| pos + p).unwrap_or(content.len());
                self.move_to(line_end, cx);
                self.edit_text("\n", cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
                cx.notify();
            }
            "open-line-above" => {
                // O — open line above and enter insert
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.move_to(line_start, cx);
                self.edit_text("\n", cx);
                // Move cursor up to the new empty line
                let new_pos = line_start;
                self.move_to(new_pos, cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
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
                        let line_end = content[pos..].find('\n').map(|p| pos + p + 1).unwrap_or(content.len());
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
                let line_end = content[pos..].find('\n').map(|p| pos + p).unwrap_or(content.len());
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
                let line_end = content[pos..].find('\n').map(|p| pos + p).unwrap_or(content.len());
                if pos < line_end {
                    self.grammar.register_content = content[pos..line_end].to_string();
                    self.selected_range = pos..line_end;
                    self.edit_text("", cx);
                }
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
                cx.notify();
            }
            "change-line" => {
                // S — change entire line
                let content = self.content();
                let pos = self.snap_to_char_boundary(self.cursor);
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let line_end = content[pos..].find('\n').map(|p| pos + p).unwrap_or(content.len());
                self.grammar.register_content = content[line_start..line_end].to_string();
                self.selected_range = line_start..line_end;
                self.edit_text("", cx);
                keymap.stack.activate_layer("vim:insert");
                self.history.break_coalescing();
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
                // . — repeat last change (not yet implemented, placeholder)
                // TODO: implement dot-repeat
            }
            "repeat-char-search" => {
                let content = self.content();
                let cursor = self.cursor;
                if let Some(target) = crate::keymap::repeat_char_search(&self.grammar, &content, cursor, count) {
                    if keymap.is_visual_active() {
                        self.select_to(target, cx);
                    } else {
                        self.move_to(target, cx);
                    }
                }
            }
            "repeat-char-search-reverse" => {
                let content = self.content();
                let cursor = self.cursor;
                if let Some(target) = crate::keymap::repeat_char_search_reverse(&self.grammar, &content, cursor, count) {
                    if keymap.is_visual_active() {
                        self.select_to(target, cx);
                    } else {
                        self.move_to(target, cx);
                    }
                }
            }
            // Scrolling
            "scroll-half-down" => {
                self.scroll_offset = (self.scroll_offset + px(200.)).max(px(0.));
                cx.notify();
            }
            "scroll-half-up" => {
                self.scroll_offset = (self.scroll_offset - px(200.)).max(px(0.));
                cx.notify();
            }
            // Go-to commands (from g prefix trie)
            "goto-doc-start" => {
                self.move_to(0, cx);
            }
            "goto-last-non-ws" => {
                let content = self.content();
                let target = crate::keymap::defaults::motion_line_end(&content, self.cursor, 1);
                self.move_to(target, cx);
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
            _ => {
                // Try plugin commands
                let content = self.content();
                let cursor = self.cursor;
                let sel = (self.selected_range.start, self.selected_range.end);
                if let Some(cmds) = self.plugins.run_command(cmd_id, &content, cursor, sel) {
                    for cmd in cmds {
                        self.dispatch(cmd, window, cx);
                    }
                }
            }
        }
    }

    /// Execute an ex-style command (`:w`, `:q`, `:set`, etc.).
    /// Returns a status message string.
    pub fn execute_ex_command(
        &mut self,
        input: &str,
        keymap: &mut crate::keymap::KeymapSystem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                self.status_message = Some(self.handle_set_command(args, keymap));
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
                // Try plugin commands
                let content = self.content();
                let cursor = self.cursor;
                let sel = (self.selected_range.start, self.selected_range.end);
                let result = self.plugins.run_command(cmd, &content, cursor, sel);
                match result {
                    Some(cmds) => {
                        for c in cmds {
                            self.dispatch(c, window, cx);
                        }
                    }
                    None => {
                        self.status_message =
                            Some(format!("E492: Not an editor command: {}", cmd));
                    }
                }
            }
        }
    }

    pub fn handle_set_command(&mut self, args: &str, keymap: &mut crate::keymap::KeymapSystem) -> String {
        if args.is_empty() {
            let mode_label = keymap.active_vim_state().unwrap_or("EDT");
            return format!(
                "vim={} mode={}",
                keymap.vim_enabled,
                mode_label
            );
        }

        match args {
            "vim" => {
                keymap.set_vim_enabled(true);
                "Vim mode enabled".into()
            }
            "novim" => {
                keymap.set_vim_enabled(false);
                "Vim mode disabled".into()
            }
            "number" | "nu" => "Line numbers not yet implemented".into(),
            "nonumber" | "nonu" => "Line numbers not yet implemented".into(),
            "wrap" => "Soft wrap not yet implemented".into(),
            "nowrap" => "Soft wrap not yet implemented".into(),
            _ => format!("Unknown option: {}", args),
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
    /// Editor commands are mostly handled by `execute_command_by_id` — this is the
    /// thin wrapper returning ItemActions.
    pub fn item_execute_command(
        &mut self,
        _cmd_id: &str,
        _viewport: (f32, f32),
        _cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        // Editor commands go through the existing execute_command_by_id path,
        // which is called directly from app.rs with the keymap reference.
        // This method exists for interface completeness.
        vec![]
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
}

impl EventEmitter<EditorEvent> for EditorState {}
