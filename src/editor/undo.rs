use std::ops::Range;
use std::time::{Duration, Instant};

/// A single atomic edit operation on the buffer.
#[derive(Clone, Debug)]
pub struct EditOp {
    /// Byte range in the document *before* the edit.
    pub range: Range<usize>,
    /// Text that was removed (empty for pure inserts).
    pub old_text: String,
    /// Text that was inserted (empty for pure deletes).
    pub new_text: String,
    /// Cursor position before the edit.
    pub cursor_before: usize,
    /// Cursor position after the edit.
    pub cursor_after: usize,
}

/// A group of related edits that undo/redo together as one unit.
#[derive(Clone, Debug)]
pub struct Transaction {
    pub ops: Vec<EditOp>,
    pub selection_before: Range<usize>,
    pub selection_after: Range<usize>,
    pub first_edit_at: Instant,
    pub last_edit_at: Instant,
    /// If true, this transaction won't merge with the next one.
    pub suppress_grouping: bool,
}

impl Transaction {
    fn new(op: EditOp, selection_before: Range<usize>) -> Self {
        let now = Instant::now();
        let selection_after = op.cursor_after..op.cursor_after;
        Self {
            ops: vec![op],
            selection_before,
            selection_after,
            first_edit_at: now,
            last_edit_at: now,
            suppress_grouping: false,
        }
    }

    /// Check if a new single-char edit can merge into this transaction.
    fn can_merge(&self, op: &EditOp, group_interval: Duration) -> bool {
        if self.suppress_grouping {
            return false;
        }
        let now = Instant::now();
        if now.duration_since(self.last_edit_at) > group_interval {
            return false;
        }
        // Only merge single-char inserts or single-char deletes
        let is_single_char_insert = op.old_text.is_empty() && op.new_text.len() <= 4;
        let is_single_char_delete = op.new_text.is_empty() && op.old_text.len() <= 4;
        if !is_single_char_insert && !is_single_char_delete {
            return false;
        }
        // Don't merge if the new edit involves a newline
        if op.new_text.contains('\n') || op.old_text.contains('\n') {
            return false;
        }
        true
    }

    fn merge(&mut self, op: EditOp) {
        self.selection_after = op.cursor_after..op.cursor_after;
        self.last_edit_at = Instant::now();
        self.ops.push(op);
    }

    /// Compute the inverse operations for undoing this transaction.
    /// Returns ops in reverse order, with ranges adjusted.
    pub fn inverse_ops(&self) -> Vec<EditOp> {
        let mut inverse = Vec::new();
        // Apply inversions in reverse order
        for op in self.ops.iter().rev() {
            inverse.push(EditOp {
                range: op.range.start..op.range.start + op.new_text.len(),
                old_text: op.new_text.clone(),
                new_text: op.old_text.clone(),
                cursor_before: op.cursor_after,
                cursor_after: op.cursor_before,
            });
        }
        inverse
    }
}

pub struct UndoHistory {
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    /// Time-based coalescing window (Zed pattern: ~500ms).
    group_interval: Duration,
    /// Nesting depth for compound operations (begin_group/end_group).
    transaction_depth: usize,
    /// Accumulating transaction for compound operations.
    compound_transaction: Option<Transaction>,
}

#[allow(dead_code)]

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            group_interval: Duration::from_millis(500),
            transaction_depth: 0,
            compound_transaction: None,
        }
    }

    /// Record an edit. Automatically coalesces with the previous transaction
    /// if within the group_interval and compatible.
    pub fn record(&mut self, op: EditOp, selection_before: Range<usize>) {
        // Any new edit clears the redo stack
        self.redo_stack.clear();

        if self.transaction_depth > 0 {
            // Inside a compound group — accumulate into the compound transaction
            if let Some(ref mut txn) = self.compound_transaction {
                txn.merge(op);
            } else {
                self.compound_transaction = Some(Transaction::new(op, selection_before));
            }
            return;
        }

        // Try to merge with the last transaction (time-based coalescing)
        if let Some(last) = self.undo_stack.last_mut() {
            if last.can_merge(&op, self.group_interval) {
                last.merge(op);
                return;
            }
        }

        // Start a new transaction
        self.undo_stack.push(Transaction::new(op, selection_before));
    }

    /// Begin a compound operation group. All edits until end_group()
    /// will be part of one undo unit.
    pub fn begin_group(&mut self, selection_before: Range<usize>) {
        if self.transaction_depth == 0 {
            self.compound_transaction = None;
        }
        self.transaction_depth += 1;
        // Store selection_before for the first level
        if self.transaction_depth == 1 && self.compound_transaction.is_none() {
            // Will be set when first op arrives
        }
        let _ = selection_before; // Used by the first record() call
    }

    /// End a compound operation group.
    pub fn end_group(&mut self) {
        if self.transaction_depth == 0 {
            return;
        }
        self.transaction_depth -= 1;
        if self.transaction_depth == 0 {
            if let Some(mut txn) = self.compound_transaction.take() {
                txn.suppress_grouping = true;
                self.undo_stack.push(txn);
            }
        }
    }

    /// Pop the last transaction from the undo stack and return it.
    /// The caller is responsible for applying the inverse operations.
    pub fn undo(&mut self) -> Option<Transaction> {
        let txn = self.undo_stack.pop()?;
        self.redo_stack.push(txn.clone());
        Some(txn)
    }

    /// Pop the last transaction from the redo stack and return it.
    /// The caller is responsible for re-applying the operations.
    pub fn redo(&mut self) -> Option<Transaction> {
        let txn = self.redo_stack.pop()?;
        self.undo_stack.push(txn.clone());
        Some(txn)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Force the current coalescing group to end, so the next edit
    /// starts a fresh transaction.
    pub fn break_coalescing(&mut self) {
        if let Some(last) = self.undo_stack.last_mut() {
            last.suppress_grouping = true;
        }
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.compound_transaction = None;
        self.transaction_depth = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_insert(at: usize, text: &str, cursor_after: usize) -> EditOp {
        EditOp {
            range: at..at,
            old_text: String::new(),
            new_text: text.to_string(),
            cursor_before: at,
            cursor_after,
        }
    }

    fn make_delete(range: Range<usize>, old_text: &str, cursor_after: usize) -> EditOp {
        EditOp {
            range,
            old_text: old_text.to_string(),
            new_text: String::new(),
            cursor_before: old_text.len(),
            cursor_after,
        }
    }

    #[test]
    fn test_basic_undo_redo() {
        let mut history = UndoHistory::new();

        // Type "hello" as separate chars — should coalesce
        for (i, ch) in "hello".chars().enumerate() {
            history.record(
                make_insert(i, &ch.to_string(), i + 1),
                i..i,
            );
        }

        // Should be one transaction due to coalescing
        assert_eq!(history.undo_stack.len(), 1);
        assert!(history.can_undo());
        assert!(!history.can_redo());

        let txn = history.undo().unwrap();
        assert_eq!(txn.ops.len(), 5);
        assert!(!history.can_undo());
        assert!(history.can_redo());

        let txn = history.redo().unwrap();
        assert_eq!(txn.ops.len(), 5);
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn test_newline_breaks_coalescing() {
        let mut history = UndoHistory::new();

        history.record(make_insert(0, "a", 1), 0..0);
        history.record(make_insert(1, "\n", 2), 1..1);

        // Newline should start a new transaction
        assert_eq!(history.undo_stack.len(), 2);
    }

    #[test]
    fn test_new_edit_clears_redo() {
        let mut history = UndoHistory::new();

        history.record(make_insert(0, "a", 1), 0..0);
        // Force new transaction
        history.break_coalescing();
        history.record(make_insert(1, "b", 2), 1..1);

        history.undo(); // undo "b"
        assert!(history.can_redo());

        // New edit should clear redo
        history.record(make_insert(1, "c", 2), 1..1);
        assert!(!history.can_redo());
    }

    #[test]
    fn test_compound_group() {
        let mut history = UndoHistory::new();

        history.begin_group(0..0);
        history.record(make_insert(0, "hello", 5), 0..0);
        history.record(make_insert(5, " world", 11), 5..5);
        history.end_group();

        // Should be one transaction
        assert_eq!(history.undo_stack.len(), 1);
        let txn = history.undo().unwrap();
        assert_eq!(txn.ops.len(), 2);
    }

    #[test]
    fn test_inverse_ops() {
        let op = EditOp {
            range: 3..3,
            old_text: String::new(),
            new_text: "hello".to_string(),
            cursor_before: 3,
            cursor_after: 8,
        };
        let txn = Transaction::new(op, 3..3);
        let inv = txn.inverse_ops();
        assert_eq!(inv.len(), 1);
        assert_eq!(inv[0].range, 3..8);
        assert_eq!(inv[0].old_text, "hello");
        assert_eq!(inv[0].new_text, "");
    }

    #[test]
    fn test_clear() {
        let mut history = UndoHistory::new();
        history.record(make_insert(0, "a", 1), 0..0);
        history.clear();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }
}
