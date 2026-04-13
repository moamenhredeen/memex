use std::ops::Range;

use super::action::{Action, LayerId, MotionId, MotionImpl, OperatorId};
use super::layer::LayerStack;

/// Result of the grammar engine processing an action.
#[derive(Clone, Debug)]
pub enum GrammarResult {
    /// Move cursor to byte offset.
    MoveCursor(usize),
    /// Insert a character at cursor.
    InsertChar(char),
    /// Delete a range and yank the text.
    DeleteRange {
        range: Range<usize>,
        yanked: String,
        enter_insert: bool,
    },
    /// Yank text without deleting.
    Yank(String),
    /// Indent a line range.
    IndentRange { line_start: usize, text: String },
    /// Dedent a line range.
    DedentRange { range: Range<usize> },
    /// Execute a registered command by ID.
    ExecuteCommand(&'static str),
    /// Execute a sequence of results.
    Batch(Vec<GrammarResult>),
    /// Activate a layer (mode switch).
    ActivateLayer(LayerId),
    /// Push a transient layer.
    PushTransient(LayerId),
    /// Replace character(s) under cursor.
    ReplaceChar { ch: char, count: usize },
    /// Run a Rhai script function.
    RunScript(String),
    /// Waiting for more input (operator pending, count accumulation).
    Pending,
    /// Key consumed, nothing to do.
    Noop,
}

/// Vim grammar engine — handles operator+motion composition, registers.
///
/// This is editor-specific: motions compute cursor positions from buffer content,
/// operators apply to text ranges. It knows nothing about key resolution.
/// The keymap system resolves keys to actions; the grammar composes actions into
/// editor mutations.
pub struct VimGrammar {
    pub pending_operator: Option<OperatorId>,
    /// Count stored when operator was pressed (for linewise ops like `3dd`).
    pending_count: Option<usize>,
    pub register: char,
    pub register_content: String,
    pub last_char_search: Option<(char, &'static str)>,
}

impl VimGrammar {
    pub fn new() -> Self {
        Self {
            pending_operator: None,
            pending_count: None,
            register: '"',
            register_content: String::new(),
            last_char_search: None,
        }
    }

    /// Clear pending state.
    pub fn clear_pending(&mut self) {
        self.pending_operator = None;
        self.pending_count = None;
    }

    /// Resolve a pre-computed motion target through the grammar.
    /// If an operator is pending, applies the operator to cursor..target range.
    /// Otherwise returns MoveCursor.
    pub fn resolve_with_target(&mut self, target: usize, content: &str, cursor: usize) -> GrammarResult {
        if let Some(op_id) = self.pending_operator.take() {
            self.pending_count = None;
            let (start, end) = if target < cursor { (target, cursor) } else { (cursor, target) };
            if start == end {
                return GrammarResult::Noop;
            }
            let yanked = content.get(start..end).unwrap_or("").to_string();
            match op_id {
                "delete" => {
                    self.register_content = yanked.clone();
                    GrammarResult::DeleteRange { range: start..end, yanked, enter_insert: false }
                }
                "change" => {
                    self.register_content = yanked.clone();
                    GrammarResult::DeleteRange { range: start..end, yanked, enter_insert: true }
                }
                "yank" => {
                    self.register_content = yanked.clone();
                    GrammarResult::Yank(yanked)
                }
                _ => GrammarResult::Noop,
            }
        } else {
            GrammarResult::MoveCursor(target)
        }
    }

    /// Process an action through the vim grammar.
    /// `count` is the accumulated count from the keymap system.
    pub fn process(
        &mut self,
        action: Action,
        key: &str,
        count: usize,
        content: &str,
        cursor: usize,
        stack: &mut LayerStack,
    ) -> GrammarResult {
        match action {
            Action::Motion(motion_id) => {
                if let Some(op_id) = self.pending_operator.take() {
                    // Operator + motion → compute range, apply operator
                    self.pending_count = None;
                    self.apply_operator_with_motion(op_id, motion_id, content, cursor, count, stack)
                } else {
                    // Just move cursor
                    if let Some(motion_impl) = stack.get_motion(motion_id) {
                        match motion_impl {
                            MotionImpl::Native(f) => {
                                let target = f(content, cursor, count);
                                GrammarResult::MoveCursor(target)
                            }
                            MotionImpl::Script(name) => {
                                GrammarResult::RunScript(name.clone())
                            }
                        }
                    } else {
                        GrammarResult::Noop
                    }
                }
            }

            Action::Operator(op_id) => {
                if self.pending_operator == Some(op_id) {
                    // Double operator (dd, yy, cc) — line-wise
                    let effective = self.pending_count.unwrap_or(count);
                    self.clear_pending();
                    self.apply_linewise_operator(op_id, content, cursor, effective)
                } else {
                    // Set pending operator, wait for motion
                    self.pending_operator = Some(op_id);
                    self.pending_count = Some(count);
                    GrammarResult::Pending
                }
            }

            Action::SelfInsert => {
                if let Some(ch) = key.chars().next() {
                    if key.chars().count() == 1 {
                        GrammarResult::InsertChar(ch)
                    } else {
                        GrammarResult::Noop
                    }
                } else {
                    GrammarResult::Noop
                }
            }

            Action::Command(cmd_id) => {
                self.clear_pending();
                GrammarResult::ExecuteCommand(cmd_id)
            }

            Action::ActivateLayer(layer_id) => {
                self.clear_pending();
                stack.activate_layer(layer_id);
                GrammarResult::ActivateLayer(layer_id)
            }

            Action::PushTransient(layer_id) => {
                stack.push_transient(layer_id);
                GrammarResult::PushTransient(layer_id)
            }

            Action::Script(name) => {
                self.clear_pending();
                GrammarResult::RunScript(name)
            }

            Action::Sequence(actions) => {
                let mut results = Vec::new();
                for a in actions {
                    let r = self.process(a, key, count, content, cursor, stack);
                    results.push(r);
                }
                GrammarResult::Batch(results)
            }

            Action::Noop => GrammarResult::Noop,
        }
    }

    /// Apply an operator with a motion to produce a range operation.
    fn apply_operator_with_motion(
        &mut self,
        op_id: OperatorId,
        motion_id: MotionId,
        content: &str,
        cursor: usize,
        count: usize,
        stack: &LayerStack,
    ) -> GrammarResult {
        let target = if let Some(MotionImpl::Native(f)) = stack.get_motion(motion_id) {
            f(content, cursor, count)
        } else {
            return GrammarResult::Noop;
        };

        let (start, end) = if target < cursor {
            (target, cursor)
        } else {
            (cursor, target)
        };

        if start == end {
            return GrammarResult::Noop;
        }

        let yanked = content.get(start..end).unwrap_or("").to_string();

        match op_id {
            "delete" => {
                self.register_content = yanked.clone();
                GrammarResult::DeleteRange {
                    range: start..end,
                    yanked,
                    enter_insert: false,
                }
            }
            "change" => {
                self.register_content = yanked.clone();
                GrammarResult::DeleteRange {
                    range: start..end,
                    yanked,
                    enter_insert: true,
                }
            }
            "yank" => {
                self.register_content = yanked.clone();
                GrammarResult::Yank(yanked)
            }
            "indent" => {
                let line_start = content[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                GrammarResult::IndentRange {
                    line_start,
                    text: "    ".to_string(),
                }
            }
            "dedent" => {
                let line_start = content[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let spaces = content[line_start..].chars().take_while(|c| *c == ' ').count().min(4);
                if spaces > 0 {
                    GrammarResult::DedentRange {
                        range: line_start..line_start + spaces,
                    }
                } else {
                    GrammarResult::Noop
                }
            }
            _ => GrammarResult::Noop,
        }
    }

    /// Apply a line-wise operator (dd, yy, cc, >>, <<).
    fn apply_linewise_operator(
        &self,
        op_id: OperatorId,
        content: &str,
        cursor: usize,
        count: usize,
    ) -> GrammarResult {
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
        let yanked = content.get(line_start..end).unwrap_or("").to_string();

        match op_id {
            "delete" => GrammarResult::DeleteRange {
                range: line_start..end,
                yanked,
                enter_insert: false,
            },
            "change" => GrammarResult::DeleteRange {
                range: line_start..end,
                yanked,
                enter_insert: true,
            },
            "yank" => GrammarResult::Yank(yanked),
            "indent" => GrammarResult::IndentRange {
                line_start,
                text: "    ".to_string(),
            },
            "dedent" => {
                let spaces = content[line_start..].chars().take_while(|c| *c == ' ').count().min(4);
                if spaces > 0 {
                    GrammarResult::DedentRange {
                        range: line_start..line_start + spaces,
                    }
                } else {
                    GrammarResult::Noop
                }
            }
            _ => GrammarResult::Noop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::action::MotionImpl;
    use crate::keymap::layer::Layer;

    fn test_stack() -> LayerStack {
        let mut stack = LayerStack::new();

        // Register motions
        stack.register_motion("right", MotionImpl::Native(|content, cursor, count| {
            let mut pos = cursor;
            for _ in 0..count {
                if pos < content.len() {
                    pos += 1;
                    while pos < content.len() && !content.is_char_boundary(pos) {
                        pos += 1;
                    }
                }
            }
            pos
        }));
        stack.register_motion("left", MotionImpl::Native(|_content, cursor, count| {
            cursor.saturating_sub(count)
        }));
        stack.register_motion("line-end", MotionImpl::Native(|content, cursor, _count| {
            content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len())
        }));
        stack.register_motion("word-forward", MotionImpl::Native(|content, cursor, count| {
            let mut pos = cursor;
            for _ in 0..count {
                // Skip current word chars
                while pos < content.len() && !content[pos..].starts_with(|c: char| c.is_whitespace()) {
                    pos += 1;
                }
                // Skip whitespace
                while pos < content.len() && content[pos..].starts_with(|c: char| c.is_whitespace()) {
                    pos += 1;
                }
            }
            pos
        }));

        let vim_normal = Layer::new("vim:normal")
            .with_group("vim-state")
            .bind("h", Action::Motion("left"))
            .bind("l", Action::Motion("right"))
            .bind("$", Action::Motion("line-end"))
            .bind("w", Action::Motion("word-forward"))
            .bind("d", Action::Operator("delete"))
            .bind("c", Action::Operator("change"))
            .bind("y", Action::Operator("yank"))
            .bind("i", Action::ActivateLayer("vim:insert"));

        let vim_insert = Layer::new("vim:insert")
            .with_group("vim-state")
            .bind("escape", Action::ActivateLayer("vim:normal"));

        let global = Layer::new("global")
            .bind("a", Action::SelfInsert);

        stack.register_layer(vim_normal);
        stack.register_layer(vim_insert);
        stack.register_layer(global);
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");
        stack
    }

    #[test]
    fn test_motion_moves_cursor() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        let result = grammar.process(Action::Motion("right"), "l", 1, "hello", 1, &mut stack);
        match result {
            GrammarResult::MoveCursor(pos) => assert_eq!(pos, 2),
            other => panic!("Expected MoveCursor, got {:?}", other),
        }
    }

    #[test]
    fn test_operator_pending() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        // Press "d" — should go pending
        let result = grammar.process(Action::Operator("delete"), "d", 1, "hello world", 0, &mut stack);
        assert!(matches!(result, GrammarResult::Pending));
        assert_eq!(grammar.pending_operator, Some("delete"));
    }

    #[test]
    fn test_delete_word() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        // "dw" — delete word
        grammar.process(Action::Operator("delete"), "d", 1, "hello world", 0, &mut stack);
        let result = grammar.process(Action::Motion("word-forward"), "w", 1, "hello world", 0, &mut stack);

        match result {
            GrammarResult::DeleteRange { range, yanked, enter_insert } => {
                assert_eq!(range, 0..6);
                assert_eq!(yanked, "hello ");
                assert!(!enter_insert);
            }
            other => panic!("Expected DeleteRange, got {:?}", other),
        }
    }

    #[test]
    fn test_change_word() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        // "cw" — change word (delete + enter insert)
        grammar.process(Action::Operator("change"), "c", 1, "hello world", 0, &mut stack);
        let result = grammar.process(Action::Motion("word-forward"), "w", 1, "hello world", 0, &mut stack);

        match result {
            GrammarResult::DeleteRange { enter_insert, .. } => {
                assert!(enter_insert);
            }
            other => panic!("Expected DeleteRange, got {:?}", other),
        }
    }

    #[test]
    fn test_yank_word() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        // "yw" — yank word
        grammar.process(Action::Operator("yank"), "y", 1, "hello world", 0, &mut stack);
        let result = grammar.process(Action::Motion("word-forward"), "w", 1, "hello world", 0, &mut stack);

        match result {
            GrammarResult::Yank(text) => assert_eq!(text, "hello "),
            other => panic!("Expected Yank, got {:?}", other),
        }
        assert_eq!(grammar.register_content, "hello ");
    }

    #[test]
    fn test_dd_linewise_delete() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        // "dd" — delete line
        grammar.process(Action::Operator("delete"), "d", 1, "hello\nworld\n", 2, &mut stack);
        let result = grammar.process(Action::Operator("delete"), "d", 1, "hello\nworld\n", 2, &mut stack);

        match result {
            GrammarResult::DeleteRange { range, yanked, .. } => {
                assert_eq!(range, 0..6); // "hello\n"
                assert_eq!(yanked, "hello\n");
            }
            other => panic!("Expected DeleteRange, got {:?}", other),
        }
    }

    #[test]
    fn test_count_with_motion() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        // Count is now passed directly from the keymap system
        let result = grammar.process(Action::Motion("right"), "l", 3, "hello world", 0, &mut stack);

        match result {
            GrammarResult::MoveCursor(pos) => assert_eq!(pos, 3),
            other => panic!("Expected MoveCursor, got {:?}", other),
        }
    }

    #[test]
    fn test_self_insert() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        let result = grammar.process(Action::SelfInsert, "a", 1, "hello", 0, &mut stack);
        match result {
            GrammarResult::InsertChar('a') => {}
            other => panic!("Expected InsertChar('a'), got {:?}", other),
        }
    }

    #[test]
    fn test_activate_layer() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        let result = grammar.process(Action::ActivateLayer("vim:insert"), "i", 1, "hello", 0, &mut stack);

        assert!(matches!(result, GrammarResult::ActivateLayer("vim:insert")));
        assert!(stack.is_active("vim:insert"));
        assert!(!stack.is_active("vim:normal")); // mutually exclusive
    }

    #[test]
    fn test_command_clears_pending() {
        let mut stack = test_stack();
        let mut grammar = VimGrammar::new();

        grammar.pending_operator = Some("delete");

        let result = grammar.process(Action::Command("undo"), "u", 1, "hello", 0, &mut stack);
        assert!(matches!(result, GrammarResult::ExecuteCommand("undo")));
        assert!(grammar.pending_operator.is_none());
    }
}
