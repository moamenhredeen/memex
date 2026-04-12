use super::{Motion, Operator, VimAction};
use crate::editor::commands::EditorCommand;

/// Compute the byte range for a motion (used by operators).
pub(super) fn compute_motion_range(
    motion: &Motion,
    content: &str,
    cursor: usize,
    count: usize,
) -> (usize, usize) {
    match motion {
        Motion::Line => {
            // Entire line(s) from cursor
            let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let mut end = cursor;
            for _ in 0..count {
                if let Some(nl) = content[end..].find('\n') {
                    end = end + nl + 1;
                } else {
                    end = content.len();
                    break;
                }
            }
            (line_start, end)
        }
        _ => {
            let target = super::normal::compute_motion_target(motion, content, cursor, count);
            if target < cursor {
                (target, cursor)
            } else {
                (cursor, target)
            }
        }
    }
}

/// Apply an operator to a range.
pub(super) fn apply_operator(
    op: Operator,
    start: usize,
    end: usize,
    content: &str,
) -> VimAction {
    if start == end {
        return VimAction::None;
    }
    let yanked = content[start..end].to_string();

    match op {
        Operator::Delete => VimAction::OperatorResult {
            delete_range: start..end,
            yank_text: yanked,
            enter_insert: false,
        },
        Operator::Change => VimAction::OperatorResult {
            delete_range: start..end,
            yank_text: yanked,
            enter_insert: true,
        },
        Operator::Yank => {
            VimAction::Commands(vec![EditorCommand::YankText(yanked)])
        }
        Operator::Indent => {
            // Indent: insert 4 spaces at start of line
            let line_start = content[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            VimAction::Commands(vec![
                EditorCommand::MoveToOffset(line_start),
                EditorCommand::InsertText("    ".to_string()),
            ])
        }
        Operator::Dedent => {
            let line_start = content[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let spaces = content[line_start..].chars().take_while(|c| *c == ' ').count().min(4);
            if spaces > 0 {
                VimAction::OperatorResult {
                    delete_range: line_start..line_start + spaces,
                    yank_text: String::new(),
                    enter_insert: false,
                }
            } else {
                VimAction::None
            }
        }
    }
}
