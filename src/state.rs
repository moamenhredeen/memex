use std::path::PathBuf;

use crate::config::{self, MemexConfig};
use crate::document::Document;
use crate::vault::{Vault, VaultRegistry};

/// Central application state shared across components.
#[derive(Clone, Debug)]
pub struct AppState {
    /// The currently open vault.
    pub vault: Option<Vault>,
    /// Persisted vault registry.
    pub registry: VaultRegistry,
    /// Configuration loaded from global and vault TOML files.
    pub config: MemexConfig,
}

impl AppState {
    pub fn new() -> Self {
        let registry = VaultRegistry::load();

        let mut state = Self {
            vault: None,
            registry,
            config: MemexConfig::default(),
        };

        // Load global config
        state.config = config::load_config();

        // Try to restore last session
        if let Some(vault_path) = state.registry.last_vault_path()
            && vault_path.is_dir()
            && let Ok(vault) = Vault::open(vault_path.clone())
        {
            state.vault = Some(vault);
        }

        state
    }

    /// Open a note and update session history.
    pub fn open_document(&mut self, path: PathBuf) -> Result<Document, std::io::Error> {
        let document = Document::open(path.clone())?;
        // Update registry with last note
        if let Some(ref vault) = self.vault {
            self.registry.upsert_vault(&vault.path, Some(&path));
            let _ = self.registry.save();
        }
        Ok(document)
    }

    /// Create a new note in the current vault. Generates a new ID,
    /// writes frontmatter with `id`, `title`, `created`, and places the
    /// file under `notes/{id}.md`.
    pub fn create_note(&mut self, title: &str) -> Result<Document, std::io::Error> {
        let vault = self
            .vault
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no vault open"))?;

        let layout = vault.layout();
        let id = crate::vault::id::generate();
        let path = layout.note_path(&id);

        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("note already exists: {}", id),
            ));
        }

        let mut fm = crate::vault::Frontmatter::default();
        fm.id = Some(id.clone());
        fm.title = Some(title.to_string());
        fm.created = Some(crate::vault::id::iso_now());

        let body = format!("# {}\n", title);
        let initial_content = crate::vault::frontmatter::write(&fm, &body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        crate::fs::save_note(&path, &initial_content)?;

        // Rescan so the index picks up the new note.
        vault.refresh()?;

        self.registry.upsert_vault(&vault.path, Some(&path));
        let _ = self.registry.save();

        Ok(Document::from_content(path, initial_content))
    }

    /// Open a vault directory.
    pub fn open_vault(&mut self, path: PathBuf) -> Result<Option<Document>, std::io::Error> {
        let vault = Vault::open(path.clone())?;

        // Try last note for this vault, otherwise first note
        let note_to_open = self
            .registry
            .last_note_for(&path)
            .filter(|p| p.exists())
            .or_else(|| vault.first_note_path());

        self.vault = Some(vault);
        let document = note_to_open
            .map(|note_path| self.open_document(note_path))
            .transpose()?;

        // Update registry
        self.registry.upsert_vault(
            &path,
            document.as_ref().and_then(|document| document.path()),
        );
        let _ = self.registry.save();

        Ok(document)
    }

    /// Restore the most recent note for the active vault.
    pub fn restore_document(&mut self) -> Option<Document> {
        let vault = self.vault.as_ref()?;
        let path = self
            .registry
            .last_note_for(&vault.path)
            .filter(|path| path.exists())
            .or_else(|| vault.first_note_path())?;
        self.open_document(path).ok()
    }

    /// Get display title of current note.
    pub fn document_title(&self, path: Option<&std::path::Path>) -> String {
        let Some(path) = path else {
            return "untitled".to_string();
        };

        self.vault
            .as_ref()
            .and_then(|vault| vault.title_for_path(path))
            .map(str::to_owned)
            .unwrap_or_else(|| crate::fs::title_from_path(path))
    }

    /// Get display name of current vault.
    pub fn vault_name(&self) -> String {
        self.vault
            .as_ref()
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "No vault".to_string())
    }
}
