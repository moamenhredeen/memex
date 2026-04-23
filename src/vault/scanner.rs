//! Scan a vault directory and produce structured notes / journal /
//! attachment lists.
//!
//! Replaces the old `fs::list_vault_files` which lumped every file
//! together regardless of purpose. The scanner understands the
//! vault layout (see `super::layout`):
//! - `notes/` and `journal/` + any top-level `.md` are scanned for
//!   markdown notes and have their frontmatter parsed.
//! - `attachments/` is scanned for opaque files (PDFs, images).
//! - `.memex/`, `.git/`, and any other hidden dir is skipped.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::frontmatter::{self, Frontmatter};
use super::layout::{ATTACHMENTS_DIR, JOURNAL_DIR, NOTES_DIR, VaultLayout};

/// A markdown note with parsed frontmatter and extracted outgoing
/// wikilinks / inline tags. Notes live under `notes/`, `journal/`,
/// or legacy at the vault root.
#[derive(Debug, Clone)]
pub struct Note {
    /// Canonical ID. Taken from `frontmatter.id` when present, falling
    /// back to the filename stem. Stable across renames once frontmatter
    /// exists.
    pub id: String,
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Display title — `frontmatter.title`, then the filename stem.
    pub title: String,
    /// Union of frontmatter `tags` and inline `#tags` in the body.
    pub tags: Vec<String>,
    /// Alternate names from frontmatter.
    pub aliases: Vec<String>,
    /// Free-form status from frontmatter (`draft`, `active`, etc.).
    pub status: Option<String>,
    /// Wikilink targets found in the body. Each entry is the raw target
    /// as written — e.g. `Other Note`, `id:20260423T142301-k3n8`, or
    /// an alias. Resolution happens in the index layer.
    pub outgoing_links: Vec<String>,
    /// File modification time when last scanned.
    pub mtime: Option<SystemTime>,
    /// True when this note lives under `journal/`.
    pub is_journal: bool,
}

/// Result of a vault scan.
#[derive(Debug, Clone, Default)]
pub struct VaultContents {
    pub notes: Vec<Note>,
    pub journal: Vec<Note>,
    pub attachments: Vec<PathBuf>,
}

/// Scan a vault. Parses frontmatter of every markdown file; skips
/// `.memex/` and hidden directories.
pub fn scan(layout: &VaultLayout) -> io::Result<VaultContents> {
    let mut contents = VaultContents::default();

    // notes/
    if layout.notes.is_dir() {
        collect_notes(&layout.notes, false, &mut contents.notes)?;
    }

    // journal/
    if layout.journal.is_dir() {
        collect_notes(&layout.journal, true, &mut contents.journal)?;
    }

    // attachments/
    if layout.attachments.is_dir() {
        collect_attachments(&layout.attachments, &mut contents.attachments)?;
    }

    // Legacy: .md files at the vault root (pre-layout). Treat them as notes.
    if layout.root.is_dir() {
        for entry in std::fs::read_dir(&layout.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && has_md_extension(&path) {
                if let Some(note) = read_note(&path, false) {
                    contents.notes.push(note);
                }
            }
        }
    }

    // Deterministic order for stable tests and predictable UI.
    contents.notes.sort_by(|a, b| a.id.cmp(&b.id));
    contents.journal.sort_by(|a, b| a.id.cmp(&b.id));
    contents.attachments.sort();

    Ok(contents)
}

fn collect_notes(root: &Path, is_journal: bool, out: &mut Vec<Note>) -> io::Result<()> {
    walk_dir(root, &mut |path| {
        if has_md_extension(path) {
            if let Some(note) = read_note(path, is_journal) {
                out.push(note);
            }
        }
        Ok(())
    })
}

fn collect_attachments(root: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    walk_dir(root, &mut |path| {
        out.push(path.to_path_buf());
        Ok(())
    })
}

/// Walk a directory tree, calling `visit` for every regular file.
/// Skips hidden entries (names starting with `.`). Uses an explicit
/// stack so deeply-nested trees don't blow the Rust stack.
fn walk_dir(root: &Path, visit: &mut dyn FnMut(&Path) -> io::Result<()>) -> io::Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                visit(&path)?;
            }
        }
    }
    Ok(())
}

fn has_md_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

/// Read and parse a single note. Returns `None` on I/O or UTF-8 errors —
/// malformed files shouldn't crash the scan; they just don't appear in
/// the index. (A later pass can surface a warning to the user.)
pub fn read_note(path: &Path, is_journal: bool) -> Option<Note> {
    let content = std::fs::read_to_string(path).ok()?;
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());

    let (fm, body) = match frontmatter::parse(&content) {
        Ok(parsed) => (parsed.frontmatter.unwrap_or_default(), parsed.body),
        // Malformed frontmatter: treat the whole file as body with no frontmatter.
        // This way one bad file doesn't hide itself from the note list.
        Err(_) => (Frontmatter::default(), content.clone()),
    };

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    let id = fm.id.clone().unwrap_or_else(|| stem.clone());
    let title = fm.title.clone().unwrap_or_else(|| stem.clone());

    let mut tags = fm.tags.clone();
    for t in extract_inline_tags(&body) {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }

    let outgoing_links = extract_wikilinks(&body);

    Some(Note {
        id,
        path: path.to_path_buf(),
        title,
        tags,
        aliases: fm.aliases,
        status: fm.status,
        outgoing_links,
        mtime,
        is_journal,
    })
}

/// Extract `[[target]]` targets from note body. Handles `[[target|alias]]`
/// by returning just the target, and `[[id:...]]` by preserving the
/// `id:` prefix so the index layer can route differently.
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            // Find closing ]] on the same line (no newline crossing).
            let mut j = start;
            let mut found = None;
            while j + 1 < bytes.len() {
                let b = bytes[j];
                if b == b'\n' {
                    break;
                }
                if b == b']' && bytes[j + 1] == b']' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                // `[[target|display]]` → take `target`; `[[target]]` → whole.
                let inner = &body[start..end];
                let target = inner
                    .split_once('|')
                    .map(|(t, _)| t)
                    .unwrap_or(inner)
                    .trim();
                if !target.is_empty() {
                    out.push(target.to_string());
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Extract `#tag` occurrences from body. Matches `#` followed by one or
/// more ASCII alphanumerics / `_` / `-`, not inside code spans or URLs.
/// Imperfect heuristic — good enough for Phase 1; can be tightened when
/// the index is in place.
pub fn extract_inline_tags(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in body.lines() {
        // Skip fenced code blocks entirely.
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        extract_tags_from_line(line, &mut out);
    }
    // De-dupe preserving first occurrence.
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.clone()));
    out
}

fn extract_tags_from_line(line: &str, out: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            // Must be at line start or preceded by whitespace.
            let boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
            if boundary {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && is_tag_char(bytes[j]) {
                    j += 1;
                }
                if j > start {
                    let tag = &line[start..j];
                    // Require at least one non-digit so `#1` (a section
                    // anchor or shorthand) doesn't become a tag. A
                    // markdown heading `# Foo` is naturally rejected
                    // because the char after `#` is a space, failing
                    // the tag-char loop before it produces anything.
                    if tag.bytes().any(|b| !b.is_ascii_digit()) {
                        out.push(tag.to_string());
                    }
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
}

fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_wikilinks_basic() {
        let links = extract_wikilinks("see [[Foo]] and [[Bar]] here");
        assert_eq!(links, vec!["Foo", "Bar"]);
    }

    #[test]
    fn extract_wikilinks_with_alias() {
        let links = extract_wikilinks("[[Target|Display]]");
        assert_eq!(links, vec!["Target"]);
    }

    #[test]
    fn extract_wikilinks_id_preserved() {
        let links = extract_wikilinks("jump to [[id:20260423T142301-k3n8]]");
        assert_eq!(links, vec!["id:20260423T142301-k3n8"]);
    }

    #[test]
    fn extract_wikilinks_ignores_unclosed() {
        let links = extract_wikilinks("typing [[partial");
        assert!(links.is_empty());
    }

    #[test]
    fn extract_wikilinks_ignores_newline_within() {
        let links = extract_wikilinks("broken [[start\nend]] link");
        assert!(links.is_empty());
    }

    #[test]
    fn extract_tags_simple() {
        let tags = extract_inline_tags("tagged #testing and #engineering here");
        assert_eq!(tags, vec!["testing", "engineering"]);
    }

    #[test]
    fn extract_tags_line_start() {
        let tags = extract_inline_tags("#draft at start");
        assert_eq!(tags, vec!["draft"]);
    }

    #[test]
    fn extract_tags_ignores_headings() {
        let tags = extract_inline_tags("# Heading 1\n## Also heading\nbody text");
        assert!(tags.is_empty(), "headings must not be tags, got {:?}", tags);
    }

    #[test]
    fn extract_tags_ignores_code_blocks() {
        let body = "normal #ok\n```\n#not-a-tag\n```\nafter #also-ok";
        let tags = extract_inline_tags(body);
        assert_eq!(tags, vec!["ok", "also-ok"]);
    }

    #[test]
    fn extract_tags_dedupes() {
        let tags = extract_inline_tags("#one #one #two #two #one");
        assert_eq!(tags, vec!["one", "two"]);
    }

    #[test]
    fn scanner_reads_notes_journal_attachments() {
        use crate::vault::layout::VaultLayout;

        let root = std::env::temp_dir().join("memex-scanner-test");
        let _ = std::fs::remove_dir_all(&root);
        let layout = VaultLayout::at(&root);
        layout.ensure().unwrap();

        std::fs::write(
            layout.notes.join("20260423T142301-k3n8.md"),
            "---\ntitle: First\ntags: [a]\n---\nbody with [[Other]] link #inline",
        ).unwrap();
        std::fs::write(
            layout.notes.join("legacy-note.md"),
            "no frontmatter here",
        ).unwrap();
        std::fs::write(
            layout.journal.join("2026-04-23.md"),
            "---\ntitle: Daily\n---\ndaily log",
        ).unwrap();
        std::fs::write(layout.attachments.join("diagram.pdf"), "%PDF-fake").unwrap();

        let contents = scan(&layout).unwrap();

        assert_eq!(contents.notes.len(), 2);
        let first = contents.notes.iter().find(|n| n.title == "First").unwrap();
        assert_eq!(first.tags, vec!["a", "inline"]);
        assert_eq!(first.outgoing_links, vec!["Other"]);

        let legacy = contents.notes.iter().find(|n| n.title == "legacy-note").unwrap();
        assert!(legacy.tags.is_empty());

        assert_eq!(contents.journal.len(), 1);
        assert!(contents.journal[0].is_journal);

        assert_eq!(contents.attachments.len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scanner_skips_hidden_and_memex_dir() {
        use crate::vault::layout::VaultLayout;

        let root = std::env::temp_dir().join("memex-scanner-hidden");
        let _ = std::fs::remove_dir_all(&root);
        let layout = VaultLayout::at(&root);
        layout.ensure().unwrap();

        // Files in .memex/ and a hand-rolled hidden .git/ must be ignored.
        std::fs::write(layout.dot_memex.join("cache.md"), "shouldn't show").unwrap();
        let git = root.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD.md"), "nope").unwrap();

        std::fs::write(layout.notes.join("real.md"), "content").unwrap();

        let contents = scan(&layout).unwrap();
        assert_eq!(contents.notes.len(), 1);
        assert_eq!(contents.notes[0].title, "real");

        std::fs::remove_dir_all(&root).ok();
    }
}
