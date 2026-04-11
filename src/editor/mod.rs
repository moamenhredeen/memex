mod blink;
pub mod commands;
mod element;
mod input;
pub mod keymap;
mod movement;
mod table;
pub mod undo;
mod view;

use std::ops::Range;

use gpui::*;
use ropey::Rope;

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
    pub mode: keymap::EditorMode,
    pub keymap: keymap::Keymap,
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
            mode: keymap::EditorMode::Insert,
            keymap: keymap::Keymap::new(),
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
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content_len());
        self.selected_range = offset..offset;
        self.cursor = offset;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    pub fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content_len());
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
                    self.move_to(self.next_grapheme(self.cursor_offset()), cx);
                } else {
                    self.move_to(self.selected_range.end, cx);
                }
            }
            MoveUp => {
                let content = self.content();
                let pos = self.cursor;
                let before = &content[..pos.min(content.len())];
                let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                if line_start == 0 {
                    self.move_to(0, cx);
                } else {
                    let prev_end = line_start - 1;
                    let prev_start =
                        content[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let prev_len = prev_end - prev_start;
                    self.move_to(prev_start + col.min(prev_len), cx);
                }
            }
            MoveDown => {
                let content = self.content();
                let pos = self.cursor;
                let before = &content[..pos.min(content.len())];
                let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                let after = &content[pos..];
                if let Some(nl) = after.find('\n') {
                    let next_start = pos + nl + 1;
                    let rest = &content[next_start..];
                    let next_len = rest.find('\n').unwrap_or(rest.len());
                    self.move_to(next_start + col.min(next_len), cx);
                } else {
                    self.move_to(content.len(), cx);
                }
            }
            MoveLineStart => {
                let content = self.content();
                let pos = self.cursor.min(content.len());
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.move_to(line_start, cx);
            }
            MoveLineEnd => {
                let content = self.content();
                let pos = self.cursor.min(content.len());
                let line_end = content[pos..]
                    .find('\n')
                    .map(|p| pos + p)
                    .unwrap_or(content.len());
                self.move_to(line_end, cx);
            }
            SelectLeft => {
                self.select_to(self.prev_grapheme(self.cursor_offset()), cx);
            }
            SelectRight => {
                self.select_to(self.next_grapheme(self.cursor_offset()), cx);
            }
            DeleteBackward => {
                if self.selected_range.is_empty() {
                    self.select_to(self.prev_grapheme(self.cursor_offset()), cx);
                }
                self.replace_text_in_range(None, "", window, cx);
            }
            DeleteForward => {
                if self.selected_range.is_empty() {
                    self.select_to(self.next_grapheme(self.cursor_offset()), cx);
                }
                self.replace_text_in_range(None, "", window, cx);
            }
            InsertNewline => {
                self.replace_text_in_range(None, "\n", window, cx);
            }
            InsertTab => {
                if !self.handle_table_tab(true, cx) {
                    self.replace_text_in_range(None, "    ", window, cx);
                }
            }
            InsertChar(ch) => {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                self.replace_text_in_range(None, s, window, cx);
            }
            Undo => self.undo(cx),
            Redo => self.redo(cx),
            TableNextCell => {
                self.handle_table_tab(true, cx);
            }
            TablePrevCell => {
                self.handle_table_tab(false, cx);
            }
        }
    }
}

pub enum EditorEvent {
    Changed,
}

impl EventEmitter<EditorEvent> for EditorState {}
