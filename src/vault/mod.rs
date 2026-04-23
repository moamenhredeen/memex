pub mod frontmatter;
pub mod id;
pub mod index;
pub mod layout;
pub mod scanner;
pub mod watcher;

pub use frontmatter::{Frontmatter, ParsedNote};
pub use index::{NoteIndex, NoteMeta, ResolveHit};
pub use layout::VaultLayout;
pub use scanner::{Note, VaultContents};
pub use watcher::{VaultChange, VaultWatcher};

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fs;

/// A vault is a directory containing markdown notes.
#[derive(Debug, Clone)]
pub struct Vault {
    pub path: PathBuf,
    pub name: String,
    /// Flat list of note + attachment paths. Derived from `contents` and
    /// kept as a convenience for consumers that still want one list of
    /// paths. New code should query `contents` or `index` instead.
    pub notes: Vec<PathBuf>,
    /// Parsed vault contents bucketed by kind (notes / journal / attachments).
    pub contents: VaultContents,
    /// In-memory index: wikilink resolution, backlinks, tags.
    pub index: NoteIndex,
}

impl Vault {
    /// Scan a directory and create a Vault from it. Also creates the
    /// well-known ~notes/attachments/journal/.memex~ folders if they
    /// don't already exist. Never touches existing files.
    pub fn open(path: PathBuf) -> Result<Self, std::io::Error> {
        let layout = VaultLayout::at(&path);
        layout.ensure()?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("vault")
            .to_string();
        let contents = scanner::scan(&layout)?;
        let index = NoteIndex::build(&contents);
        let notes = derive_paths(&contents);
        Ok(Self { path, name, notes, contents, index })
    }

    /// Resolved paths for this vault's well-known folders.
    pub fn layout(&self) -> VaultLayout {
        VaultLayout::at(&self.path)
    }

    /// Refresh the note list by re-scanning the directory.
    pub fn refresh(&mut self) -> Result<(), std::io::Error> {
        let layout = self.layout();
        self.contents = scanner::scan(&layout)?;
        self.index = NoteIndex::build(&self.contents);
        self.notes = derive_paths(&self.contents);
        Ok(())
    }

    /// Get note titles for display/search. Uses frontmatter titles when
    /// available, falling back to the filename stem for legacy notes.
    pub fn note_titles(&self) -> Vec<(String, PathBuf)> {
        self.contents
            .notes
            .iter()
            .chain(self.contents.journal.iter())
            .map(|n| (n.title.clone(), n.path.clone()))
            .collect()
    }

    /// Find notes that link to the given target title via the index.
    /// Returns `(display_title, path)` pairs.
    pub fn find_backlinks(&self, target_title: &str) -> Vec<(String, PathBuf)> {
        // Resolve the target to its ID, then pull from the precomputed
        // backlink map. Ambiguous titles link to all candidates — we
        // collect backlinks for each.
        let ids = match self.index.resolve_link(target_title) {
            Some(ResolveHit::Unique(id)) => vec![id.to_string()],
            Some(ResolveHit::Ambiguous(ids)) => ids.to_vec(),
            None => return Vec::new(),
        };

        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();
        for target_id in &ids {
            for source_id in self.index.backlinks_for(target_id) {
                if !seen.insert(source_id.clone()) {
                    continue;
                }
                if let Some(meta) = self.index.get(source_id) {
                    results.push((meta.title.clone(), meta.path.clone()));
                }
            }
        }
        results
    }
}

fn derive_paths(contents: &VaultContents) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = contents
        .notes
        .iter()
        .chain(contents.journal.iter())
        .map(|n| n.path.clone())
        .collect();
    v.extend(contents.attachments.iter().cloned());
    v.sort();
    v
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
