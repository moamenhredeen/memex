use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fs;

/// A vault is a directory containing markdown notes.
#[derive(Debug, Clone)]
pub struct Vault {
    pub path: PathBuf,
    pub name: String,
    pub notes: Vec<PathBuf>,
}

impl Vault {
    /// Scan a directory and create a Vault from it.
    pub fn open(path: PathBuf) -> Result<Self, std::io::Error> {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("vault")
            .to_string();
        let notes = fs::list_vault_files(&path)?;
        Ok(Self { path, name, notes })
    }

    /// Refresh the note list by re-scanning the directory.
    pub fn refresh(&mut self) -> Result<(), std::io::Error> {
        self.notes = fs::list_vault_files(&self.path)?;
        Ok(())
    }

    /// Get note titles for display/search.
    pub fn note_titles(&self) -> Vec<(String, PathBuf)> {
        self.notes
            .iter()
            .map(|p| (fs::title_from_path(p), p.clone()))
            .collect()
    }
}

/// Persisted registry of known vaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultRegistry {
    pub vaults: Vec<VaultEntry>,
    pub last_vault: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub path: String,
    pub last_note: Option<String>,
    /// Unix timestamp of last time this vault was opened.
    #[serde(default)]
    pub last_opened: u64,
}

impl VaultRegistry {
    /// Load registry from disk, or return empty if not found.
    pub fn load() -> Self {
        let path = Self::registry_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save registry to disk.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::registry_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)
    }

    /// Add or update a vault entry.
    pub fn upsert_vault(&mut self, vault_path: &Path, last_note: Option<&Path>) {
        let path_str = vault_path.to_string_lossy().to_string();
        let last_note_str = last_note.map(|p| p.to_string_lossy().to_string());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if let Some(entry) = self.vaults.iter_mut().find(|e| e.path == path_str) {
            entry.last_note = last_note_str;
            entry.last_opened = now;
        } else {
            self.vaults.push(VaultEntry {
                path: path_str.clone(),
                last_note: last_note_str,
                last_opened: now,
            });
        }
        self.last_vault = Some(path_str);
    }

    /// Get the last-opened note for a given vault.
    pub fn last_note_for(&self, vault_path: &Path) -> Option<PathBuf> {
        let path_str = vault_path.to_string_lossy().to_string();
        self.vaults
            .iter()
            .find(|e| e.path == path_str)
            .and_then(|e| e.last_note.as_ref())
            .map(PathBuf::from)
    }

    /// Get the last-opened vault path.
    pub fn last_vault_path(&self) -> Option<PathBuf> {
        self.last_vault.as_ref().map(PathBuf::from)
    }

    /// Get all registered vault paths.
    pub fn vault_paths(&self) -> Vec<PathBuf> {
        self.vaults.iter().map(|e| PathBuf::from(&e.path)).collect()
    }

    /// Get registered vaults sorted by most-recently-used (newest first).
    /// Excludes the currently active vault if `exclude` is provided.
    pub fn recent_vaults(&self, exclude: Option<&Path>) -> Vec<&VaultEntry> {
        let exclude_str = exclude.map(|p| p.to_string_lossy().to_string());
        let mut vaults: Vec<&VaultEntry> = self
            .vaults
            .iter()
            .filter(|e| {
                if let Some(ref ex) = exclude_str {
                    &e.path != ex
                } else {
                    true
                }
            })
            .collect();
        vaults.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
        vaults
    }

    fn registry_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("memex")
            .join("vaults.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_scan() {
        let dir = std::env::temp_dir().join("memex-test-vault");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note-a.md"), "# Note A").unwrap();
        std::fs::write(dir.join("note-b.md"), "# Note B").unwrap();
        std::fs::write(dir.join("readme.txt"), "not a note").unwrap();

        let vault = Vault::open(dir.clone()).unwrap();
        assert_eq!(vault.notes.len(), 2);
        assert_eq!(vault.name, "memex-test-vault");

        let titles = vault.note_titles();
        assert!(titles.iter().any(|(t, _)| t == "note-a"));
        assert!(titles.iter().any(|(t, _)| t == "note-b"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_registry_roundtrip() {
        let mut reg = VaultRegistry::default();
        reg.upsert_vault(
            Path::new("/home/user/notes"),
            Some(Path::new("/home/user/notes/todo.md")),
        );
        reg.upsert_vault(Path::new("/home/user/work"), None);

        assert_eq!(reg.vaults.len(), 2);
        assert_eq!(
            reg.last_note_for(Path::new("/home/user/notes")),
            Some(PathBuf::from("/home/user/notes/todo.md"))
        );
        assert_eq!(reg.last_note_for(Path::new("/home/user/work")), None);
        assert_eq!(
            reg.last_vault_path(),
            Some(PathBuf::from("/home/user/work"))
        );

        let json = serde_json::to_string(&reg).unwrap();
        let loaded: VaultRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.vaults.len(), 2);
    }
}
