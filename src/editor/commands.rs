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
    MoveToOffset(usize),

    // Selection
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectToOffset(usize),
    SelectAll,
    DeleteSelection,
    YankSelection,

    // Editing
    InsertChar(char),
    InsertNewline,
    InsertTab,
    InsertText(String),
    DeleteBackward,
    DeleteForward,
    DeleteRange(std::ops::Range<usize>),

    // Clipboard / Yank
    YankText(String),

    // History
    Undo,
    Redo,

    // Table
    TableNextCell,
    TablePrevCell,

    // Mode
    EnterMode(String),
    ToggleVimMode,

    // Visual mode operations
    IndentSelection,
    DedentSelection,
    JoinSelection,
    ToggleCaseSelection,
    UppercaseSelection,
    LowercaseSelection,

    // Outline
    OutlineCycleFold,
    OutlineGlobalCycle,
    OutlinePromote,
    OutlineDemote,
    OutlineMoveUp,
    OutlineMoveDown,
    OutlineNextHeading,
    OutlinePrevHeading,
}
