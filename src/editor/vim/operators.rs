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
            // Don't delete, just yank
            VimAction::Commands(vec![EditorCommand::YankText(yanked)])
        }
    }
}
