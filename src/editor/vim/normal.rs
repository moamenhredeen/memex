use super::{Motion, Operator, VimAction, VimState, WaitingFor};
use crate::editor::commands::EditorCommand;
use crate::editor::keymap::EditorMode;

/// Handle a key press in Normal mode.
pub(super) fn handle_normal_key(
    state: &mut VimState,
    key: &str,
    content: &str,
    cursor: usize,
) -> VimAction {
    match key {
        // === Mode transitions ===
        "i" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Insert)
        }
        "I" => {
            // Insert at first non-whitespace of line
            state.clear_pending();
            let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let first_non_ws = content[line_start..]
                .find(|c: char| !c.is_whitespace() || c == '\n')
                .map(|i| line_start + i)
                .unwrap_or(line_start);
            VimAction::InsertAt(first_non_ws)
        }
        "a" => {
            state.clear_pending();
            let target = if cursor < content.len() && content.as_bytes().get(cursor) != Some(&b'\n') {
                next_char_boundary(content, cursor)
            } else {
                cursor
            };
            VimAction::InsertAt(target)
        }
        "A" => {
            // Append at end of line
            state.clear_pending();
            let line_end = content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len());
            VimAction::InsertAt(line_end)
        }
        "o" => {
            state.clear_pending();
            let line_end = content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len());
            VimAction::Commands(vec![
                EditorCommand::MoveToOffset(line_end),
                EditorCommand::InsertNewline,
            ])
        }
        "O" => {
            state.clear_pending();
            let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            VimAction::Commands(vec![
                EditorCommand::MoveToOffset(line_start),
                EditorCommand::InsertNewline,
                EditorCommand::MoveUp,
            ])
        }
        "s" => {
            // Substitute: delete char under cursor, enter insert mode
            state.clear_pending();
            let count = state.effective_count();
            let mut end = cursor;
            for _ in 0..count {
                if end < content.len() && content.as_bytes().get(end) != Some(&b'\n') {
                    end = next_char_boundary(content, end);
                }
            }
            if end > cursor {
                super::operators::apply_operator(Operator::Change, cursor, end, content)
            } else {
                VimAction::ChangeMode(EditorMode::Insert)
            }
        }
        "S" => {
            // Substitute entire line (like cc)
            state.clear_pending();
            let (start, end) = super::operators::compute_motion_range(
                &Motion::Line, content, cursor, 1,
            );
            super::operators::apply_operator(Operator::Change, start, end, content)
        }
        "C" => {
            // Change to end of line
            state.clear_pending();
            let line_end = content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len());
            if line_end > cursor {
                super::operators::apply_operator(Operator::Change, cursor, line_end, content)
            } else {
                VimAction::ChangeMode(EditorMode::Insert)
            }
        }
        "D" => {
            // Delete to end of line
            state.clear_pending();
            let line_end = content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len());
            if line_end > cursor {
                super::operators::apply_operator(Operator::Delete, cursor, line_end, content)
            } else {
                VimAction::None
            }
        }
        "Y" => {
            // Yank entire line (like yy)
            state.clear_pending();
            let (start, end) = super::operators::compute_motion_range(
                &Motion::Line, content, cursor, 1,
            );
            super::operators::apply_operator(Operator::Yank, start, end, content)
        }
        "J" => {
            // Join current line with next line
            state.clear_pending();
            let line_end = content[cursor..].find('\n').map(|p| cursor + p);
            if let Some(nl_pos) = line_end {
                let abs_nl = nl_pos;
                // Remove newline and leading whitespace of next line, replace with single space
                let next_line_start = abs_nl + 1;
                let trimmed = content[next_line_start..]
                    .find(|c: char| !c.is_ascii_whitespace() || c == '\n')
                    .map(|i| next_line_start + i)
                    .unwrap_or(next_line_start);
                VimAction::OperatorResult {
                    delete_range: abs_nl..trimmed,
                    yank_text: String::new(),
                    enter_insert: false,
                }
            } else {
                VimAction::None
            }
        }

        // === Visual mode ===
        "v" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Visual)
        }
        "V" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::VisualLine)
        }

        // === Operators ===
        "d" => {
            if state.pending_operator == Some(Operator::Delete) {
                state.clear_pending();
                let count = state.effective_count();
                let (start, end) = super::operators::compute_motion_range(
                    &Motion::Line, content, cursor, count,
                );
                super::operators::apply_operator(Operator::Delete, start, end, content)
            } else {
                state.pending_operator = Some(Operator::Delete);
                VimAction::None
            }
        }
        "c" => {
            if state.pending_operator == Some(Operator::Change) {
                state.clear_pending();
                let (start, end) = super::operators::compute_motion_range(
                    &Motion::Line, content, cursor, 1,
                );
                super::operators::apply_operator(Operator::Change, start, end, content)
            } else {
                state.pending_operator = Some(Operator::Change);
                VimAction::None
            }
        }
        "y" => {
            if state.pending_operator == Some(Operator::Yank) {
                state.clear_pending();
                let (start, end) = super::operators::compute_motion_range(
                    &Motion::Line, content, cursor, 1,
                );
                super::operators::apply_operator(Operator::Yank, start, end, content)
            } else {
                state.pending_operator = Some(Operator::Yank);
                VimAction::None
            }
        }
        ">" => {
            if state.pending_operator == Some(Operator::Indent) {
                // >> — indent current line
                state.clear_pending();
                let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                VimAction::Commands(vec![
                    EditorCommand::MoveToOffset(line_start),
                    EditorCommand::InsertText("    ".to_string()),
                ])
            } else {
                state.pending_operator = Some(Operator::Indent);
                VimAction::None
            }
        }
        "<" => {
            if state.pending_operator == Some(Operator::Dedent) {
                // << — dedent current line
                state.clear_pending();
                let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let line_text = &content[line_start..];
                let spaces = line_text.chars().take_while(|c| *c == ' ').count().min(4);
                if spaces > 0 {
                    VimAction::OperatorResult {
                        delete_range: line_start..line_start + spaces,
                        yank_text: String::new(),
                        enter_insert: false,
                    }
                } else {
                    VimAction::None
                }
            } else {
                state.pending_operator = Some(Operator::Dedent);
                VimAction::None
            }
        }

        // === Paste ===
        "p" => {
            state.clear_pending();
            let text = state.register_content.clone();
            if text.is_empty() {
                return VimAction::None;
            }
            if text.ends_with('\n') {
                // Linewise paste below
                let line_end = content[cursor..].find('\n').map(|p| cursor + p + 1).unwrap_or(content.len());
                VimAction::Commands(vec![
                    EditorCommand::MoveToOffset(line_end),
                    EditorCommand::InsertText(text),
                ])
            } else {
                let target = if cursor < content.len() {
                    next_char_boundary(content, cursor)
                } else {
                    cursor
                };
                VimAction::Commands(vec![
                    EditorCommand::MoveToOffset(target),
                    EditorCommand::InsertText(text),
                ])
            }
        }
        "P" => {
            // Paste before cursor
            state.clear_pending();
            let text = state.register_content.clone();
            if text.is_empty() {
                return VimAction::None;
            }
            if text.ends_with('\n') {
                // Linewise paste above
                let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                VimAction::Commands(vec![
                    EditorCommand::MoveToOffset(line_start),
                    EditorCommand::InsertText(text),
                ])
            } else {
                VimAction::Commands(vec![
                    EditorCommand::InsertText(text),
                ])
            }
        }

        // === Single char operations ===
        "x" => {
            state.clear_pending();
            let count = state.effective_count();
            let mut end = cursor;
            for _ in 0..count {
                if end < content.len() && content.as_bytes().get(end) != Some(&b'\n') {
                    end = next_char_boundary(content, end);
                }
            }
            if end > cursor {
                super::operators::apply_operator(Operator::Delete, cursor, end, content)
            } else {
                VimAction::None
            }
        }
        "X" => {
            // Delete char before cursor (like backspace)
            state.clear_pending();
            let count = state.effective_count();
            let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let mut start = cursor;
            for _ in 0..count {
                if start > line_start {
                    start = prev_char_boundary(content, start);
                }
            }
            if start < cursor {
                super::operators::apply_operator(Operator::Delete, start, cursor, content)
            } else {
                VimAction::None
            }
        }
        "~" => {
            // Toggle case of char under cursor
            state.clear_pending();
            if cursor < content.len() {
                let ch = content[cursor..].chars().next().unwrap();
                if ch == '\n' {
                    return VimAction::None;
                }
                let toggled: String = if ch.is_uppercase() {
                    ch.to_lowercase().collect()
                } else {
                    ch.to_uppercase().collect()
                };
                let end = next_char_boundary(content, cursor);
                VimAction::ReplaceAndAdvance(toggled, cursor..end)
            } else {
                VimAction::None
            }
        }

        // === Undo/Redo ===
        "u" => {
            state.clear_pending();
            VimAction::Command(EditorCommand::Undo)
        }
        "." => {
            // Dot repeat — replay last change
            state.clear_pending();
            if let Some(ref change) = state.last_change {
                VimAction::Commands(change.commands.clone())
            } else {
                VimAction::None
            }
        }

        // === Search char motions ===
        "f" => {
            state.waiting_for_char = Some(WaitingFor::FindChar);
            VimAction::None
        }
        "F" => {
            state.waiting_for_char = Some(WaitingFor::FindCharBackward);
            VimAction::None
        }
        "t" => {
            state.waiting_for_char = Some(WaitingFor::TilChar);
            VimAction::None
        }
        "T" => {
            state.waiting_for_char = Some(WaitingFor::TilCharBackward);
            VimAction::None
        }
        ";" => {
            // Repeat last f/t/F/T
            if let Some(ref repeat) = state.last_char_search {
                let motion = repeat.clone();
                return state.resolve_motion_or_operator(motion, content, cursor);
            }
            VimAction::None
        }
        "," => {
            // Repeat last f/t/F/T in reverse
            if let Some(ref repeat) = state.last_char_search {
                let reversed = match repeat {
                    Motion::FindChar(c) => Motion::FindCharBackward(*c),
                    Motion::FindCharBackward(c) => Motion::FindChar(*c),
                    Motion::TilChar(c) => Motion::TilCharBackward(*c),
                    Motion::TilCharBackward(c) => Motion::TilChar(*c),
                    other => other.clone(),
                };
                return state.resolve_motion_or_operator(reversed, content, cursor);
            }
            VimAction::None
        }

        // === Replace ===
        "r" => {
            state.waiting_for_char = Some(WaitingFor::Replace);
            VimAction::None
        }

        // === Motions ===
        "h" => resolve_motion(state, Motion::Left, content, cursor),
        "l" => resolve_motion(state, Motion::Right, content, cursor),
        "j" => resolve_motion(state, Motion::Down, content, cursor),
        "k" => resolve_motion(state, Motion::Up, content, cursor),
        "w" => resolve_motion(state, Motion::WordForward, content, cursor),
        "W" => resolve_motion(state, Motion::BigWordForward, content, cursor),
        "b" => resolve_motion(state, Motion::WordBackward, content, cursor),
        "B" => resolve_motion(state, Motion::BigWordBackward, content, cursor),
        "e" => resolve_motion(state, Motion::WordEnd, content, cursor),
        "E" => resolve_motion(state, Motion::BigWordEnd, content, cursor),
        "0" => resolve_motion(state, Motion::LineStart, content, cursor),
        "^" => resolve_motion(state, Motion::FirstNonWhitespace, content, cursor),
        "$" => resolve_motion(state, Motion::LineEnd, content, cursor),
        "_" => resolve_motion(state, Motion::FirstNonWhitespace, content, cursor),
        "G" => {
            let count = state.count;
            state.clear_pending();
            if let Some(n) = count {
                // {n}G — go to line n
                resolve_motion(state, Motion::GotoLine(n), content, cursor)
            } else {
                resolve_motion(state, Motion::DocEnd, content, cursor)
            }
        }
        "g" => {
            state.waiting_for_char = Some(WaitingFor::GPrefix);
            VimAction::None
        }
        "{" => resolve_motion(state, Motion::ParagraphBackward, content, cursor),
        "}" => resolve_motion(state, Motion::ParagraphForward, content, cursor),
        "%" => resolve_motion(state, Motion::MatchingBracket, content, cursor),

        // === Escape clears pending ===
        "escape" => {
            state.clear_pending();
            VimAction::None
        }

        // === Command mode ===
        ":" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Command)
        }

        _ => {
            state.clear_pending();
            VimAction::None
        }
    }
}

fn resolve_motion(
    state: &mut VimState,
    motion: Motion,
    content: &str,
    cursor: usize,
) -> VimAction {
    state.resolve_motion_or_operator(motion, content, cursor)
}

/// Compute the target cursor position for a motion.
pub(super) fn compute_motion_target(
    motion: &Motion,
    content: &str,
    cursor: usize,
    count: usize,
) -> usize {
    match motion {
        Motion::Left => {
            let mut pos = cursor;
            let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
            for _ in 0..count {
                if pos <= line_start { break; }
                pos = prev_char_boundary(content, pos);
            }
            pos
        }
        Motion::Right => {
            let mut pos = cursor;
            for _ in 0..count {
                if pos >= content.len() { break; }
                let next = next_char_boundary(content, pos);
                // Don't move past newline
                if content.as_bytes().get(pos) == Some(&b'\n') { break; }
                pos = next;
            }
            pos
        }
        Motion::Up => {
            let mut pos = cursor;
            for _ in 0..count {
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                if line_start == 0 {
                    pos = 0;
                    break;
                }
                let prev_end = line_start - 1;
                let prev_start = content[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let prev_len = prev_end - prev_start;
                pos = prev_start + col.min(prev_len);
            }
            pos
        }
        Motion::Down => {
            let mut pos = cursor;
            for _ in 0..count {
                let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = pos - line_start;
                let after = &content[pos..];
                if let Some(nl) = after.find('\n') {
                    let next_start = pos + nl + 1;
                    let rest = &content[next_start..];
                    let next_len = rest.find('\n').unwrap_or(rest.len());
                    pos = next_start + col.min(next_len);
                } else {
                    pos = content.len();
                    break;
                }
            }
            pos
        }
        Motion::WordForward => {
            let mut pos = cursor;
            for _ in 0..count {
                pos = next_word_start(content, pos);
            }
            pos
        }
        Motion::WordBackward => {
            let mut pos = cursor;
            for _ in 0..count {
                pos = prev_word_start(content, pos);
            }
            pos
        }
        Motion::WordEnd => {
            let mut pos = cursor;
            for _ in 0..count {
                pos = next_word_end(content, pos);
            }
            pos
        }
        Motion::BigWordForward => {
            let mut pos = cursor;
            for _ in 0..count {
                pos = next_big_word_start(content, pos);
            }
            pos
        }
        Motion::BigWordBackward => {
            let mut pos = cursor;
            for _ in 0..count {
                pos = prev_big_word_start(content, pos);
            }
            pos
        }
        Motion::BigWordEnd => {
            let mut pos = cursor;
            for _ in 0..count {
                pos = next_big_word_end(content, pos);
            }
            pos
        }
        Motion::LineStart => {
            content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
        }
        Motion::FirstNonWhitespace => {
            let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            content[line_start..]
                .find(|c: char| !c.is_whitespace() || c == '\n')
                .map(|i| line_start + i)
                .unwrap_or(line_start)
        }
        Motion::LineEnd => {
            content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len())
        }
        Motion::DocStart => 0,
        Motion::DocEnd => content.len(),
        Motion::GotoLine(n) => {
            let target_line = (*n).max(1) - 1;
            let mut pos = 0;
            for _ in 0..target_line {
                if let Some(nl) = content[pos..].find('\n') {
                    pos = pos + nl + 1;
                } else {
                    return content.len();
                }
            }
            pos
        }
        Motion::Line => cursor,
        Motion::FindChar(ch) => {
            let after = &content[cursor..];
            let skip = next_char_boundary_in(after, 0);
            if let Some(pos) = after[skip..].find(*ch) {
                cursor + skip + pos
            } else {
                cursor
            }
        }
        Motion::TilChar(ch) => {
            let after = &content[cursor..];
            let skip = next_char_boundary_in(after, 0);
            if let Some(pos) = after[skip..].find(*ch) {
                let target = cursor + skip + pos;
                if target > cursor { prev_char_boundary(content, target) } else { cursor }
            } else {
                cursor
            }
        }
        Motion::FindCharBackward(ch) => {
            let before = &content[..cursor];
            if let Some(pos) = before.rfind(*ch) {
                pos
            } else {
                cursor
            }
        }
        Motion::TilCharBackward(ch) => {
            let before = &content[..cursor];
            if let Some(pos) = before.rfind(*ch) {
                next_char_boundary(content, pos)
            } else {
                cursor
            }
        }
        Motion::ParagraphForward => {
            let bytes = content.as_bytes();
            let len = bytes.len();
            let mut pos = cursor;
            for _ in 0..count {
                // Skip current non-empty lines
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                // Skip blank lines
                while pos < len && bytes[pos] == b'\n' {
                    pos += 1;
                }
                // Find next blank line
                while pos < len {
                    if bytes[pos] == b'\n' {
                        break;
                    }
                    pos += 1;
                }
            }
            pos.min(len)
        }
        Motion::ParagraphBackward => {
            let bytes = content.as_bytes();
            let mut pos = cursor;
            for _ in 0..count {
                if pos > 0 { pos -= 1; }
                // Skip current blank lines
                while pos > 0 && bytes[pos] == b'\n' {
                    pos -= 1;
                }
                // Skip non-empty lines backwards
                while pos > 0 && bytes[pos] != b'\n' {
                    pos -= 1;
                }
                // Find previous blank line
                while pos > 0 && bytes[pos - 1] != b'\n' {
                    pos -= 1;
                }
            }
            pos
        }
        Motion::MatchingBracket => {
            let bytes = content.as_bytes();
            if cursor >= bytes.len() { return cursor; }
            let ch = bytes[cursor];
            let (open, close, forward) = match ch {
                b'(' => (b'(', b')', true),
                b')' => (b'(', b')', false),
                b'[' => (b'[', b']', true),
                b']' => (b'[', b']', false),
                b'{' => (b'{', b'}', true),
                b'}' => (b'{', b'}', false),
                _ => return cursor,
            };
            let mut depth: i32 = 0;
            if forward {
                for i in cursor..bytes.len() {
                    if bytes[i] == open { depth += 1; }
                    if bytes[i] == close { depth -= 1; }
                    if depth == 0 { return i; }
                }
            } else {
                for i in (0..=cursor).rev() {
                    if bytes[i] == close { depth += 1; }
                    if bytes[i] == open { depth -= 1; }
                    if depth == 0 { return i; }
                }
            }
            cursor
        }
        // Indent/Dedent are handled as operators, not motions
        Motion::Indent | Motion::Dedent => cursor,
    }
}

// Utility functions for char boundary navigation
fn next_char_boundary(s: &str, offset: usize) -> usize {
    let mut p = offset + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p.min(s.len())
}

fn prev_char_boundary(s: &str, offset: usize) -> usize {
    if offset == 0 { return 0; }
    let mut p = offset - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn next_char_boundary_in(s: &str, offset: usize) -> usize {
    if offset >= s.len() { return s.len(); }
    let mut p = offset + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p.min(s.len())
}

fn next_word_start(content: &str, cursor: usize) -> usize {
    let bytes = content.as_bytes();
    let len = content.len();
    if cursor >= len { return len; }

    let mut pos = cursor;
    // Skip current word chars
    while pos < len && is_word_char(bytes[pos]) {
        pos += 1;
    }
    // Skip non-word, non-newline chars
    while pos < len && !is_word_char(bytes[pos]) && bytes[pos] != b'\n' {
        pos += 1;
    }
    // If we hit a newline, move past it
    if pos < len && bytes[pos] == b'\n' {
        pos += 1;
    }
    pos.min(len)
}

fn prev_word_start(content: &str, cursor: usize) -> usize {
    if cursor == 0 { return 0; }
    let bytes = content.as_bytes();
    let mut pos = cursor;

    if pos > 0 { pos -= 1; }
    while pos > 0 && !is_word_char(bytes[pos]) {
        pos -= 1;
    }
    while pos > 0 && is_word_char(bytes[pos - 1]) {
        pos -= 1;
    }
    pos
}

fn next_word_end(content: &str, cursor: usize) -> usize {
    let bytes = content.as_bytes();
    let len = content.len();
    if cursor >= len { return len; }

    let mut pos = cursor;
    if pos < len { pos += 1; }
    while pos < len && !is_word_char(bytes[pos]) {
        pos += 1;
    }
    while pos < len && is_word_char(bytes[pos]) {
        pos += 1;
    }
    if pos > 0 { pos -= 1; }
    pos.max(cursor)
}

// WORD motions (space-delimited, ignoring punctuation)
fn next_big_word_start(content: &str, cursor: usize) -> usize {
    let bytes = content.as_bytes();
    let len = content.len();
    if cursor >= len { return len; }
    let mut pos = cursor;
    // Skip non-whitespace
    while pos < len && !bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    // Skip whitespace
    while pos < len && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos.min(len)
}

fn prev_big_word_start(content: &str, cursor: usize) -> usize {
    if cursor == 0 { return 0; }
    let bytes = content.as_bytes();
    let mut pos = cursor;
    if pos > 0 { pos -= 1; }
    while pos > 0 && bytes[pos].is_ascii_whitespace() {
        pos -= 1;
    }
    while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    pos
}

fn next_big_word_end(content: &str, cursor: usize) -> usize {
    let bytes = content.as_bytes();
    let len = content.len();
    if cursor >= len { return len; }
    let mut pos = cursor;
    if pos < len { pos += 1; }
    while pos < len && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    while pos < len && !bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos > 0 { pos -= 1; }
    pos.max(cursor)
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
