use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use crate::document::Document;
use crate::plugin::PluginEngine;

use super::commands::EditorCommand;
use super::undo::{EditOp, Transaction, UndoHistory};

/// A cloneable handle to state shared by every window displaying a buffer.
#[derive(Clone)]
pub struct EditorBuffer {
    inner: Rc<RefCell<EditorBufferInner>>,
}

struct EditorBufferInner {
    document: Document,
    history: UndoHistory,
    plugins: PluginEngine,
    revision: u64,
}

impl EditorBuffer {
    pub fn new(document: Document) -> Self {
        Self {
            inner: Rc::new(RefCell::new(EditorBufferInner {
                document,
                history: UndoHistory::new(),
                plugins: PluginEngine::new(),
                revision: 0,
            })),
        }
    }

    pub fn content(&self) -> String {
        self.inner.borrow().document.content()
    }

    pub fn len_bytes(&self) -> usize {
        self.inner.borrow().document.buffer.len_bytes()
    }

    pub fn len_chars(&self) -> usize {
        self.inner.borrow().document.buffer.len_chars()
    }

    pub fn byte_to_char(&self, offset: usize) -> usize {
        self.inner.borrow().document.buffer.byte_to_char(offset)
    }

    pub fn char_to_byte(&self, index: usize) -> usize {
        self.inner.borrow().document.buffer.char_to_byte(index)
    }

    pub fn slice_bytes(&self, range: Range<usize>) -> String {
        let inner = self.inner.borrow();
        let start = inner.document.buffer.byte_to_char(range.start);
        let end = inner.document.buffer.byte_to_char(range.end);
        inner.document.buffer.slice(start..end).to_string()
    }

    pub fn document_path(&self) -> Option<PathBuf> {
        self.inner.borrow().document.path().map(ToOwned::to_owned)
    }

    pub fn is_dirty(&self) -> bool {
        self.inner.borrow().document.is_dirty()
    }

    pub fn revision(&self) -> u64 {
        self.inner.borrow().revision
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        self.inner.borrow_mut().document.save()
    }

    pub fn replace_content(&self, content: String) {
        let mut inner = self.inner.borrow_mut();
        inner.document.replace_content(content);
        inner.history.clear();
        inner.revision += 1;
    }

    pub fn replace_document(&self, document: Document) {
        let mut inner = self.inner.borrow_mut();
        inner.document = document;
        inner.history.clear();
        inner.revision += 1;
    }

    pub fn replace_range(&self, range: Range<usize>, new_text: &str) {
        let mut inner = self.inner.borrow_mut();
        let start = inner.document.buffer.byte_to_char(range.start);
        let end = inner.document.buffer.byte_to_char(range.end);
        if start != end {
            inner.document.buffer.remove(start..end);
        }
        if !new_text.is_empty() {
            inner.document.buffer.insert(start, new_text);
        }
        inner.document.mark_dirty();
        inner.revision += 1;
    }

    pub fn record_edit(&self, op: EditOp, selection_before: Range<usize>) {
        self.inner.borrow_mut().history.record(op, selection_before);
    }

    pub fn begin_edit_group(&self, selection_before: Range<usize>) {
        self.inner.borrow_mut().history.begin_group(selection_before);
    }

    pub fn end_edit_group(&self) {
        self.inner.borrow_mut().history.end_group();
    }

    pub fn break_undo_coalescing(&self) {
        self.inner.borrow_mut().history.break_coalescing();
    }

    pub fn undo(&self) -> Option<Transaction> {
        self.inner.borrow_mut().history.undo()
    }

    pub fn redo(&self) -> Option<Transaction> {
        self.inner.borrow_mut().history.redo()
    }

    pub fn run_plugin_command(
        &self,
        name: &str,
        content: &str,
        cursor: usize,
        selection: (usize, usize),
    ) -> Option<Vec<EditorCommand>> {
        self.inner
            .borrow_mut()
            .plugins
            .run_command(name, content, cursor, selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_handles_share_document_content_and_revision() {
        let buffer = EditorBuffer::new(Document::scratch("before".into()));
        let second_window = buffer.clone();

        buffer.replace_range(0..6, "after");

        assert_eq!(second_window.content(), "after");
        assert_eq!(second_window.revision(), 1);
    }

    #[test]
    fn replacing_document_resets_buffer_local_history() {
        let buffer = EditorBuffer::new(Document::scratch("before".into()));
        buffer.record_edit(
            EditOp {
                range: 0..0,
                old_text: String::new(),
                new_text: "change".into(),
                cursor_before: 0,
                cursor_after: 6,
            },
            0..0,
        );

        buffer.replace_document(Document::scratch("after".into()));

        assert_eq!(buffer.content(), "after");
        assert!(buffer.undo().is_none());
    }
}
