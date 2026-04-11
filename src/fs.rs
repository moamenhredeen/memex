use std::path::{Path, PathBuf};

/// Read a markdown note from disk.
pub fn read_note(path: &Path) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}

/// Save a markdown note to disk, creating parent directories if needed.
pub fn save_note(path: &Path, content: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// Convert a title string into a filename-safe slug.
/// "My Cool Note" → "my-cool-note.md"
pub fn slugify(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else if c == ' ' || c == '_' || c == '-' {
                '-'
            } else {
                // skip non-alphanumeric, non-space chars
                '\0'
            }
        })
        .filter(|c| *c != '\0')
        .collect();

    // collapse multiple hyphens
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen && !result.is_empty() {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // trim trailing hyphen
    let trimmed = result.trim_end_matches('-');
    format!("{}.md", trimmed)
}

/// Extract a display title from a note path.
/// Uses the file stem (e.g. "my-note.md" → "my-note").
pub fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

/// List all .md files in a directory (non-recursive for now).
pub fn list_notes(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut notes = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" {
                        notes.push(path);
                    }
                }
            }
        }
    }
    notes.sort();
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("My Cool Note"), "my-cool-note.md");
        assert_eq!(slugify("hello world"), "hello-world.md");
        assert_eq!(slugify("Already-Slug"), "already-slug.md");
        assert_eq!(slugify("  spaces  "), "spaces.md");
        assert_eq!(slugify("special!@#chars"), "specialchars.md");
    }

    #[test]
    fn test_title_from_path() {
        assert_eq!(title_from_path(Path::new("/notes/my-note.md")), "my-note");
        assert_eq!(title_from_path(Path::new("untitled.md")), "untitled");
    }

    #[test]
    fn test_read_write_note() {
        let dir = std::env::temp_dir().join("memex-test-rw");
        let path = dir.join("test-note.md");

        save_note(&path, "# Hello\nWorld").unwrap();
        let content = read_note(&path).unwrap();
        assert_eq!(content, "# Hello\nWorld");

        // cleanup
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_list_notes() {
        let dir = std::env::temp_dir().join("memex-test-list");
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("a.md"), "note a").unwrap();
        fs::write(dir.join("b.md"), "note b").unwrap();
        fs::write(dir.join("c.txt"), "not a note").unwrap();

        let notes = list_notes(&dir).unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|p| p.extension().unwrap() == "md"));

        fs::remove_dir_all(&dir).ok();
    }
}
