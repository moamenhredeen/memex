use crate::document::Document;
use crate::plugin::PluginEngine;

use super::undo::UndoHistory;

/// State shared by every editor window displaying the same markdown buffer.
///
/// Cursor, selection, folding, and viewport state intentionally live in
/// `EditorState`; they belong to an individual window, not to the buffer.
pub struct EditorBuffer {
    pub document: Document,
    pub history: UndoHistory,
    pub plugins: PluginEngine,
}

impl EditorBuffer {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            history: UndoHistory::new(),
            plugins: PluginEngine::new(),
        }
    }

    pub fn replace_document(&mut self, document: Document) {
        self.document = document;
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::*;
    use crate::editor::undo::EditOp;

    fn edit(range: Range<usize>) -> EditOp {
        EditOp {
            range,
            old_text: String::new(),
            new_text: "change".into(),
            cursor_before: 0,
            cursor_after: 6,
        }
    }

    #[test]
    fn replacing_document_resets_buffer_local_history() {
        let mut buffer = EditorBuffer::new(Document::scratch("before".into()));
        buffer.history.record(edit(0..0), 0..0);

        buffer.replace_document(Document::scratch("after".into()));

        assert_eq!(buffer.document.content(), "after");
        assert!(!buffer.history.can_undo());
    }
}
