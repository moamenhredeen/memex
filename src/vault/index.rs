//! In-memory note index.
//!
//! Single source of truth used by wikilink resolution, backlinks, tag
//! search, and title fuzzy-match. Built from [`super::scanner::scan`]
//! output; updated incrementally by the file watcher.
//!
//! Keys are deliberately lowercased where case-insensitive matching is
//! expected (titles, aliases, tags). IDs stay case-sensitive.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::scanner::{Note, VaultContents};

/// Fully-resolved note metadata kept in the index. Mirrors [`Note`] but
/// owns the data so the index can live independently of the scan result.
#[derive(Debug, Clone)]
pub struct NoteMeta {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub status: Option<String>,
    pub outgoing_links: Vec<String>,
    pub is_journal: bool,
}

impl From<Note> for NoteMeta {
    fn from(n: Note) -> Self {
        Self {
            id: n.id,
            path: n.path,
            title: n.title,
            tags: n.tags,
            aliases: n.aliases,
            status: n.status,
            outgoing_links: n.outgoing_links,
            is_journal: n.is_journal,
        }
    }
}

/// Resolved lookup result. Priority at query time: direct id → alias → title.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolveHit<'a> {
    /// Unambiguous match.
    Unique(&'a str),
    /// Multiple notes share this key. Caller decides disambiguation
    /// (or surfaces a warning).
    Ambiguous(&'a [String]),
}

/// The index. Rebuild with [`NoteIndex::build`]; mutate incrementally
/// with [`NoteIndex::upsert`] / [`NoteIndex::remove`].
#[derive(Debug, Default, Clone)]
pub struct NoteIndex {
    by_id: HashMap<String, NoteMeta>,
    /// Lowercased title → IDs (Vec because duplicate titles are legal).
    by_title: HashMap<String, Vec<String>>,
    /// Lowercased alias → IDs.
    by_alias: HashMap<String, Vec<String>>,
    /// Tag → IDs. Tags are case-preserving in display, but matched
    /// case-insensitively here.
    by_tag: BTreeMap<String, Vec<String>>,
    /// `target_id → notes that link to it`. Populated by resolving
    /// outgoing_links at build / upsert time.
    backlinks: HashMap<String, Vec<String>>,
}

impl NoteIndex {
    /// Build from a completed scan. Journal notes participate in the
    /// index too — they're full notes with their own IDs.
    pub fn build(contents: &VaultContents) -> Self {
        let mut idx = Self::default();
        for n in contents.notes.iter().chain(contents.journal.iter()) {
            idx.insert_note(n.clone().into());
        }
        idx.rebuild_backlinks();
        idx
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&NoteMeta> {
        self.by_id.get(id)
    }

    /// All IDs that carry a given tag (case-insensitive match).
    pub fn notes_with_tag(&self, tag: &str) -> &[String] {
        self.by_tag
            .get(&tag.to_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Full tag list with counts. Useful for `:tags` command.
    pub fn all_tags(&self) -> Vec<(String, usize)> {
        self.by_tag
            .iter()
            .map(|(t, ids)| (t.clone(), ids.len()))
            .collect()
    }

    /// Resolve a wikilink target to a note ID. Priority: explicit
    /// `id:...` → alias → title.
    pub fn resolve_link(&self, target: &str) -> Option<ResolveHit<'_>> {
        if let Some(rest) = target.strip_prefix("id:") {
            return self.by_id.get(rest).map(|m| ResolveHit::Unique(m.id.as_str()));
        }
        let key = target.to_lowercase();
        if let Some(ids) = self.by_alias.get(&key) {
            return Some(slice_to_hit(ids));
        }
        if let Some(ids) = self.by_title.get(&key) {
            return Some(slice_to_hit(ids));
        }
        None
    }

    /// IDs of notes that link to `target_id`.
    pub fn backlinks_for(&self, target_id: &str) -> &[String] {
        self.backlinks
            .get(target_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Orphans: notes with zero incoming and zero outgoing links.
    pub fn orphans(&self) -> Vec<&str> {
        self.by_id
            .values()
            .filter(|m| {
                m.outgoing_links.is_empty()
                    && self.backlinks.get(&m.id).map_or(true, Vec::is_empty)
            })
            .map(|m| m.id.as_str())
            .collect()
    }

    /// Add a note, replacing any existing entry with the same ID. Call
    /// [`rebuild_backlinks`] afterwards when doing many upserts.
    pub fn upsert(&mut self, note: NoteMeta) {
        if self.by_id.contains_key(&note.id) {
            self.remove(&note.id);
        }
        self.insert_note(note);
        self.rebuild_backlinks();
    }

    /// Remove a note by ID. Returns the removed metadata, if any.
    pub fn remove(&mut self, id: &str) -> Option<NoteMeta> {
        let meta = self.by_id.remove(id)?;
        remove_from_map(&mut self.by_title, &meta.title.to_lowercase(), id);
        for alias in &meta.aliases {
            remove_from_map(&mut self.by_alias, &alias.to_lowercase(), id);
        }
        for tag in &meta.tags {
            remove_from_map_btree(&mut self.by_tag, &tag.to_lowercase(), id);
        }
        // Leave the backlink map dirty — caller should rebuild, or the
        // inconsistency is acceptable if the next op also mutates.
        self.rebuild_backlinks();
        Some(meta)
    }

    /// Find a note by its path.
    pub fn by_path(&self, path: &Path) -> Option<&NoteMeta> {
        self.by_id.values().find(|m| m.path == path)
    }

    fn insert_note(&mut self, note: NoteMeta) {
        self.by_title
            .entry(note.title.to_lowercase())
            .or_default()
            .push(note.id.clone());
        for alias in &note.aliases {
            self.by_alias
                .entry(alias.to_lowercase())
                .or_default()
                .push(note.id.clone());
        }
        for tag in &note.tags {
            self.by_tag
                .entry(tag.to_lowercase())
                .or_default()
                .push(note.id.clone());
        }
        self.by_id.insert(note.id.clone(), note);
    }

    /// Recompute the reverse-link map from every note's outgoing_links.
    /// Called at build time and after structural mutations.
    fn rebuild_backlinks(&mut self) {
        self.backlinks.clear();
        let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
        for meta in self.by_id.values() {
            for target in &meta.outgoing_links {
                if let Some(ResolveHit::Unique(id)) = self.resolve_link(target) {
                    if id != meta.id {
                        seen.entry(id.to_string()).or_default().insert(meta.id.clone());
                    }
                }
            }
        }
        for (target, sources) in seen {
            let mut v: Vec<String> = sources.into_iter().collect();
            v.sort();
            self.backlinks.insert(target, v);
        }
    }
}

fn slice_to_hit(ids: &[String]) -> ResolveHit<'_> {
    if ids.len() == 1 {
        ResolveHit::Unique(ids[0].as_str())
    } else {
        ResolveHit::Ambiguous(ids)
    }
}

fn remove_from_map(map: &mut HashMap<String, Vec<String>>, key: &str, id: &str) {
    if let Some(v) = map.get_mut(key) {
        v.retain(|i| i != id);
        if v.is_empty() {
            map.remove(key);
        }
    }
}

fn remove_from_map_btree(map: &mut BTreeMap<String, Vec<String>>, key: &str, id: &str) {
    if let Some(v) = map.get_mut(key) {
        v.retain(|i| i != id);
        if v.is_empty() {
            map.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn note(id: &str, title: &str, links: &[&str], tags: &[&str]) -> NoteMeta {
        NoteMeta {
            id: id.to_string(),
            path: PathBuf::from(format!("notes/{}.md", id)),
            title: title.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            aliases: vec![],
            status: None,
            outgoing_links: links.iter().map(|t| t.to_string()).collect(),
            is_journal: false,
        }
    }

    fn contents_with(notes: Vec<NoteMeta>) -> VaultContents {
        let mut contents = VaultContents::default();
        for n in notes {
            contents.notes.push(Note {
                id: n.id,
                path: n.path,
                title: n.title,
                tags: n.tags,
                aliases: n.aliases,
                status: n.status,
                outgoing_links: n.outgoing_links,
                mtime: None,
                is_journal: n.is_journal,
            });
        }
        contents
    }

    #[test]
    fn resolve_by_title_case_insensitive() {
        let idx = NoteIndex::build(&contents_with(vec![note("a", "Foo", &[], &[])]));
        assert!(matches!(idx.resolve_link("Foo"), Some(ResolveHit::Unique("a"))));
        assert!(matches!(idx.resolve_link("foo"), Some(ResolveHit::Unique("a"))));
        assert!(matches!(idx.resolve_link("FOO"), Some(ResolveHit::Unique("a"))));
    }

    #[test]
    fn resolve_by_id_prefix() {
        let idx = NoteIndex::build(&contents_with(vec![note("xyz", "Foo", &[], &[])]));
        assert!(matches!(idx.resolve_link("id:xyz"), Some(ResolveHit::Unique("xyz"))));
    }

    #[test]
    fn resolve_by_alias_prefers_over_title() {
        let mut n = note("a", "Canonical", &[], &[]);
        n.aliases = vec!["Old Name".to_string()];
        let idx = NoteIndex::build(&contents_with(vec![n]));
        assert!(matches!(idx.resolve_link("old name"), Some(ResolveHit::Unique("a"))));
    }

    #[test]
    fn ambiguous_title_returns_ambiguous() {
        let idx = NoteIndex::build(&contents_with(vec![
            note("a", "Dup", &[], &[]),
            note("b", "Dup", &[], &[]),
        ]));
        match idx.resolve_link("Dup") {
            Some(ResolveHit::Ambiguous(ids)) => {
                let mut sorted: Vec<&str> = ids.iter().map(String::as_str).collect();
                sorted.sort();
                assert_eq!(sorted, vec!["a", "b"]);
            }
            other => panic!("expected ambiguous, got {:?}", other),
        }
    }

    #[test]
    fn backlinks_built_from_outgoing() {
        let idx = NoteIndex::build(&contents_with(vec![
            note("a", "A", &["B", "C"], &[]),
            note("b", "B", &["C"], &[]),
            note("c", "C", &[], &[]),
        ]));
        let c_back = idx.backlinks_for("c");
        assert_eq!(c_back, &["a".to_string(), "b".to_string()]);
        let b_back = idx.backlinks_for("b");
        assert_eq!(b_back, &["a".to_string()]);
        assert!(idx.backlinks_for("a").is_empty());
    }

    #[test]
    fn self_links_are_ignored_in_backlinks() {
        let idx = NoteIndex::build(&contents_with(vec![note("a", "A", &["A"], &[])]));
        assert!(idx.backlinks_for("a").is_empty());
    }

    #[test]
    fn tags_indexed_case_insensitive() {
        let idx = NoteIndex::build(&contents_with(vec![
            note("a", "A", &[], &["Rust", "testing"]),
            note("b", "B", &[], &["RUST"]),
        ]));
        let mut ids: Vec<&str> = idx.notes_with_tag("rust").iter().map(String::as_str).collect();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);

        let tags = idx.all_tags();
        let rust = tags.iter().find(|(t, _)| t == "rust").unwrap();
        assert_eq!(rust.1, 2);
    }

    #[test]
    fn orphans_have_no_links_either_direction() {
        let idx = NoteIndex::build(&contents_with(vec![
            note("hub", "Hub", &["leaf"], &[]),
            note("leaf", "Leaf", &[], &[]),
            note("lonely", "Lonely", &[], &[]),
        ]));
        let orphans = idx.orphans();
        assert_eq!(orphans, vec!["lonely"]);
    }

    #[test]
    fn upsert_updates_in_place() {
        let mut idx = NoteIndex::build(&contents_with(vec![note("a", "Old", &[], &["x"])]));
        let mut updated = note("a", "New", &[], &["y"]);
        updated.tags = vec!["y".to_string()];
        idx.upsert(updated);

        assert!(idx.resolve_link("Old").is_none());
        assert!(matches!(idx.resolve_link("New"), Some(ResolveHit::Unique("a"))));
        assert!(idx.notes_with_tag("x").is_empty());
        assert_eq!(idx.notes_with_tag("y"), &["a".to_string()]);
    }

    #[test]
    fn remove_clears_all_keys() {
        let mut idx = NoteIndex::build(&contents_with(vec![
            note("a", "A", &["B"], &["r"]),
            note("b", "B", &[], &[]),
        ]));
        idx.remove("a");
        assert!(idx.resolve_link("A").is_none());
        assert!(idx.notes_with_tag("r").is_empty());
        assert!(idx.backlinks_for("b").is_empty());
        assert!(idx.resolve_link("B").is_some()); // b still present
    }
}
