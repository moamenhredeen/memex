//! Unified minibuffer — emacs-style always-visible command/completion interface.
//!
//! The minibuffer is the single point of entry for all interactive prompts:
//! command palette (M-x / vim :), note search (Ctrl+P), vault switching, etc.
//!
//! Architecture follows Zed's Picker/PickerDelegate pattern:
//! - `Minibuffer` struct owns the input state (text, cursor, vim mode, selection).
//! - `MinibufferDelegate` trait is implemented by each completion provider.
//! - The app layer wires them together: routes keys through the minibuffer,
//!   calls the delegate for candidates, and performs side effects on confirm.

/// A candidate displayed in the vertico-style completion list.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Primary display text (command name, note title, vault name).
    pub label: String,
    /// Optional secondary text (description, path, keybinding).
    pub detail: Option<String>,
    /// Whether this is a special action (e.g., "Create new note").
    pub is_action: bool,
    /// Data payload — command id, file path, etc.
    pub data: String,
}

/// What the delegate wants the app to do after a confirm/complete.
#[derive(Debug)]
pub enum MinibufferResult {
    /// Close the minibuffer.
    Dismiss,
    /// Close and execute the command with given id and raw input.
    Execute { command_id: String, raw_input: String },
    /// Close and open a file/note at this path.
    OpenPath(String),
    /// Close and create a new note with this title.
    CreateNote(String),
    /// Close current delegate and switch to a new one (e.g., vault -> notes).
    Chain(DelegateKind),
    /// Show a message in the minibuffer echo area.
    Message(String),
}

/// Identifies which delegate is active. Used to dispatch to the right
/// candidate/confirm logic in the app layer.
///
/// This is an enum rather than trait objects because we have a known set of
/// delegates and want to avoid Box<dyn> complexity. Each variant maps to
/// concrete candidate-generation and confirmation logic in app.rs.
///
/// To add a new delegate: add a variant here, implement candidate generation
/// and confirm handling in app.rs's match arms.
#[derive(Clone, Debug, PartialEq)]
pub enum DelegateKind {
    /// Command palette (M-x, vim :) — fuzzy-searches CommandRegistry.
    Command,
    /// Note search (Ctrl+P, :notes) — fuzzy-searches vault notes.
    NoteSearch,
    /// Recent vaults picker (:vault-switch) — MRU-ordered registered vaults.
    VaultSwitch,
    /// Directory browser (:vault-open) — navigate filesystem to choose a vault.
    VaultOpen,
    /// PDF table of contents browser — fuzzy-searches TOC entries.
    PdfToc,
    /// PDF go-to-page — type a page number and jump to it.
    PdfGotoPage,
}

/// Editing mode within the minibuffer input line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MinibufferVimMode {
    /// Standard text input (always used when vim is disabled).
    Insert,
    /// Vim normal mode — motions, editing commands, j/k for candidates.
    Normal,
}

/// Actions returned by `Minibuffer::handle_key()`.
///
/// The minibuffer is pure state — it never performs side effects itself.
/// Instead it returns these actions, and the app layer interprets them.
#[derive(Debug, PartialEq)]
pub enum MinibufferAction {
    /// Input text or cursor position changed — re-render and re-filter candidates.
    Updated,
    /// User confirmed the current selection (Enter).
    Confirm,
    /// User requested tab-completion (Tab) — insert selected candidate text.
    Complete,
    /// User wants to dismiss the minibuffer (Escape, Ctrl+G, empty backspace).
    Dismiss,
}

/// Unified minibuffer state.
///
/// Owns the single-line text input with full cursor-aware editing and
/// optional vim motions. Delegates candidate generation and confirmation
/// to the app layer based on `delegate_kind`.
pub struct Minibuffer {
    /// Whether the minibuffer is currently active (accepting input).
    pub active: bool,
    /// Which delegate is providing candidates.
    pub delegate_kind: DelegateKind,
    /// Prompt text shown before the input (e.g., ":", "M-x", "Find note:").
    pub prompt: String,
    /// The input text.
    pub input: String,
    /// Byte offset of the cursor within `input`.
    pub cursor: usize,
    /// Index of the currently selected candidate in the completion list.
    pub selected: usize,
    /// Whether vim keybindings are enabled.
    pub vim_enabled: bool,
    /// Vim editing mode within the minibuffer.
    pub vim_mode: MinibufferVimMode,
    /// Message shown when the minibuffer is idle (echo area).
    pub message: Option<String>,
}

impl Minibuffer {
    pub fn new() -> Self {
        Self {
            active: false,
            delegate_kind: DelegateKind::Command,
            prompt: String::new(),
            input: String::new(),
            cursor: 0,
            selected: 0,
            vim_enabled: false,
            vim_mode: MinibufferVimMode::Insert,
            message: None,
        }
    }

    /// Activate the minibuffer with a given delegate and prompt.
    pub fn activate(&mut self, kind: DelegateKind, prompt: &str, vim_enabled: bool) {
        self.active = true;
        self.delegate_kind = kind;
        self.prompt = prompt.to_string();
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
        self.vim_enabled = vim_enabled;
        // Minibuffer always starts in insert mode (like vim cmdline).
        self.vim_mode = MinibufferVimMode::Insert;
    }

    /// Dismiss the minibuffer and reset state.
    pub fn dismiss(&mut self) {
        self.active = false;
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
    }

    /// Set the echo area message (shown when idle).
    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
    }

    /// Clear the echo area message.
    pub fn clear_message(&mut self) {
        self.message = None;
    }

    /// Handle a key press. Returns a `MinibufferAction` telling the app what happened.
    ///
    /// `candidate_count` is the number of currently visible candidates (needed
    /// for bounds-checking selection navigation).
    pub fn handle_key(
        &mut self,
        key: &str,
        ctrl: bool,
        _shift: bool,
        candidate_count: usize,
    ) -> MinibufferAction {
        if key == "escape" {
            if self.vim_enabled && self.vim_mode == MinibufferVimMode::Insert {
                // Escape in insert mode → switch to normal mode
                self.vim_mode = MinibufferVimMode::Normal;
                return MinibufferAction::Updated;
            }
            // Escape in normal mode (or vim disabled) → dismiss
            return MinibufferAction::Dismiss;
        }

        match self.vim_mode {
            MinibufferVimMode::Normal => self.handle_vim_normal(key, ctrl, candidate_count),
            MinibufferVimMode::Insert => self.handle_insert(key, ctrl, candidate_count),
        }
    }

    /// Get text before and after cursor for rendering.
    pub fn input_parts(&self) -> (&str, &str) {
        let pos = self.cursor.min(self.input.len());
        (&self.input[..pos], &self.input[pos..])
    }

    // ── Insert mode ──────────────────────────────────────────────────

    fn handle_insert(
        &mut self,
        key: &str,
        ctrl: bool,
        candidate_count: usize,
    ) -> MinibufferAction {
        if ctrl {
            return self.handle_ctrl(key, candidate_count);
        }

        match key {
            "enter" => MinibufferAction::Confirm,
            "tab" => MinibufferAction::Complete,
            "backspace" => {
                if self.cursor > 0 {
                    let prev = self.prev_char_boundary();
                    self.input.drain(prev..self.cursor);
                    self.cursor = prev;
                    self.selected = 0;
                    MinibufferAction::Updated
                } else if self.input.is_empty() {
                    MinibufferAction::Dismiss
                } else {
                    MinibufferAction::Updated
                }
            }
            "delete" => {
                if self.cursor < self.input.len() {
                    let next = self.next_char_boundary();
                    self.input.drain(self.cursor..next);
                    self.selected = 0;
                }
                MinibufferAction::Updated
            }
            "left" => {
                if self.cursor > 0 {
                    self.cursor = self.prev_char_boundary();
                }
                MinibufferAction::Updated
            }
            "right" => {
                if self.cursor < self.input.len() {
                    self.cursor = self.next_char_boundary();
                }
                MinibufferAction::Updated
            }
            "up" => {
                self.move_selection_up();
                MinibufferAction::Updated
            }
            "down" => {
                self.move_selection_down(candidate_count);
                MinibufferAction::Updated
            }
            "home" => {
                self.cursor = 0;
                MinibufferAction::Updated
            }
            "end" => {
                self.cursor = self.input.len();
                MinibufferAction::Updated
            }
            _ => {
                if key.len() == 1 {
                    self.input.insert_str(self.cursor, key);
                    self.cursor += key.len();
                    self.selected = 0;
                }
                MinibufferAction::Updated
            }
        }
    }

    fn handle_ctrl(&mut self, key: &str, candidate_count: usize) -> MinibufferAction {
        match key {
            "u" => {
                self.input.drain(..self.cursor);
                self.cursor = 0;
                self.selected = 0;
                MinibufferAction::Updated
            }
            "k" => {
                self.input.truncate(self.cursor);
                MinibufferAction::Updated
            }
            "a" => {
                self.cursor = 0;
                MinibufferAction::Updated
            }
            "e" => {
                self.cursor = self.input.len();
                MinibufferAction::Updated
            }
            "w" => {
                let new_pos = self.prev_word_boundary();
                self.input.drain(new_pos..self.cursor);
                self.cursor = new_pos;
                self.selected = 0;
                MinibufferAction::Updated
            }
            "n" => {
                self.move_selection_down(candidate_count);
                MinibufferAction::Updated
            }
            "p" => {
                self.move_selection_up();
                MinibufferAction::Updated
            }
            "g" => MinibufferAction::Dismiss,
            _ => MinibufferAction::Updated,
        }
    }

    // ── Vim normal mode ──────────────────────────────────────────────

    fn handle_vim_normal(
        &mut self,
        key: &str,
        ctrl: bool,
        candidate_count: usize,
    ) -> MinibufferAction {
        if ctrl {
            return match key {
                "n" => {
                    self.move_selection_down(candidate_count);
                    MinibufferAction::Updated
                }
                "p" => {
                    self.move_selection_up();
                    MinibufferAction::Updated
                }
                _ => MinibufferAction::Updated,
            };
        }

        match key {
            // -- Mode switches --
            "i" => {
                self.vim_mode = MinibufferVimMode::Insert;
                MinibufferAction::Updated
            }
            "a" => {
                self.vim_mode = MinibufferVimMode::Insert;
                if self.cursor < self.input.len() {
                    self.cursor = self.next_char_boundary();
                }
                MinibufferAction::Updated
            }
            "I" => {
                self.vim_mode = MinibufferVimMode::Insert;
                self.cursor = 0;
                MinibufferAction::Updated
            }
            "A" => {
                self.vim_mode = MinibufferVimMode::Insert;
                self.cursor = self.input.len();
                MinibufferAction::Updated
            }

            // -- Motions --
            "h" | "left" => {
                if self.cursor > 0 {
                    self.cursor = self.prev_char_boundary();
                }
                MinibufferAction::Updated
            }
            "l" | "right" => {
                if self.cursor < self.input.len() {
                    self.cursor = self.next_char_boundary();
                }
                MinibufferAction::Updated
            }
            "0" | "home" => {
                self.cursor = 0;
                MinibufferAction::Updated
            }
            "$" | "end" => {
                self.cursor = self.input.len();
                MinibufferAction::Updated
            }
            "^" | "_" => {
                self.cursor = self
                    .input
                    .find(|c: char| !c.is_whitespace())
                    .unwrap_or(0);
                MinibufferAction::Updated
            }
            "w" => {
                self.cursor = self.next_word_boundary();
                MinibufferAction::Updated
            }
            "b" => {
                self.cursor = self.prev_word_boundary();
                MinibufferAction::Updated
            }
            "e" => {
                self.cursor = self.word_end_boundary();
                MinibufferAction::Updated
            }

            // -- Editing --
            "x" => {
                if self.cursor < self.input.len() {
                    let next = self.next_char_boundary();
                    self.input.drain(self.cursor..next);
                    self.selected = 0;
                }
                MinibufferAction::Updated
            }
            "X" => {
                if self.cursor > 0 {
                    let prev = self.prev_char_boundary();
                    self.input.drain(prev..self.cursor);
                    self.cursor = prev;
                    self.selected = 0;
                }
                MinibufferAction::Updated
            }
            "D" => {
                self.input.truncate(self.cursor);
                MinibufferAction::Updated
            }
            "C" => {
                self.input.truncate(self.cursor);
                self.vim_mode = MinibufferVimMode::Insert;
                MinibufferAction::Updated
            }
            "S" => {
                self.input.clear();
                self.cursor = 0;
                self.vim_mode = MinibufferVimMode::Insert;
                MinibufferAction::Updated
            }
            "d" => {
                // dd equivalent — clear entire input
                self.input.clear();
                self.cursor = 0;
                self.selected = 0;
                MinibufferAction::Updated
            }

            // -- Candidate navigation --
            "j" | "down" => {
                self.move_selection_down(candidate_count);
                MinibufferAction::Updated
            }
            "k" | "up" => {
                self.move_selection_up();
                MinibufferAction::Updated
            }

            // -- Confirm/complete --
            "enter" => MinibufferAction::Confirm,
            "tab" => MinibufferAction::Complete,

            _ => MinibufferAction::Updated,
        }
    }

    // ── Text navigation helpers ──────────────────────────────────────

    fn prev_char_boundary(&self) -> usize {
        if self.cursor == 0 {
            return 0;
        }
        let mut p = self.cursor - 1;
        while p > 0 && !self.input.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn next_char_boundary(&self) -> usize {
        if self.cursor >= self.input.len() {
            return self.input.len();
        }
        let mut p = self.cursor + 1;
        while p < self.input.len() && !self.input.is_char_boundary(p) {
            p += 1;
        }
        p.min(self.input.len())
    }

    fn prev_word_boundary(&self) -> usize {
        if self.cursor == 0 {
            return 0;
        }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor;
        if pos > 0 {
            pos -= 1;
        }
        // Skip non-word chars
        while pos > 0 && !is_word_byte(bytes[pos]) {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && is_word_byte(bytes[pos - 1]) {
            pos -= 1;
        }
        pos
    }

    fn next_word_boundary(&self) -> usize {
        let bytes = self.input.as_bytes();
        let len = bytes.len();
        if self.cursor >= len {
            return len;
        }
        let mut pos = self.cursor;
        // Skip current word chars
        while pos < len && is_word_byte(bytes[pos]) {
            pos += 1;
        }
        // Skip non-word chars
        while pos < len && !is_word_byte(bytes[pos]) {
            pos += 1;
        }
        pos.min(len)
    }

    fn word_end_boundary(&self) -> usize {
        let bytes = self.input.as_bytes();
        let len = bytes.len();
        if self.cursor >= len {
            return len;
        }
        let mut pos = self.cursor;
        if pos < len {
            pos += 1;
        }
        // Skip non-word chars
        while pos < len && !is_word_byte(bytes[pos]) {
            pos += 1;
        }
        // Skip word chars
        while pos < len && is_word_byte(bytes[pos]) {
            pos += 1;
        }
        if pos > 0 {
            pos -= 1;
        }
        pos.max(self.cursor)
    }

    fn move_selection_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_selection_down(&mut self, max: usize) {
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activate_dismiss() {
        let mut mb = Minibuffer::new();
        assert!(!mb.active);

        mb.activate(DelegateKind::Command, "M-x", false);
        assert!(mb.active);
        assert_eq!(mb.delegate_kind, DelegateKind::Command);
        assert_eq!(mb.prompt, "M-x");
        assert!(mb.input.is_empty());
        assert_eq!(mb.cursor, 0);
        assert_eq!(mb.selected, 0);

        mb.dismiss();
        assert!(!mb.active);
    }

    #[test]
    fn test_typing_and_cursor() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);

        for ch in "hello".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        assert_eq!(mb.input, "hello");
        assert_eq!(mb.cursor, 5);

        // Move left twice
        mb.handle_key("left", false, false, 0);
        mb.handle_key("left", false, false, 0);
        assert_eq!(mb.cursor, 3);

        // Insert at cursor
        mb.handle_key("X", false, false, 0);
        assert_eq!(mb.input, "helXlo");
        assert_eq!(mb.cursor, 4);
    }

    #[test]
    fn test_backspace() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::NoteSearch, "Find:", false);

        for ch in "ab".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        mb.handle_key("backspace", false, false, 0);
        assert_eq!(mb.input, "a");
        assert_eq!(mb.cursor, 1);

        mb.handle_key("backspace", false, false, 0);
        assert_eq!(mb.input, "");

        // Empty backspace should dismiss
        let action = mb.handle_key("backspace", false, false, 0);
        assert_eq!(action, MinibufferAction::Dismiss);
    }

    #[test]
    fn test_ctrl_u_kills_backward() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);
        for ch in "hello world".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        // Move left 5 chars to cursor at "world"
        for _ in 0..5 {
            mb.handle_key("left", false, false, 0);
        }
        assert_eq!(mb.cursor, 6);

        mb.handle_key("u", true, false, 0); // Ctrl+U
        assert_eq!(mb.input, "world");
        assert_eq!(mb.cursor, 0);
    }

    #[test]
    fn test_ctrl_w_kills_word() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);
        for ch in "hello world".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        mb.handle_key("w", true, false, 0); // Ctrl+W
        assert_eq!(mb.input, "hello ");
        assert_eq!(mb.cursor, 6);
    }

    #[test]
    fn test_ctrl_a_e() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);
        for ch in "test".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        mb.handle_key("a", true, false, 0); // Ctrl+A
        assert_eq!(mb.cursor, 0);
        mb.handle_key("e", true, false, 0); // Ctrl+E
        assert_eq!(mb.cursor, 4);
    }

    #[test]
    fn test_selection_navigation() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::NoteSearch, "Find:", false);
        assert_eq!(mb.selected, 0);

        mb.handle_key("down", false, false, 5);
        assert_eq!(mb.selected, 1);
        mb.handle_key("down", false, false, 5);
        assert_eq!(mb.selected, 2);
        mb.handle_key("up", false, false, 5);
        assert_eq!(mb.selected, 1);

        // Ctrl+N/P
        mb.handle_key("n", true, false, 5);
        assert_eq!(mb.selected, 2);
        mb.handle_key("p", true, false, 5);
        assert_eq!(mb.selected, 1);

        // Bounds
        mb.selected = 4;
        mb.handle_key("down", false, false, 5);
        assert_eq!(mb.selected, 4);
        mb.selected = 0;
        mb.handle_key("up", false, false, 5);
        assert_eq!(mb.selected, 0);
    }

    #[test]
    fn test_confirm_and_escape() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);

        assert_eq!(
            mb.handle_key("enter", false, false, 0),
            MinibufferAction::Confirm
        );

        let mut mb2 = Minibuffer::new();
        mb2.activate(DelegateKind::Command, ":", false);
        assert_eq!(
            mb2.handle_key("escape", false, false, 0),
            MinibufferAction::Dismiss
        );
    }

    #[test]
    fn test_tab_completion() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);
        assert_eq!(
            mb.handle_key("tab", false, false, 5),
            MinibufferAction::Complete
        );
    }

    #[test]
    fn test_vim_normal_motions() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", true);

        for ch in "hello world".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        assert_eq!(mb.cursor, 11);

        // Enter vim normal mode manually
        mb.vim_mode = MinibufferVimMode::Normal;

        // h moves left
        mb.handle_key("h", false, false, 0);
        assert_eq!(mb.cursor, 10);

        // 0 goes to start
        mb.handle_key("0", false, false, 0);
        assert_eq!(mb.cursor, 0);

        // $ goes to end
        mb.handle_key("$", false, false, 0);
        assert_eq!(mb.cursor, 11);

        // w moves to next word
        mb.handle_key("0", false, false, 0);
        mb.handle_key("w", false, false, 0);
        assert_eq!(mb.cursor, 6); // "world"

        // b moves back
        mb.handle_key("$", false, false, 0);
        mb.handle_key("b", false, false, 0);
        assert_eq!(mb.cursor, 6);
    }

    #[test]
    fn test_vim_normal_editing() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", true);
        for ch in "hello world".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        mb.vim_mode = MinibufferVimMode::Normal;

        // x deletes char at cursor
        mb.handle_key("0", false, false, 0);
        mb.handle_key("x", false, false, 0);
        assert_eq!(mb.input, "ello world");

        // D truncates from cursor
        mb.handle_key("w", false, false, 0);
        mb.handle_key("D", false, false, 0);
        assert_eq!(mb.input, "ello ");

        // S clears and enters insert
        mb.handle_key("S", false, false, 0);
        assert_eq!(mb.input, "");
        assert_eq!(mb.vim_mode, MinibufferVimMode::Insert);
    }

    #[test]
    fn test_vim_normal_candidate_nav() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", true);
        mb.vim_mode = MinibufferVimMode::Normal;

        mb.handle_key("j", false, false, 5);
        assert_eq!(mb.selected, 1);
        mb.handle_key("k", false, false, 5);
        assert_eq!(mb.selected, 0);
    }

    #[test]
    fn test_vim_mode_switches() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", true);
        mb.vim_mode = MinibufferVimMode::Normal;

        // i switches to insert
        mb.handle_key("i", false, false, 0);
        assert_eq!(mb.vim_mode, MinibufferVimMode::Insert);

        // Reset
        mb.vim_mode = MinibufferVimMode::Normal;
        mb.cursor = 0;
        mb.input = "test".to_string();

        // A switches to insert at end
        mb.handle_key("A", false, false, 0);
        assert_eq!(mb.vim_mode, MinibufferVimMode::Insert);
        assert_eq!(mb.cursor, 4);
    }

    #[test]
    fn test_input_parts() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);
        for ch in "hello".chars() {
            mb.handle_key(&ch.to_string(), false, false, 0);
        }
        mb.handle_key("left", false, false, 0);
        mb.handle_key("left", false, false, 0);

        let (before, after) = mb.input_parts();
        assert_eq!(before, "hel");
        assert_eq!(after, "lo");
    }

    #[test]
    fn test_message() {
        let mut mb = Minibuffer::new();
        mb.set_message("Written");
        assert_eq!(mb.message.as_deref(), Some("Written"));
        mb.clear_message();
        assert!(mb.message.is_none());
    }

    #[test]
    fn test_ctrl_g_dismisses() {
        let mut mb = Minibuffer::new();
        mb.activate(DelegateKind::Command, ":", false);
        assert_eq!(
            mb.handle_key("g", true, false, 0),
            MinibufferAction::Dismiss
        );
    }

    #[test]
    fn test_delegate_kind_preserved() {
        let mut mb = Minibuffer::new();

        mb.activate(DelegateKind::NoteSearch, "Find note:", false);
        assert_eq!(mb.delegate_kind, DelegateKind::NoteSearch);

        mb.activate(DelegateKind::VaultSwitch, "Switch vault:", false);
        assert_eq!(mb.delegate_kind, DelegateKind::VaultSwitch);

        mb.activate(DelegateKind::VaultOpen, "Open vault:", false);
        assert_eq!(mb.delegate_kind, DelegateKind::VaultOpen);

        mb.activate(DelegateKind::Command, "M-x", false);
        assert_eq!(mb.delegate_kind, DelegateKind::Command);
    }
}
