mod normal;
mod operators;
mod visual;

use super::commands::EditorCommand;
use super::keymap::EditorMode;

/// Vim operator types.
#[derive(Clone, Debug, PartialEq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Dedent,
}

/// Vim motion types.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBackward,
    WordEnd,
    BigWordForward,   // W
    BigWordBackward,  // B
    BigWordEnd,       // E
    LineStart,        // 0
    FirstNonWhitespace, // ^
    LineEnd,          // $
    Line,             // dd, yy, cc — entire line
    DocStart,         // gg
    DocEnd,           // G
    GotoLine(usize),  // {n}G
    FindChar(char),        // f{char}
    TilChar(char),         // t{char}
    FindCharBackward(char),// F{char}
    TilCharBackward(char), // T{char}
    ParagraphForward,      // }
    ParagraphBackward,     // {
    MatchingBracket,       // %
    Indent,                // > (motion for operator)
    Dedent,                // < (motion for operator)
}

/// Per-editor vim state.
pub struct VimState {
    /// Whether vim mode is enabled.
    pub enabled: bool,
    /// Pending operator (after d, c, y — waiting for motion).
    pub pending_operator: Option<Operator>,
    /// Accumulated count prefix (e.g. the "3" in "3dw").
    pub count: Option<usize>,
    /// Named register for yank/paste.
    pub register: char,
    /// Yank register contents.
    pub register_content: String,
    /// Pending input state for f/t/r/g motions.
    pub waiting_for_char: Option<WaitingFor>,
    /// Last change for dot-repeat.
    pub last_change: Option<RecordedChange>,
    /// Last f/t/F/T search for ; and , repeat.
    pub last_char_search: Option<Motion>,
}

/// What we're waiting for after a key press.
#[derive(Clone, Debug)]
pub enum WaitingFor {
    FindChar,          // f
    TilChar,           // t
    FindCharBackward,  // F
    TilCharBackward,   // T
    Replace,           // r
    GPrefix,           // g (waiting for second key: gg, etc.)
}

/// A recorded change for dot-repeat.
#[derive(Clone, Debug)]
pub struct RecordedChange {
    pub commands: Vec<EditorCommand>,
}

impl VimState {
    pub fn new() -> Self {
        Self {
            enabled: false,
            pending_operator: None,
            count: None,
            register: '"',
            register_content: String::new(),
            waiting_for_char: None,
            last_change: None,
            last_char_search: None,
        }
    }

    /// Check if we're in operator-pending state.
    pub fn is_operator_pending(&self) -> bool {
        self.pending_operator.is_some()
    }

    /// Check if we're waiting for a character input (f, t, r).
    pub fn is_waiting(&self) -> bool {
        self.waiting_for_char.is_some()
    }

    /// Get the effective count (default 1).
    pub fn effective_count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    /// Push a digit to the count accumulator.
    pub fn push_count_digit(&mut self, digit: u8) {
        let current = self.count.unwrap_or(0);
        self.count = Some(current * 10 + digit as usize);
    }

    /// Clear all pending state.
    pub fn clear_pending(&mut self) {
        self.pending_operator = None;
        self.count = None;
        self.waiting_for_char = None;
    }

    /// Process a key in Normal mode. Returns an optional command and optional mode change.
    pub fn handle_normal_key(
        &mut self,
        key: &str,
        content: &str,
        cursor: usize,
    ) -> VimAction {
        // If waiting for a character (f, t, F, T, r, g)
        if let Some(waiting) = self.waiting_for_char.take() {
            if let Some(ch) = key.chars().next() {
                if key.chars().count() == 1 {
                    return match waiting {
                        WaitingFor::FindChar => {
                            let motion = Motion::FindChar(ch);
                            self.last_char_search = Some(motion.clone());
                            self.resolve_motion_or_operator(motion, content, cursor)
                        }
                        WaitingFor::TilChar => {
                            let motion = Motion::TilChar(ch);
                            self.last_char_search = Some(motion.clone());
                            self.resolve_motion_or_operator(motion, content, cursor)
                        }
                        WaitingFor::FindCharBackward => {
                            let motion = Motion::FindCharBackward(ch);
                            self.last_char_search = Some(motion.clone());
                            self.resolve_motion_or_operator(motion, content, cursor)
                        }
                        WaitingFor::TilCharBackward => {
                            let motion = Motion::TilCharBackward(ch);
                            self.last_char_search = Some(motion.clone());
                            self.resolve_motion_or_operator(motion, content, cursor)
                        }
                        WaitingFor::Replace => {
                            let count = self.effective_count();
                            self.clear_pending();
                            VimAction::ReplaceChar(ch, count)
                        }
                        WaitingFor::GPrefix => {
                            self.clear_pending();
                            match ch {
                                'g' => {
                                    // gg — go to document start
                                    VimAction::Command(EditorCommand::MoveToOffset(
                                        normal::compute_motion_target(&Motion::DocStart, content, cursor, 1)
                                    ))
                                }
                                'd' => {
                                    // gd — placeholder: go to definition (same as gg for now)
                                    VimAction::Command(EditorCommand::MoveToOffset(0))
                                }
                                _ => VimAction::None,
                            }
                        }
                    };
                }
            }
            self.clear_pending();
            return VimAction::None;
        }

        // Count digits (0 is special: only count if we already have digits)
        if let Some(digit) = key.chars().next() {
            if digit.is_ascii_digit() && (digit != '0' || self.count.is_some()) {
                self.push_count_digit(digit as u8 - b'0');
                return VimAction::None;
            }
        }

        normal::handle_normal_key(self, key, content, cursor)
    }

    /// Process a key in Visual mode.
    pub fn handle_visual_key(
        &mut self,
        key: &str,
        content: &str,
        cursor: usize,
    ) -> VimAction {
        visual::handle_visual_key(self, key, content, cursor)
    }

    /// Resolve a motion — either execute it directly or apply with pending operator.
    fn resolve_motion_or_operator(
        &mut self,
        motion: Motion,
        content: &str,
        cursor: usize,
    ) -> VimAction {
        let count = self.effective_count();

        if let Some(op) = self.pending_operator.take() {
            self.count = None;
            let (start, end) = operators::compute_motion_range(&motion, content, cursor, count);
            operators::apply_operator(op, start, end, content)
        } else {
            self.count = None;
            let target = normal::compute_motion_target(&motion, content, cursor, count);
            VimAction::Command(EditorCommand::MoveToOffset(target))
        }
    }
}

/// Result of processing a vim key.
#[derive(Debug)]
pub enum VimAction {
    /// Do nothing (key consumed, waiting for more input).
    None,
    /// Execute a single editor command.
    Command(EditorCommand),
    /// Execute multiple commands in sequence.
    Commands(Vec<EditorCommand>),
    /// Replace character(s) at cursor.
    ReplaceChar(char, usize),
    /// Replace text at range and advance cursor (for ~ toggle case).
    ReplaceAndAdvance(String, std::ops::Range<usize>),
    /// Change mode.
    ChangeMode(EditorMode),
    /// Switch to insert mode at a specific offset.
    InsertAt(usize),
    /// Delete a range and optionally enter insert mode.
    OperatorResult {
        delete_range: std::ops::Range<usize>,
        yank_text: String,
        enter_insert: bool,
    },
    /// Request the command palette (vim ":" key).
    RequestCommand,
}
