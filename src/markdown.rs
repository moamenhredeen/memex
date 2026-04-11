use std::ops::Range;

#[derive(Clone, Debug)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub kind: StyleKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StyleKind {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
    Strikethrough,
    HeadingSyntax,
    CodeFence,
    HrSyntax,
    ListBullet,
    TableSyntax,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineKind {
    Normal,
    Heading(u8),
    CodeBlock,
    ThematicBreak,
    ListItem,
    TableRow,
}

pub struct LineInfo {
    pub kind: LineKind,
    pub spans: Vec<StyleSpan>,
}

pub fn analyze_line(line: &str, in_code_block: &mut bool) -> LineInfo {
    let trimmed = line.trim();

    if trimmed.starts_with("```") {
        *in_code_block = !*in_code_block;
        return LineInfo {
            kind: LineKind::CodeBlock,
            spans: vec![StyleSpan {
                range: 0..line.len().max(1),
                kind: StyleKind::CodeFence,
            }],
        };
    }

    if *in_code_block {
        return LineInfo {
            kind: LineKind::CodeBlock,
            spans: vec![StyleSpan {
                range: 0..line.len().max(1),
                kind: StyleKind::Code,
            }],
        };
    }

    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return LineInfo {
            kind: LineKind::ThematicBreak,
            spans: vec![StyleSpan {
                range: 0..line.len().max(1),
                kind: StyleKind::HrSyntax,
            }],
        };
    }

    for level in (1u8..=6).rev() {
        let prefix = "#".repeat(level as usize);
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            if rest.is_empty() || rest.starts_with(' ') {
                return heading_line_info(line, level);
            }
        }
    }

    if let Some(bullet_end) = detect_list_prefix(trimmed) {
        let leading_ws = line.len() - line.trim_start().len();
        let prefix_end = leading_ws + bullet_end;
        return list_line_info(line, prefix_end);
    }

    if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
        return table_line_info(line);
    }

    let spans = parse_inline_styles(line);
    LineInfo {
        kind: LineKind::Normal,
        spans,
    }
}

pub fn detect_list_prefix(trimmed: &str) -> Option<usize> {
    if trimmed.starts_with("- [ ] ")
        || trimmed.starts_with("- [x] ")
        || trimmed.starts_with("- [X] ")
    {
        return Some(6);
    }
    if (trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ "))
        && trimmed.len() > 2
    {
        return Some(2);
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() - 1 && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        return Some(i + 2);
    }
    None
}

fn list_line_info(line: &str, prefix_end: usize) -> LineInfo {
    let mut spans = vec![StyleSpan {
        range: 0..prefix_end,
        kind: StyleKind::ListBullet,
    }];
    if prefix_end < line.len() {
        let content_spans = parse_inline_styles(&line[prefix_end..]);
        for mut s in content_spans {
            s.range = (s.range.start + prefix_end)..(s.range.end + prefix_end);
            spans.push(s);
        }
    }
    LineInfo {
        kind: LineKind::ListItem,
        spans,
    }
}

fn table_line_info(line: &str) -> LineInfo {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut normal_start = 0;

    while i < bytes.len() {
        if bytes[i] == b'|' {
            if i > normal_start {
                spans.push(StyleSpan {
                    range: normal_start..i,
                    kind: StyleKind::Normal,
                });
            }
            spans.push(StyleSpan {
                range: i..i + 1,
                kind: StyleKind::TableSyntax,
            });
            normal_start = i + 1;
        }
        i += 1;
    }
    if normal_start < line.len() {
        spans.push(StyleSpan {
            range: normal_start..line.len(),
            kind: StyleKind::Normal,
        });
    }
    if spans.is_empty() {
        spans.push(StyleSpan {
            range: 0..line.len().max(1),
            kind: StyleKind::Normal,
        });
    }
    LineInfo {
        kind: LineKind::TableRow,
        spans,
    }
}

fn heading_line_info(line: &str, level: u8) -> LineInfo {
    let prefix_end = line.find(' ').map(|i| i + 1).unwrap_or(line.len());
    let mut spans = vec![StyleSpan {
        range: 0..prefix_end,
        kind: StyleKind::HeadingSyntax,
    }];
    if prefix_end < line.len() {
        let content_spans = parse_inline_styles(&line[prefix_end..]);
        for mut s in content_spans {
            s.range = (s.range.start + prefix_end)..(s.range.end + prefix_end);
            spans.push(s);
        }
    }
    LineInfo {
        kind: LineKind::Heading(level),
        spans,
    }
}

pub fn parse_inline_styles(text: &str) -> Vec<StyleSpan> {
    if text.is_empty() {
        return vec![StyleSpan {
            range: 0..0,
            kind: StyleKind::Normal,
        }];
    }

    let mut spans = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut normal_start = 0;

    while i < len {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        if bytes[i] == b'`' {
            if let Some(end) = find_closing(text, i + 1, "`") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 1,
                    kind: StyleKind::Code,
                });
                i = end + 1;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'*' && i + 2 < len && bytes[i + 1] == b'*' && bytes[i + 2] == b'*' {
            if let Some(end) = find_closing(text, i + 3, "***") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 3,
                    kind: StyleKind::BoldItalic,
                });
                i = end + 3;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'*' {
            if let Some(end) = find_closing(text, i + 2, "**") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 2,
                    kind: StyleKind::Bold,
                });
                i = end + 2;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'*' {
            if let Some(end) = find_closing(text, i + 1, "*") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 1,
                    kind: StyleKind::Italic,
                });
                i = end + 1;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'~' && i + 1 < len && bytes[i + 1] == b'~' {
            if let Some(end) = find_closing(text, i + 2, "~~") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 2,
                    kind: StyleKind::Strikethrough,
                });
                i = end + 2;
                normal_start = i;
                continue;
            }
        }
        i += 1;
    }

    push_normal(&mut spans, normal_start, len);

    if spans.is_empty() {
        spans.push(StyleSpan {
            range: 0..text.len(),
            kind: StyleKind::Normal,
        });
    }

    spans
}

fn push_normal(spans: &mut Vec<StyleSpan>, start: usize, end: usize) {
    if end > start {
        spans.push(StyleSpan {
            range: start..end,
            kind: StyleKind::Normal,
        });
    }
}

fn find_closing(text: &str, start: usize, delimiter: &str) -> Option<usize> {
    if start >= text.len() {
        return None;
    }
    text[start..].find(delimiter).map(|pos| start + pos)
}

pub fn parse_table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner.split('|').map(|s| s.to_string()).collect()
}

pub fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return false;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    inner
        .split('|')
        .all(|cell| cell.trim().chars().all(|c| c == '-' || c == ':' || c == ' '))
}

/// Compute column widths from table rows (minimum 3).
pub fn compute_col_widths(rows: &[Vec<String>], is_separator: &[bool]) -> Vec<usize> {
    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![3usize; max_cols];
    for (ri, row) in rows.iter().enumerate() {
        if is_separator[ri] {
            continue;
        }
        for (ci, cell) in row.iter().enumerate() {
            widths[ci] = widths[ci].max(cell.trim().len());
        }
    }
    widths
}

/// Format a table as aligned markdown text.
pub fn format_table(
    rows: &[Vec<String>],
    is_separator: &[bool],
    col_widths: &[usize],
) -> String {
    let mut out = String::new();
    for (ri, row) in rows.iter().enumerate() {
        out.push('|');
        for (ci, cell) in row.iter().enumerate() {
            let w = col_widths.get(ci).copied().unwrap_or(3);
            if is_separator[ri] {
                out.push(' ');
                for _ in 0..w {
                    out.push('-');
                }
                out.push(' ');
            } else {
                let content = cell.trim();
                out.push(' ');
                out.push_str(content);
                for _ in content.len()..w {
                    out.push(' ');
                }
                out.push(' ');
            }
            out.push('|');
        }
        if ri < rows.len() - 1 {
            out.push('\n');
        }
    }
    out
}

/// Calculate cursor byte offset within a formatted table for a given cell.
/// Places cursor at end of cell content.
pub fn cursor_pos_in_formatted_table(
    target_row: usize,
    target_col: usize,
    rows: &[Vec<String>],
    col_widths: &[usize],
    is_separator: &[bool],
) -> usize {
    let mut pos = 0;
    for ri in 0..target_row {
        pos += 1; // leading |
        for ci in 0..rows[ri].len() {
            let w = col_widths.get(ci).copied().unwrap_or(3);
            pos += 1 + w + 1 + 1; // space + width + space + |
        }
        pos += 1; // newline
    }
    pos += 1; // leading | of target row
    for ci in 0..target_col {
        let w = col_widths.get(ci).copied().unwrap_or(3);
        pos += 1 + w + 1 + 1;
    }
    pos += 1; // space after |
    if !is_separator[target_row] {
        pos += rows[target_row][target_col].trim().len();
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_table_cells() {
        let cells = parse_table_cells("| Name | Role | Status |");
        assert_eq!(cells, vec![" Name ", " Role ", " Status "]);
    }

    #[test]
    fn test_parse_table_cells_no_spaces() {
        let cells = parse_table_cells("|a|b|c|");
        assert_eq!(cells, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_is_separator_row_valid() {
        assert!(is_separator_row("|------|------|--------|"));
        assert!(is_separator_row("| --- | --- |"));
        assert!(is_separator_row("|:---:|:---|---:|"));
    }

    #[test]
    fn test_is_separator_row_invalid() {
        assert!(!is_separator_row("| Name | Role |"));
        assert!(!is_separator_row("not a table"));
    }

    #[test]
    fn test_detect_list_prefix_unordered() {
        assert_eq!(detect_list_prefix("- item"), Some(2));
        assert_eq!(detect_list_prefix("* item"), Some(2));
        assert_eq!(detect_list_prefix("+ item"), Some(2));
    }

    #[test]
    fn test_detect_list_prefix_ordered() {
        assert_eq!(detect_list_prefix("1. item"), Some(3));
        assert_eq!(detect_list_prefix("12. item"), Some(4));
    }

    #[test]
    fn test_detect_list_prefix_tasks() {
        assert_eq!(detect_list_prefix("- [ ] todo"), Some(6));
        assert_eq!(detect_list_prefix("- [x] done"), Some(6));
        assert_eq!(detect_list_prefix("- [X] done"), Some(6));
    }

    #[test]
    fn test_detect_list_prefix_not_list() {
        assert_eq!(detect_list_prefix("normal text"), None);
        assert_eq!(detect_list_prefix("---"), None);
    }

    #[test]
    fn test_analyze_line_list() {
        let mut in_code = false;
        let info = analyze_line("- hello **world**", &mut in_code);
        assert_eq!(info.kind, LineKind::ListItem);
        assert_eq!(info.spans[0].kind, StyleKind::ListBullet);
        assert_eq!(info.spans[0].range, 0..2);
        assert!(info.spans.iter().any(|s| s.kind == StyleKind::Bold));
    }

    #[test]
    fn test_analyze_line_table() {
        let mut in_code = false;
        let info = analyze_line("| a | b |", &mut in_code);
        assert_eq!(info.kind, LineKind::TableRow);
        assert!(info.spans.iter().any(|s| s.kind == StyleKind::TableSyntax));
    }

    #[test]
    fn test_inline_bold() {
        let spans = parse_inline_styles("hello **bold** world");
        let bold = spans.iter().find(|s| s.kind == StyleKind::Bold);
        assert!(bold.is_some());
        let bold = bold.unwrap();
        assert_eq!(&"hello **bold** world"[bold.range.clone()], "**bold**");
    }

    #[test]
    fn test_inline_italic() {
        let spans = parse_inline_styles("hello *italic* world");
        let italic = spans.iter().find(|s| s.kind == StyleKind::Italic);
        assert!(italic.is_some());
        let italic = italic.unwrap();
        assert_eq!(&"hello *italic* world"[italic.range.clone()], "*italic*");
    }

    #[test]
    fn test_inline_code() {
        let spans = parse_inline_styles("use `code` here");
        let code = spans.iter().find(|s| s.kind == StyleKind::Code);
        assert!(code.is_some());
        assert_eq!(&"use `code` here"[code.unwrap().range.clone()], "`code`");
    }

    #[test]
    fn test_inline_strikethrough() {
        let spans = parse_inline_styles("~~deleted~~");
        assert_eq!(spans[0].kind, StyleKind::Strikethrough);
    }

    #[test]
    fn test_heading_levels() {
        let mut in_code = false;
        for level in 1u8..=6 {
            let prefix = "#".repeat(level as usize);
            let line = format!("{} Title", prefix);
            let info = analyze_line(&line, &mut in_code);
            assert_eq!(info.kind, LineKind::Heading(level));
        }
    }

    #[test]
    fn test_code_block() {
        let mut in_code = false;
        let info = analyze_line("```rust", &mut in_code);
        assert_eq!(info.kind, LineKind::CodeBlock);
        assert!(in_code);

        let info = analyze_line("let x = 1;", &mut in_code);
        assert_eq!(info.kind, LineKind::CodeBlock);

        let info = analyze_line("```", &mut in_code);
        assert_eq!(info.kind, LineKind::CodeBlock);
        assert!(!in_code);
    }

    #[test]
    fn test_format_table_alignment() {
        let rows = vec![
            vec![" Name ".into(), " Role ".into(), " Status ".into()],
            vec!["---".into(), "---".into(), "---".into()],
            vec![" Alice ".into(), " Dev ".into(), " Active ".into()],
            vec![" Bob ".into(), " Design ".into(), " Away ".into()],
        ];
        let is_sep = vec![false, true, false, false];
        let widths = compute_col_widths(&rows, &is_sep);
        assert_eq!(widths, vec![5, 6, 6]); // Alice, Design, Active/Status
        let formatted = format_table(&rows, &is_sep, &widths);
        let lines: Vec<&str> = formatted.split('\n').collect();
        assert_eq!(lines[0], "| Name  | Role   | Status |");
        assert_eq!(lines[1], "| ----- | ------ | ------ |");
        assert_eq!(lines[2], "| Alice | Dev    | Active |");
        assert_eq!(lines[3], "| Bob   | Design | Away   |");
        // All lines should be same length
        assert!(lines.iter().all(|l| l.len() == lines[0].len()));
    }

    #[test]
    fn test_cursor_pos_in_formatted_table() {
        let rows = vec![
            vec![" Name ".into(), " Role ".into()],
            vec!["---".into(), "---".into()],
            vec![" Alice ".into(), " Dev ".into()],
        ];
        let is_sep = vec![false, true, false];
        let widths = compute_col_widths(&rows, &is_sep);
        let formatted = format_table(&rows, &is_sep, &widths);

        // Row 0, Col 0: after "| Name" => pos should point after "Name"
        let pos = cursor_pos_in_formatted_table(0, 0, &rows, &widths, &is_sep);
        assert_eq!(&formatted[pos - 4..pos], "Name");

        // Row 0, Col 1: after "| Role" => pos should point after "Role"  
        let pos = cursor_pos_in_formatted_table(0, 1, &rows, &widths, &is_sep);
        assert_eq!(&formatted[pos - 4..pos], "Role");

        // Row 2, Col 0: after "| Alice"
        let pos = cursor_pos_in_formatted_table(2, 0, &rows, &widths, &is_sep);
        assert_eq!(&formatted[pos - 5..pos], "Alice");

        // Row 2, Col 1: after "| Dev"
        let pos = cursor_pos_in_formatted_table(2, 1, &rows, &widths, &is_sep);
        assert_eq!(&formatted[pos - 3..pos], "Dev");
    }

    #[test]
    fn test_format_table_uneven_columns() {
        // Simulate a table where user typed short content in some cells
        let rows = vec![
            vec![" a ".into(), " longer text ".into()],
            vec![" x ".into(), " y ".into()],
        ];
        let is_sep = vec![false, false];
        let widths = compute_col_widths(&rows, &is_sep);
        let formatted = format_table(&rows, &is_sep, &widths);
        let lines: Vec<&str> = formatted.split('\n').collect();
        // Both lines should be same length
        assert_eq!(lines[0].len(), lines[1].len());
        assert_eq!(lines[0], "| a   | longer text |");
        assert_eq!(lines[1], "| x   | y           |");
    }
}
