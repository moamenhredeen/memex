mod blink;
pub mod commands;
mod display_map;
mod element;
mod input;
pub mod keymap;
mod movement;
mod table;
pub mod undo;
mod view;
pub mod vim;

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
    pub vim: vim::VimState,
    pub display_map: display_map::DisplayMap,
    pub plugins: crate::plugin::PluginEngine,
    /// Vim command-line input (the text after `:`)
    pub command_line: String,
    /// Status message shown briefly after command execution
    pub status_message: Option<String>,
    /// Suppress the next OS text input (set after vim consumes a key that
    /// changes mode, so the OS input method doesn't also insert the char).
    pub suppress_next_input: bool,
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

        let mut plugins = crate::plugin::PluginEngine::new();
        plugins.load_all_plugins(None);

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
            vim: vim::VimState::new(),
            display_map: display,
            plugins,
            command_line: String::new(),
            status_message: None,
            suppress_next_input: false,
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

    /// Return the selection range in (start, end) order.
    fn ordered_selection(&self) -> (usize, usize) {
        let s = self.selected_range.start;
        let e = self.selected_range.end;
        if s <= e { (s, e) } else { (e, s) }
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
                let pos = self.cursor_offset();
                let before = &content[..pos.min(content.len())];
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
                let pos = self.cursor_offset();
                let before = &content[..pos.min(content.len())];
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
                    self.replace_text_in_range(None, "", window, cx);
                }
            }
            YankSelection => {
                if !self.selected_range.is_empty() {
                    let char_start = self.buffer.byte_to_char(self.selected_range.start);
                    let char_end = self.buffer.byte_to_char(self.selected_range.end);
                    let text = self.buffer.slice(char_start..char_end).to_string();
                    self.vim.register_content = text;
                    // Collapse selection
                    let pos = self.selected_range.start;
                    self.move_to(pos, cx);
                }
            }
            YankText(text) => {
                self.vim.register_content = text;
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
            DeleteRange(range) => {
                self.selected_range = range;
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
            InsertText(text) => {
                self.replace_text_in_range(None, &text, window, cx);
            }
            Undo => self.undo(cx),
            Redo => self.redo(cx),
            TableNextCell => {
                self.handle_table_tab(true, cx);
            }
            TablePrevCell => {
                self.handle_table_tab(false, cx);
            }
            EnterMode(new_mode) => {
                self.mode = new_mode;
                cx.notify();
            }
            ToggleVimMode => {
                self.vim.enabled = !self.vim.enabled;
                if self.vim.enabled {
                    self.mode = keymap::EditorMode::Normal;
                } else {
                    self.mode = keymap::EditorMode::Insert;
                }
                self.history.break_coalescing();
                cx.notify();
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
                        self.replace_text_in_range(None, "    ", window, cx);
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
                            self.replace_text_in_range(None, "", window, cx);
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
                self.replace_text_in_range(None, &joined, window, cx);
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
                self.replace_text_in_range(None, &toggled, window, cx);
            }
            UppercaseSelection => {
                let (start, end) = self.ordered_selection();
                let content = self.content();
                let upper = content[start..end].to_uppercase();
                self.selected_range = start..end;
                self.replace_text_in_range(None, &upper, window, cx);
            }
            LowercaseSelection => {
                let (start, end) = self.ordered_selection();
                let content = self.content();
                let lower = content[start..end].to_lowercase();
                self.selected_range = start..end;
                self.replace_text_in_range(None, &lower, window, cx);
            }
        }
    }

    /// Handle vim key processing. Called from view.rs for Normal/Visual modes.
    pub fn handle_vim_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = self.content();
        let cursor = self.cursor;

        let action = match self.mode {
            keymap::EditorMode::Normal => self.vim.handle_normal_key(key, &content, cursor),
            keymap::EditorMode::Visual | keymap::EditorMode::VisualLine => {
                self.vim.handle_visual_key(key, &content, cursor)
            }
            _ => return,
        };

        self.apply_vim_action(action, window, cx);
    }

    fn apply_vim_action(
        &mut self,
        action: vim::VimAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use vim::VimAction;
        match action {
            VimAction::None => {}
            VimAction::Command(cmd) => {
                self.dispatch(cmd, window, cx);
            }
            VimAction::Commands(cmds) => {
                for cmd in cmds {
                    self.dispatch(cmd, window, cx);
                }
            }
            VimAction::ChangeMode(new_mode) => {
                self.mode = new_mode;
                if new_mode == keymap::EditorMode::Insert {
                    // Break undo coalescing when entering insert mode
                    self.history.break_coalescing();
                }
                cx.notify();
            }
            VimAction::InsertAt(offset) => {
                self.move_to(offset, cx);
                self.mode = keymap::EditorMode::Insert;
                self.history.break_coalescing();
                cx.notify();
            }
            VimAction::ReplaceChar(ch, count) => {
                let content = self.content();
                let pos = self.cursor;
                let mut end = pos;
                for _ in 0..count {
                    if end < content.len() {
                        let mut p = end + 1;
                        while p < content.len() && !content.is_char_boundary(p) {
                            p += 1;
                        }
                        end = p;
                    }
                }
                if end > pos {
                    let replacement: String = std::iter::repeat(ch).take(count).collect();
                    self.selected_range = pos..end;
                    self.replace_text_in_range(None, &replacement, window, cx);
                }
            }
            VimAction::ReplaceAndAdvance(replacement, range) => {
                let next_pos = range.end.min(self.content().len());
                self.selected_range = range;
                self.replace_text_in_range(None, &replacement, window, cx);
                let new_len = self.content().len();
                self.move_to(next_pos.min(new_len), cx);
            }
            VimAction::OperatorResult {
                delete_range,
                yank_text,
                enter_insert,
            } => {
                self.vim.register_content = yank_text;
                let target = delete_range.start;
                self.selected_range = delete_range;
                self.replace_text_in_range(None, "", window, cx);
                // After J (join), keep cursor at join point
                self.move_to(target.min(self.content().len()), cx);
                if enter_insert {
                    self.mode = keymap::EditorMode::Insert;
                    self.history.break_coalescing();
                    cx.notify();
                }
            }
        }
    }

    /// Handle key input while in Command mode (`:` bar).
    pub fn handle_command_key(
        &mut self,
        key: &str,
        ctrl: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match key {
            "escape" => {
                self.command_line.clear();
                self.mode = keymap::EditorMode::Normal;
                cx.notify();
            }
            "enter" => {
                let cmd = self.command_line.clone();
                self.command_line.clear();
                self.mode = keymap::EditorMode::Normal;
                self.execute_ex_command(&cmd, window, cx);
                cx.notify();
            }
            "backspace" => {
                if self.command_line.is_empty() {
                    self.mode = keymap::EditorMode::Normal;
                } else {
                    self.command_line.pop();
                }
                cx.notify();
            }
            _ if ctrl => {
                // Ctrl+U clears the command line (like bash/vim)
                if key == "u" {
                    self.command_line.clear();
                    cx.notify();
                }
            }
            _ => {
                // Append printable characters
                if key.len() == 1 {
                    self.command_line.push_str(key);
                    cx.notify();
                }
            }
        }
    }

    /// Execute an ex-style command (`:w`, `:q`, `:set`, etc.).
    /// Returns a status message string.
    fn execute_ex_command(
        &mut self,
        input: &str,
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
                self.status_message = Some(self.handle_set_command(args));
            }
            "noh" | "nohlsearch" => {
                self.status_message = Some("Search highlighting cleared".into());
            }
            "e" | "edit" => {
                if args.is_empty() {
                    // Reload current file
                    self.status_message = Some("Specify a file path".into());
                } else {
                    cx.emit(EditorEvent::RequestOpen(args.to_string()));
                }
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

    fn handle_set_command(&mut self, args: &str) -> String {
        if args.is_empty() {
            return format!(
                "vim={} mode={}",
                self.vim.enabled,
                self.mode.label()
            );
        }

        match args {
            "vim" => {
                self.vim.enabled = true;
                self.mode = keymap::EditorMode::Normal;
                "Vim mode enabled".into()
            }
            "novim" => {
                self.vim.enabled = false;
                self.mode = keymap::EditorMode::Insert;
                "Vim mode disabled".into()
            }
            "number" | "nu" => "Line numbers not yet implemented".into(),
            "nonumber" | "nonu" => "Line numbers not yet implemented".into(),
            "wrap" => "Soft wrap not yet implemented".into(),
            "nowrap" => "Soft wrap not yet implemented".into(),
            _ => format!("Unknown option: {}", args),
        }
    }
}

pub enum EditorEvent {
    Changed,
    RequestSave,
    RequestQuit,
    RequestOpen(String),
}

impl EventEmitter<EditorEvent> for EditorState {}
