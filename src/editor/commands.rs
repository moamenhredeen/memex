/// All editor operations that can be triggered by keybindings, commands, or plugins.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    // Cursor movement
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveLineStart,
    MoveLineEnd,

    // Selection
    SelectLeft,
    SelectRight,

    // Editing
    InsertChar(char),
    InsertNewline,
    InsertTab,
    DeleteBackward,
    DeleteForward,

    // Clipboard (future)
    // Copy,
    // Cut,
    // Paste,

    // History
    Undo,
    Redo,

    // Table
    TableNextCell,
    TablePrevCell,
}
