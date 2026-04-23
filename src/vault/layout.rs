//! Vault folder layout conventions.
//!
//! A Memex vault is a directory with four well-known subfolders:
//! ```text
//! my-vault/
//!   notes/          # flat, ID-based note filenames
//!   attachments/    # PDFs, images, drawings
//!   journal/        # daily notes (YYYY-MM-DD.md)
//!   .memex/         # per-vault cache and config
//! ```
//!
//! Missing folders are created when a vault is opened. Existing files
//! are never moved or touched — this module is additive only.

use std::io;
use std::path::{Path, PathBuf};

pub const NOTES_DIR: &str = "notes";
pub const ATTACHMENTS_DIR: &str = "attachments";
pub const JOURNAL_DIR: &str = "journal";
pub const DOT_MEMEX_DIR: &str = ".memex";

/// Resolved paths for the well-known folders, rooted at a vault path.
pub struct VaultLayout {
    pub root: PathBuf,
    pub notes: PathBuf,
    pub attachments: PathBuf,
    pub journal: PathBuf,
    pub dot_memex: PathBuf,
}

impl VaultLayout {
    pub fn at(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            notes: root.join(NOTES_DIR),
            attachments: root.join(ATTACHMENTS_DIR),
            journal: root.join(JOURNAL_DIR),
            dot_memex: root.join(DOT_MEMEX_DIR),
            root,
        }
    }

    /// Create any missing well-known folders. Safe to call on an
    /// already-initialised vault — `create_dir_all` is idempotent.
    pub fn ensure(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.notes)?;
        std::fs::create_dir_all(&self.attachments)?;
        std::fs::create_dir_all(&self.journal)?;
        std::fs::create_dir_all(&self.dot_memex)?;
        Ok(())
    }

    /// Where a new note with the given ID should live.
    pub fn note_path(&self, id: &str) -> PathBuf {
        self.notes.join(format!("{}.md", id))
    }

    /// Where a journal entry for the given ISO date (YYYY-MM-DD) lives.
    pub fn journal_path(&self, iso_date: &str) -> PathBuf {
        self.journal.join(format!("{}.md", iso_date))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("memex-layout-{}", name));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn ensure_creates_all_folders() {
        let root = tmp_root("ensure");
        let layout = VaultLayout::at(&root);
        layout.ensure().unwrap();

        assert!(layout.notes.is_dir());
        assert!(layout.attachments.is_dir());
        assert!(layout.journal.is_dir());
        assert!(layout.dot_memex.is_dir());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ensure_is_idempotent() {
        let root = tmp_root("idempotent");
        let layout = VaultLayout::at(&root);
        layout.ensure().unwrap();
        // Drop a sentinel file inside notes/ to verify we don't clobber.
        let sentinel = layout.notes.join("existing.md");
        std::fs::write(&sentinel, "hello").unwrap();
        layout.ensure().unwrap();
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "hello");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn note_path_uses_notes_dir() {
        let layout = VaultLayout::at("/root");
        let p = layout.note_path("20260423T142301-k3n8");
        assert!(p.to_string_lossy().ends_with("notes/20260423T142301-k3n8.md")
            || p.to_string_lossy().ends_with("notes\\20260423T142301-k3n8.md"));
    }

    #[test]
    fn journal_path_uses_journal_dir() {
        let layout = VaultLayout::at("/root");
        let p = layout.journal_path("2026-04-23");
        assert!(p.to_string_lossy().ends_with("journal/2026-04-23.md")
            || p.to_string_lossy().ends_with("journal\\2026-04-23.md"));
    }
}
