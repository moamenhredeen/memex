use std::path::PathBuf;

use crate::config::{self, MemexConfig};
use crate::fs;
use crate::vault::{Vault, VaultRegistry};

/// Central application state shared across components.
#[derive(Clone, Debug)]
pub struct AppState {
    /// The currently open vault.
    pub vault: Option<Vault>,
    /// Path to the currently open note file.
    pub current_note: Option<PathBuf>,
    /// The editor content (source of truth for the buffer).
    pub content: String,
    /// Whether the buffer has unsaved changes.
    pub dirty: bool,
    /// Persisted vault registry.
    pub registry: VaultRegistry,
    /// Configuration loaded from Rhai scripts.
    pub config: MemexConfig,
}

impl AppState {
    pub fn new() -> Self {
        let registry = VaultRegistry::load();

        let mut state = Self {
            vault: None,
            current_note: None,
            content: String::new(),
            dirty: false,
            registry,
            config: MemexConfig::default(),
        };

        // Load global config
        state.config = config::load_config(None);

        // Try to restore last session
        if let Some(vault_path) = state.registry.last_vault_path() {
            if vault_path.is_dir() {
                if let Ok(vault) = Vault::open(vault_path.clone()) {
                    // Reload config with vault path for per-vault overrides
                    state.config = config::load_config(Some(&vault_path));

                    // Try last note, otherwise first note
                    let note_to_open = state
                        .registry
                        .last_note_for(&vault_path)
                        .filter(|p| p.exists())
                        .or_else(|| vault.notes.first().cloned());

                    state.vault = Some(vault);

                    if let Some(note_path) = note_to_open {
                        let _ = state.open_note(note_path);
                    }
                }
            }
        }

        state
    }

    /// Open a note file: read from disk and update state.
    pub fn open_note(&mut self, path: PathBuf) -> Result<(), std::io::Error> {
        let content = fs::read_note(&path)?;
        self.content = content;
        self.current_note = Some(path.clone());
        self.dirty = false;

        // Update registry with last note
        if let Some(ref vault) = self.vault {
            self.registry
                .upsert_vault(&vault.path, Some(&path));
            let _ = self.registry.save();
        }

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
        let vault = self.vault.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no vault open")
        })?;

        let filename = fs::slugify(title);
        let path = vault.path.join(&filename);

        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("note already exists: {}", filename),
            ));
        }

        let initial_content = format!("# {}\n", title);
        fs::save_note(&path, &initial_content)?;

        // Refresh vault notes list
        vault.refresh()?;

        self.content = initial_content;
        self.current_note = Some(path.clone());
        self.dirty = false;

        // Update registry
        self.registry.upsert_vault(&vault.path, Some(&path));
        let _ = self.registry.save();

        Ok(path)
    }

    /// Open a vault directory.
    pub fn open_vault(&mut self, path: PathBuf) -> Result<(), std::io::Error> {
        let vault = Vault::open(path.clone())?;

        // Reload config with vault-specific overrides
        self.config = config::load_config(Some(&path));

        // Try last note for this vault, otherwise first note
        let note_to_open = self
            .registry
            .last_note_for(&path)
            .filter(|p| p.exists())
            .or_else(|| vault.notes.first().cloned());

        self.vault = Some(vault);

        if let Some(note_path) = note_to_open {
            self.open_note(note_path)?;
        } else {
            self.content = String::new();
            self.current_note = None;
            self.dirty = false;
        }

        // Update registry
        self.registry
            .upsert_vault(&path, self.current_note.as_deref());
        let _ = self.registry.save();

        Ok(())
    }

    /// Reload configuration from Rhai scripts.
    pub fn reload_config(&mut self) {
        let vault_path = self.vault.as_ref().map(|v| v.path.as_path());
        self.config = config::load_config(vault_path);
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
        self.vault
            .as_ref()
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "No vault".to_string())
    }
}
