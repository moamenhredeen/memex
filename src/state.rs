use std::path::PathBuf;

use freya::prelude::*;

use crate::fs;

/// Central application state shared across components.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Path to the currently open vault directory.
    pub vault_path: Option<PathBuf>,
    /// Path to the currently open note file.
    pub current_note: Option<PathBuf>,
    /// The editor content (source of truth for the buffer).
    pub content: String,
    /// Whether the buffer has unsaved changes.
    pub dirty: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            vault_path: None,
            current_note: None,
            content: String::new(),
            dirty: false,
        }
    }

    /// Open a note file: read from disk and update state.
    pub fn open_note(&mut self, path: PathBuf) -> Result<(), std::io::Error> {
        let content = fs::read_note(&path)?;
        self.content = content;
        self.current_note = Some(path);
        self.dirty = false;
        Ok(())
    }

    /// Save the current buffer to its file path.
    pub fn save(&mut self) -> Result<(), std::io::Error> {
        if let Some(ref path) = self.current_note {
            fs::save_note(path, &self.content)?;
            self.dirty = false;
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no file path set",
            ))
        }
    }

    /// Create a new note in the current vault.
    pub fn create_note(&mut self, title: &str) -> Result<PathBuf, std::io::Error> {
        let vault = self.vault_path.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no vault open")
        })?;

        let filename = fs::slugify(title);
        let path = vault.join(&filename);

        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("note already exists: {}", filename),
            ));
        }

        let initial_content = format!("# {}\n", title);
        fs::save_note(&path, &initial_content)?;

        self.content = initial_content;
        self.current_note = Some(path.clone());
        self.dirty = false;
        Ok(path)
    }

    /// Get display title of current note.
    pub fn current_title(&self) -> String {
        self.current_note
            .as_ref()
            .map(|p| fs::title_from_path(p))
            .unwrap_or_else(|| "untitled".to_string())
    }

    /// Get display name of current vault.
    pub fn vault_name(&self) -> String {
        self.vault_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("No vault")
            .to_string()
    }

    /// Set vault path and scan for notes.
    pub fn open_vault(&mut self, path: PathBuf) -> Result<Vec<PathBuf>, std::io::Error> {
        let notes = fs::list_notes(&path)?;
        self.vault_path = Some(path);

        // Open the first note if available
        if let Some(first) = notes.first() {
            self.open_note(first.clone())?;
        } else {
            self.content = String::new();
            self.current_note = None;
            self.dirty = false;
        }

        Ok(notes)
    }
}
