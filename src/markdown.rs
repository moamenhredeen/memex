#[derive(Debug, Clone, PartialEq)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
    Body,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyledLine {
    pub level: HeadingLevel,
    /// The heading marker (e.g. "# ", "## "), empty for body lines
    pub marker: String,
    /// The content after stripping the heading marker
    pub content: String,
    /// The original full line text
    pub raw: String,
}

/// Parse a single line into a StyledLine.
pub fn parse_line(line: &str) -> StyledLine {
    if let Some(content) = line.strip_prefix("### ") {
        StyledLine {
            level: HeadingLevel::H3,
            marker: "### ".to_string(),
            content: content.to_string(),
            raw: line.to_string(),
        }
    } else if let Some(content) = line.strip_prefix("## ") {
        StyledLine {
            level: HeadingLevel::H2,
            marker: "## ".to_string(),
            content: content.to_string(),
            raw: line.to_string(),
        }
    } else if let Some(content) = line.strip_prefix("# ") {
        StyledLine {
            level: HeadingLevel::H1,
            marker: "# ".to_string(),
            content: content.to_string(),
            raw: line.to_string(),
        }
    } else {
        StyledLine {
            level: HeadingLevel::Body,
            marker: String::new(),
            content: line.to_string(),
            raw: line.to_string(),
        }
    }
}

/// Parse a multi-line string into styled lines, detecting markdown headings.
pub fn parse(text: &str) -> Vec<StyledLine> {
    text.lines().map(|line| parse_line(line)).collect()
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
        assert_eq!(lines[0].content, "Hello");
        assert_eq!(lines[0].marker, "# ");
        assert_eq!(lines[1].level, HeadingLevel::H2);
        assert_eq!(lines[1].content, "World");
        assert_eq!(lines[2].level, HeadingLevel::H3);
        assert_eq!(lines[2].content, "Sub");
        assert_eq!(lines[3].level, HeadingLevel::Body);
        assert_eq!(lines[3].content, "Body text");
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
        assert_eq!(lines[1].content, "");
    }
}
