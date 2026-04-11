use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    Body,
}

/// Style flags for a text span within a line.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanStyle {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link_url: Option<String>,
}

impl SpanStyle {
    fn plain(text: String) -> Self {
        Self {
            text,
            bold: false,
            italic: false,
            strikethrough: false,
            code: false,
            link_url: None,
        }
    }
}

/// A parsed line with heading level and styled spans.
#[derive(Debug, Clone, PartialEq)]
pub struct StyledLine {
    pub level: HeadingLevel,
    /// The heading marker (e.g. "# "), empty for body lines.
    pub marker: String,
    /// Styled text spans for rendering.
    pub spans: Vec<SpanStyle>,
    /// The original raw line text.
    pub raw: String,
}

/// Parse a single line into a StyledLine with inline formatting.
pub fn parse_line(line: &str) -> StyledLine {
    // Detect heading level from raw markdown prefix
    let (level, marker) = detect_heading(line);

    // Strip the marker for pulldown-cmark parsing
    let content = &line[marker.len()..];

    if content.is_empty() {
        return StyledLine {
            level,
            marker,
            spans: vec![],
            raw: line.to_string(),
        };
    }

    let spans = parse_inline(content);

    StyledLine {
        level,
        marker,
        spans,
        raw: line.to_string(),
    }
}

/// Detect heading level from line prefix, returning (level, marker_string).
fn detect_heading(line: &str) -> (HeadingLevel, String) {
    // Count leading '#' followed by space
    let trimmed = line.trim_start();
    let hash_count = trimmed.bytes().take_while(|&b| b == b'#').count();

    if hash_count > 0 && hash_count <= 6 && trimmed.as_bytes().get(hash_count) == Some(&b' ') {
        let level = match hash_count {
            1 => HeadingLevel::H1,
            2 => HeadingLevel::H2,
            3 => HeadingLevel::H3,
            4 => HeadingLevel::H4,
            5 => HeadingLevel::H5,
            _ => HeadingLevel::H6,
        };
        // Marker includes leading whitespace + hashes + space
        let prefix_ws = line.len() - trimmed.len();
        let marker = line[..prefix_ws + hash_count + 1].to_string();
        (level, marker)
    } else {
        (HeadingLevel::Body, String::new())
    }
}

/// Parse inline markdown (bold, italic, code, strikethrough, links) into styled spans.
fn parse_inline(text: &str) -> Vec<SpanStyle> {
    let opts = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(text, opts);

    let mut spans = Vec::new();
    let mut bold = false;
    let mut italic = false;
    let mut strikethrough = false;
    let mut code = false;
    let mut link_url: Option<String> = None;

    for event in parser {
        match &event {
            Event::Text(t) => {
                spans.push(SpanStyle {
                    text: t.to_string(),
                    bold,
                    italic,
                    strikethrough,
                    code,
                    link_url: link_url.clone(),
                });
            }
            Event::Code(t) => {
                spans.push(SpanStyle {
                    text: t.to_string(),
                    bold,
                    italic,
                    strikethrough,
                    code: true,
                    link_url: link_url.clone(),
                });
            }
            Event::SoftBreak | Event::HardBreak => {
                spans.push(SpanStyle::plain(" ".to_string()));
            }
            Event::Start(tag) => match tag {
                Tag::Strong => bold = true,
                Tag::Emphasis => italic = true,
                Tag::Strikethrough => strikethrough = true,
                Tag::CodeBlock(_) => code = true,
                Tag::Link { dest_url, .. } => {
                    link_url = Some(dest_url.to_string());
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Strong => bold = false,
                TagEnd::Emphasis => italic = false,
                TagEnd::Strikethrough => strikethrough = false,
                TagEnd::CodeBlock => code = false,
                TagEnd::Link => link_url = None,
                _ => {}
            },
            _ => {}
        }
    }

    spans
}

/// Parse a multi-line string into styled lines.
pub fn parse(text: &str) -> Vec<StyledLine> {
    text.lines().map(parse_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_headings() {
        let input = "# Hello\n## World\n### Sub\nBody text";
        let lines = parse(input);

        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].level, HeadingLevel::H1);
        assert_eq!(lines[0].marker, "# ");
        assert_eq!(lines[0].spans[0].text, "Hello");
        assert_eq!(lines[1].level, HeadingLevel::H2);
        assert_eq!(lines[2].level, HeadingLevel::H3);
        assert_eq!(lines[3].level, HeadingLevel::Body);
        assert_eq!(lines[3].spans[0].text, "Body text");
        assert_eq!(lines[3].marker, "");
    }

    #[test]
    fn test_no_space_after_hash_is_body() {
        let lines = parse("#NoSpace");
        assert_eq!(lines[0].level, HeadingLevel::Body);
    }

    #[test]
    fn test_empty_lines() {
        let lines = parse("# Title\n\nBody");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].level, HeadingLevel::Body);
        assert!(lines[1].spans.is_empty());
    }

    #[test]
    fn test_bold_italic() {
        let line = parse_line("**bold** and *italic*");
        assert_eq!(line.level, HeadingLevel::Body);
        assert_eq!(line.spans.len(), 3);
        assert!(line.spans[0].bold);
        assert_eq!(line.spans[0].text, "bold");
        assert!(!line.spans[1].bold);
        assert_eq!(line.spans[1].text, " and ");
        assert!(line.spans[2].italic);
        assert_eq!(line.spans[2].text, "italic");
    }

    #[test]
    fn test_inline_code() {
        let line = parse_line("use `println!` here");
        assert_eq!(line.spans.len(), 3);
        assert!(!line.spans[0].code);
        assert_eq!(line.spans[0].text, "use ");
        assert!(line.spans[1].code);
        assert_eq!(line.spans[1].text, "println!");
        assert!(!line.spans[2].code);
    }

    #[test]
    fn test_strikethrough() {
        let line = parse_line("~~deleted~~ text");
        assert_eq!(line.spans.len(), 2);
        assert!(line.spans[0].strikethrough);
        assert_eq!(line.spans[0].text, "deleted");
        assert!(!line.spans[1].strikethrough);
    }

    #[test]
    fn test_link() {
        let line = parse_line("click [here](https://example.com) now");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].text, "click ");
        assert!(line.spans[1].link_url.is_some());
        assert_eq!(line.spans[1].text, "here");
        assert_eq!(
            line.spans[1].link_url.as_deref(),
            Some("https://example.com")
        );
        assert!(line.spans[2].link_url.is_none());
    }

    #[test]
    fn test_nested_bold_italic() {
        let line = parse_line("***bold italic***");
        assert_eq!(line.spans.len(), 1);
        assert!(line.spans[0].bold);
        assert!(line.spans[0].italic);
    }

    #[test]
    fn test_heading_with_inline() {
        let line = parse_line("## A **bold** heading");
        assert_eq!(line.level, HeadingLevel::H2);
        assert_eq!(line.marker, "## ");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].text, "A ");
        assert!(line.spans[1].bold);
        assert_eq!(line.spans[1].text, "bold");
        assert_eq!(line.spans[2].text, " heading");
    }
}
