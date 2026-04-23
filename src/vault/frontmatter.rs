//! YAML frontmatter parser and writer for notes.
//!
//! A note may optionally start with a YAML block delimited by `---`:
//! ```text
//! ---
//! id: 20260423T142301-k3n8
//! title: Unit testing
//! created: 2026-04-23T14:23:01Z
//! tags: [testing, engineering]
//! ---
//! # Body content here
//! ```
//!
//! The parser splits a note into `(Frontmatter, body)`, and the writer
//! serialises a `Frontmatter` back to the exact block shape. Unknown
//! keys are preserved via a catch-all `extra` field so round-trips
//! don't drop metadata set by other tools.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The canonical frontmatter schema for Memex notes.
///
/// Any YAML keys outside these named fields are preserved in `extra`
/// so other tools' metadata (e.g. Obsidian's `cssclasses`, plugins'
/// custom fields) survives round-trips through Memex.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    /// Stable identifier for this note. Immutable — referenced by
    /// `[[id:...]]` wikilinks and never changes after creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Human-readable title. The filename does NOT have to match —
    /// renaming the title is a frontmatter edit, no file rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// ISO-8601 creation timestamp (set once, at note creation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,

    /// Free-form status (e.g. `draft`, `active`, `archived`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Flat list of tags. Nested tags aren't supported — use multiple
    /// tags instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Alternate names. Wikilinks resolve against aliases, so renaming
    /// a title can preserve old links by appending the old title here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,

    /// Any other YAML keys — preserved verbatim on round-trip.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

/// Outcome of splitting a note into frontmatter + body.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedNote {
    pub frontmatter: Option<Frontmatter>,
    /// The rest of the file after the frontmatter block. The leading
    /// newline after the closing `---` is consumed so the body starts
    /// with real content.
    pub body: String,
}

/// Parse a note. Returns `frontmatter: None` when the file has no
/// frontmatter block, `Some` when it does. Invalid YAML inside a
/// well-formed `---` block is returned as an error so the caller
/// can decide whether to surface it to the user or treat the whole
/// file as body.
pub fn parse(content: &str) -> Result<ParsedNote, serde_yaml::Error> {
    let Some((yaml, body)) = split_frontmatter_block(content) else {
        return Ok(ParsedNote {
            frontmatter: None,
            body: content.to_string(),
        });
    };

    let frontmatter = serde_yaml::from_str::<Frontmatter>(yaml)?;
    Ok(ParsedNote {
        frontmatter: Some(frontmatter),
        body,
    })
}

/// Serialise a note: frontmatter block (if present) followed by body.
/// Emits the block only when the frontmatter has at least one field set.
pub fn write(frontmatter: &Frontmatter, body: &str) -> Result<String, serde_yaml::Error> {
    if is_empty_frontmatter(frontmatter) {
        return Ok(body.to_string());
    }
    let yaml = serde_yaml::to_string(frontmatter)?;
    // serde_yaml emits a trailing newline already; normalise so we
    // always produce exactly `---\n{yaml}---\n{body}`.
    let yaml = yaml.trim_end().to_string();
    Ok(format!("---\n{}\n---\n{}", yaml, body))
}

fn is_empty_frontmatter(f: &Frontmatter) -> bool {
    f.id.is_none()
        && f.title.is_none()
        && f.created.is_none()
        && f.status.is_none()
        && f.tags.is_empty()
        && f.aliases.is_empty()
        && f.extra.is_empty()
}

/// Find the YAML block bounded by `---` fences at the very start of the
/// content. Returns `(yaml_body, rest_of_file)`.
///
/// Rules (matching pulldown-cmark's `ENABLE_YAML_STYLE_METADATA_BLOCKS`):
/// - Opening `---` must be the first line of the file.
/// - Closing `---` must sit on its own line.
/// - No trailing whitespace on either fence.
fn split_frontmatter_block(content: &str) -> Option<(&str, String)> {
    let rest = content.strip_prefix("---\n").or_else(|| content.strip_prefix("---\r\n"))?;
    // Find the next line that is exactly `---`.
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            let yaml = &rest[..offset];
            let body_start = offset + line.len();
            let body = rest[body_start..].to_string();
            return Some((yaml, body));
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_frontmatter() {
        let result = parse("# Hello\nWorld").unwrap();
        assert!(result.frontmatter.is_none());
        assert_eq!(result.body, "# Hello\nWorld");
    }

    #[test]
    fn parse_basic_frontmatter() {
        let content = "---\ntitle: Test\ntags: [a, b]\n---\n# Body\n";
        let result = parse(content).unwrap();
        let fm = result.frontmatter.unwrap();
        assert_eq!(fm.title.as_deref(), Some("Test"));
        assert_eq!(fm.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(result.body, "# Body\n");
    }

    #[test]
    fn parse_preserves_unknown_keys() {
        let content = "---\ntitle: T\ncustom_field: hello\n---\nbody";
        let result = parse(content).unwrap();
        let fm = result.frontmatter.unwrap();
        assert!(fm.extra.contains_key("custom_field"));
    }

    #[test]
    fn write_emits_frontmatter_when_populated() {
        let mut fm = Frontmatter::default();
        fm.id = Some("20260423T142301-k3n8".to_string());
        fm.title = Some("Test".to_string());
        fm.tags = vec!["a".to_string()];

        let out = write(&fm, "# Body\n").unwrap();
        assert!(out.starts_with("---\n"));
        assert!(out.contains("id: 20260423T142301-k3n8"));
        assert!(out.contains("title: Test"));
        assert!(out.contains("tags:"));
        assert!(out.ends_with("---\n# Body\n"));
    }

    #[test]
    fn write_empty_frontmatter_is_noop() {
        let fm = Frontmatter::default();
        let out = write(&fm, "# Body\n").unwrap();
        assert_eq!(out, "# Body\n");
    }

    #[test]
    fn roundtrip_preserves_unknown_keys() {
        let content = "---\ntitle: T\ncustom: value\nnum: 42\n---\nbody";
        let parsed = parse(content).unwrap();
        let fm = parsed.frontmatter.unwrap();
        let out = write(&fm, &parsed.body).unwrap();
        let reparsed = parse(&out).unwrap().frontmatter.unwrap();
        assert_eq!(reparsed.title, fm.title);
        assert_eq!(reparsed.extra, fm.extra);
    }

    #[test]
    fn parse_unclosed_block_is_not_frontmatter() {
        // No closing `---` — treat as plain body.
        let content = "---\ntitle: T\nno closer here\n";
        let result = parse(content).unwrap();
        assert!(result.frontmatter.is_none());
        assert_eq!(result.body, content);
    }

    #[test]
    fn parse_fence_not_at_start_is_not_frontmatter() {
        let content = "# Heading\n---\ntitle: T\n---\n";
        let result = parse(content).unwrap();
        // `---` after a heading is a thematic break, not frontmatter.
        assert!(result.frontmatter.is_none());
    }

    #[test]
    fn parse_empty_frontmatter_block() {
        let content = "---\n---\n# Body\n";
        let result = parse(content).unwrap();
        assert!(result.frontmatter.is_some());
        assert_eq!(result.body, "# Body\n");
    }

    #[test]
    fn parse_crlf_line_endings() {
        let content = "---\r\ntitle: T\r\n---\r\n# Body\r\n";
        let result = parse(content).unwrap();
        let fm = result.frontmatter.unwrap();
        assert_eq!(fm.title.as_deref(), Some("T"));
    }
}
