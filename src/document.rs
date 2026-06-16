use std::path::{Path, PathBuf};

use ropey::Rope;

use crate::fs;

/// An editable markdown document.
///
/// The document is the single owner of file identity, text, and dirty state.
/// View-specific state such as cursors, selections, and scrolling remains in
/// `EditorState`.
pub struct Document {
    path: Option<PathBuf>,
    pub(crate) buffer: Rope,
    dirty: bool,
}

impl Document {
    pub fn scratch(content: String) -> Self {
        Self {
            path: None,
            buffer: Rope::from_str(&content),
            dirty: false,
        }
    }

    pub fn open(path: PathBuf) -> Result<Self, std::io::Error> {
        let content = fs::read_note(&path)?;
        Ok(Self::from_content(path, content))
    }

    pub fn from_content(path: PathBuf, content: String) -> Self {
        Self {
            path: Some(path),
            buffer: Rope::from_str(&content),
            dirty: false,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn content(&self) -> String {
        self.buffer.to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn replace_content(&mut self, content: String) {
        self.buffer = Rope::from_str(&content);
        self.dirty = false;
    }

    pub fn save(&mut self) -> Result<(), std::io::Error> {
        let path = self.path.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "document has no file path")
        })?;
        fs::save_note(path, &self.content())?;
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_clears_dirty_state() {
        let dir = std::env::temp_dir().join("memex-test-document-save");
        let path = dir.join("note.md");
        let mut document = Document::from_content(path.clone(), "before".into());
        document.buffer = Rope::from_str("after");
        document.mark_dirty();

        document.save().unwrap();

        assert!(!document.is_dirty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "after");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn scratch_document_cannot_be_saved() {
        let mut document = Document::scratch("text".into());
        assert_eq!(
            document.save().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
