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
        // Mode transitions
        "i" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Insert)
        }
        "a" => {
            state.clear_pending();
            let target = if cursor < content.len() {
                next_char_boundary(content, cursor)
            } else {
                cursor
            };
            VimAction::InsertAt(target)
        }
        "o" => {
            state.clear_pending();
            // Move to end of line, insert newline, enter insert mode
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

        // Visual mode
        "v" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Visual)
        }
        "V" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::VisualLine)
        }

        // Operators
        "d" => {
            if state.pending_operator == Some(Operator::Delete) {
                // dd — delete line
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
                // cc — change line
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
                // yy — yank line
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

        // Paste
        "p" => {
            state.clear_pending();
            let text = state.register_content.clone();
            if text.is_empty() {
                return VimAction::None;
            }
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

        // Single char operations
        "x" => {
            state.clear_pending();
            let count = state.effective_count();
            let mut end = cursor;
            for _ in 0..count {
                if end < content.len() {
                    end = next_char_boundary(content, end);
                }
            }
            if end > cursor {
                super::operators::apply_operator(Operator::Delete, cursor, end, content)
            } else {
                VimAction::None
            }
        }

        // Undo/Redo
        "u" => {
            state.clear_pending();
            VimAction::Command(EditorCommand::Undo)
        }

        // f/t char search
        "f" => {
            state.waiting_for_char = Some(WaitingFor::FindChar);
            VimAction::None
        }
        "t" => {
            state.waiting_for_char = Some(WaitingFor::TilChar);
            VimAction::None
        }

        // Replace
        "r" => {
            state.waiting_for_char = Some(WaitingFor::Replace);
            VimAction::None
        }

        // Motions
        "h" => resolve_motion(state, Motion::Left, content, cursor),
        "l" => resolve_motion(state, Motion::Right, content, cursor),
        "j" => resolve_motion(state, Motion::Down, content, cursor),
        "k" => resolve_motion(state, Motion::Up, content, cursor),
        "w" => resolve_motion(state, Motion::WordForward, content, cursor),
        "b" => resolve_motion(state, Motion::WordBackward, content, cursor),
        "e" => resolve_motion(state, Motion::WordEnd, content, cursor),
        "0" => resolve_motion(state, Motion::LineStart, content, cursor),
        "$" => resolve_motion(state, Motion::LineEnd, content, cursor),
        "G" => resolve_motion(state, Motion::DocEnd, content, cursor),
        "g" => {
            // gg — go to document start (simplified: just g acts as gg)
            resolve_motion(state, Motion::DocStart, content, cursor)
        }

        // Escape clears pending
        "escape" => {
            state.clear_pending();
            VimAction::None
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
            for _ in 0..count {
                if pos == 0 { break; }
                pos = prev_char_boundary(content, pos);
            }
            pos
        }
        Motion::Right => {
            let mut pos = cursor;
            for _ in 0..count {
                if pos >= content.len() { break; }
                pos = next_char_boundary(content, pos);
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
        Motion::LineStart => {
            content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
        }
        Motion::LineEnd => {
            content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len())
        }
        Motion::DocStart => 0,
        Motion::DocEnd => content.len(),
        Motion::Line => cursor, // handled specially by operators
        Motion::FindChar(ch) => {
            let after = &content[cursor..];
            // Skip current char, find next occurrence
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

    // Move back one
    if pos > 0 { pos -= 1; }
    // Skip whitespace/non-word backwards
    while pos > 0 && !is_word_char(bytes[pos]) {
        pos -= 1;
    }
    // Skip word chars backwards
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
    // Move forward one
    if pos < len { pos += 1; }
    // Skip non-word chars
    while pos < len && !is_word_char(bytes[pos]) {
        pos += 1;
    }
    // Skip word chars to end
    while pos < len && is_word_char(bytes[pos]) {
        pos += 1;
    }
    if pos > 0 { pos -= 1; }
    pos.max(cursor)
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
