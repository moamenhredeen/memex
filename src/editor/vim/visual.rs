use super::{Motion, Operator, VimAction, VimState};
use crate::editor::commands::EditorCommand;
use crate::editor::keymap::EditorMode;

/// Handle a key press in Visual mode.
pub(super) fn handle_visual_key(
    state: &mut VimState,
    key: &str,
    content: &str,
    cursor: usize,
) -> VimAction {
    match key {
        // Escape → back to normal
        "escape" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Normal)
        }

        // Operators on selection
        "d" | "x" => {
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::DeleteSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }
        "c" => {
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::DeleteSelection,
                EditorCommand::EnterMode(EditorMode::Insert),
            ])
        }
        "y" => {
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::YankSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }

        // Motion keys extend selection
        "h" => VimAction::Command(EditorCommand::SelectLeft),
        "l" => VimAction::Command(EditorCommand::SelectRight),
        "j" => VimAction::Command(EditorCommand::SelectDown),
        "k" => VimAction::Command(EditorCommand::SelectUp),
        "w" => {
            let target = super::normal::compute_motion_target(
                &Motion::WordForward, content, cursor, 1,
            );
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "b" => {
            let target = super::normal::compute_motion_target(
                &Motion::WordBackward, content, cursor, 1,
            );
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "0" => {
            let target = super::normal::compute_motion_target(
                &Motion::LineStart, content, cursor, 1,
            );
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "$" => {
            let target = super::normal::compute_motion_target(
                &Motion::LineEnd, content, cursor, 1,
            );
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }

        _ => VimAction::None,
    }
}
