use super::{Motion, VimAction, VimState};
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
        "c" | "s" => {
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::DeleteSelection,
                EditorCommand::EnterMode(EditorMode::Insert),
            ])
        }
        "S" => {
            // Same as c in visual
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
        ">" => {
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::IndentSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }
        "<" => {
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::DedentSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }
        "J" => {
            // Join selected lines
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::JoinSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }
        "~" => {
            // Toggle case of selection
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::ToggleCaseSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }
        "u" => {
            // Lowercase selection
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::LowercaseSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }
        "U" => {
            // Uppercase selection
            state.clear_pending();
            VimAction::Commands(vec![
                EditorCommand::UppercaseSelection,
                EditorCommand::EnterMode(EditorMode::Normal),
            ])
        }

        // Mode switch
        "v" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Normal)
        }
        "V" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::VisualLine)
        }

        // Motion keys extend selection
        "h" => VimAction::Command(EditorCommand::SelectLeft),
        "l" => VimAction::Command(EditorCommand::SelectRight),
        "j" => VimAction::Command(EditorCommand::SelectDown),
        "k" => VimAction::Command(EditorCommand::SelectUp),
        "w" | "W" => {
            let motion = if key == "W" { Motion::BigWordForward } else { Motion::WordForward };
            let target = super::normal::compute_motion_target(&motion, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "b" | "B" => {
            let motion = if key == "B" { Motion::BigWordBackward } else { Motion::WordBackward };
            let target = super::normal::compute_motion_target(&motion, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "e" | "E" => {
            let motion = if key == "E" { Motion::BigWordEnd } else { Motion::WordEnd };
            let target = super::normal::compute_motion_target(&motion, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "0" => {
            let target = super::normal::compute_motion_target(&Motion::LineStart, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "^" | "_" => {
            let target = super::normal::compute_motion_target(&Motion::FirstNonWhitespace, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "$" => {
            let target = super::normal::compute_motion_target(&Motion::LineEnd, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "G" => {
            let target = super::normal::compute_motion_target(&Motion::DocEnd, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "g" => {
            let target = super::normal::compute_motion_target(&Motion::DocStart, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "{" => {
            let target = super::normal::compute_motion_target(&Motion::ParagraphBackward, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "}" => {
            let target = super::normal::compute_motion_target(&Motion::ParagraphForward, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }
        "%" => {
            let target = super::normal::compute_motion_target(&Motion::MatchingBracket, content, cursor, 1);
            VimAction::Command(EditorCommand::SelectToOffset(target))
        }

        // Escape already handled above
        ":" => {
            state.clear_pending();
            VimAction::ChangeMode(EditorMode::Command)
        }

        _ => VimAction::None,
    }
}
