use std::ops::Range;

const INDENT: &str = "    ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeEdit {
    pub range: Range<usize>,
    pub text: String,
    pub cursor_after: usize,
}

pub fn smart_insert(content: &str, range: Range<usize>, text: &str) -> Option<CodeEdit> {
    let cursor = range.end;
    let context = code_context(content, cursor)?;

    if text == "\n" {
        return smart_newline(content, range, &context);
    }

    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    if let Some((open, close)) = pair_for_open(ch) {
        return smart_open_pair(content, range, open, close);
    }

    if is_closing_pair(ch) {
        return smart_close_pair(content, range, ch);
    }

    None
}

pub fn smart_backspace(content: &str, cursor: usize) -> Option<CodeEdit> {
    code_context(content, cursor)?;
    let prev = previous_char(content, cursor)?;
    let next = next_char(content, cursor)?;
    if matching_close(prev) == Some(next) {
        let start = cursor - prev.len_utf8();
        let end = cursor + next.len_utf8();
        return Some(CodeEdit {
            range: start..end,
            text: String::new(),
            cursor_after: start,
        });
    }
    None
}

fn smart_open_pair(
    content: &str,
    range: Range<usize>,
    open: char,
    close: char,
) -> Option<CodeEdit> {
    if open == '\'' && range.is_empty() && quote_would_split_word(content, range.start) {
        return None;
    }

    let selected = content.get(range.clone())?;
    let text = format!("{open}{selected}{close}");
    let cursor_after = if selected.is_empty() {
        range.start + open.len_utf8()
    } else {
        range.start + text.len()
    };

    Some(CodeEdit {
        range,
        text,
        cursor_after,
    })
}

fn smart_close_pair(content: &str, range: Range<usize>, close: char) -> Option<CodeEdit> {
    if !range.is_empty() {
        return None;
    }

    let line_start = line_start_at(content, range.start);
    let before = &content[line_start..range.start];
    let line_is_indent_only = before.chars().all(|ch| ch == ' ' || ch == '\t');
    let next_is_same = next_char(content, range.start) == Some(close);

    if line_is_indent_only && !before.is_empty() {
        let dedented = remove_one_indent(before);
        let text = if next_is_same {
            dedented.to_string()
        } else {
            format!("{dedented}{close}")
        };
        let cursor_after = line_start + dedented.len() + close.len_utf8();
        return Some(CodeEdit {
            range: line_start..range.start,
            text,
            cursor_after,
        });
    }

    if next_is_same {
        return Some(CodeEdit {
            range: range.start..range.start,
            text: String::new(),
            cursor_after: range.start + close.len_utf8(),
        });
    }

    None
}

fn smart_newline(
    content: &str,
    range: Range<usize>,
    context: &CodeContext<'_>,
) -> Option<CodeEdit> {
    if !range.is_empty() {
        return None;
    }

    let line_start = line_start_at(content, range.start);
    let before = &content[line_start..range.start];
    let base_indent = leading_indent(before);
    let trimmed_before = before.trim_end();

    let mut inserted = String::from("\n");
    inserted.push_str(base_indent);

    let opener = trimmed_before.chars().rev().find(|ch| !ch.is_whitespace());
    let closer = next_char(content, range.start);
    let split_pair = opener.and_then(matching_close) == closer;

    let should_indent = split_pair || should_indent_after(trimmed_before, context.language);
    if should_indent {
        inserted.push_str(INDENT);
    }

    let cursor_after = range.start + inserted.len();

    if split_pair {
        inserted.push('\n');
        inserted.push_str(base_indent);
    }

    Some(CodeEdit {
        range,
        text: inserted,
        cursor_after,
    })
}

fn should_indent_after(line: &str, language: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    if line.ends_with('{') || line.ends_with('(') || line.ends_with('[') {
        return true;
    }

    let language = language.trim().to_ascii_lowercase();
    if matches!(language.as_str(), "python" | "py" | "yaml" | "yml") && line.ends_with(':') {
        return true;
    }

    if matches!(language.as_str(), "bash" | "sh" | "shell" | "fish") {
        let last = line.split_whitespace().last().unwrap_or("");
        return matches!(last, "then" | "do" | "else");
    }

    false
}

fn pair_for_open(ch: char) -> Option<(char, char)> {
    match ch {
        '(' => Some(('(', ')')),
        '[' => Some(('[', ']')),
        '{' => Some(('{', '}')),
        '"' => Some(('"', '"')),
        '\'' => Some(('\'', '\'')),
        '`' => Some(('`', '`')),
        _ => None,
    }
}

fn is_closing_pair(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}')
}

fn matching_close(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

fn quote_would_split_word(content: &str, cursor: usize) -> bool {
    previous_char(content, cursor).is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        || next_char(content, cursor).is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}

fn previous_char(content: &str, cursor: usize) -> Option<char> {
    content.get(..cursor)?.chars().next_back()
}

fn next_char(content: &str, cursor: usize) -> Option<char> {
    content.get(cursor..)?.chars().next()
}

fn line_start_at(content: &str, offset: usize) -> usize {
    content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn leading_indent(line_prefix: &str) -> &str {
    let end = line_prefix
        .char_indices()
        .find(|(_, ch)| *ch != ' ' && *ch != '\t')
        .map(|(idx, _)| idx)
        .unwrap_or(line_prefix.len());
    &line_prefix[..end]
}

fn remove_one_indent(indent: &str) -> &str {
    if let Some(rest) = indent.strip_suffix(INDENT) {
        rest
    } else {
        indent.strip_suffix('\t').unwrap_or(indent)
    }
}

struct CodeContext<'a> {
    language: &'a str,
}

fn code_context<'a>(content: &'a str, cursor: usize) -> Option<CodeContext<'a>> {
    let mut offset = 0;
    let mut block: Option<(u8, usize, &'a str)> = None;

    for raw_line in content.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line_end = offset + line.len();
        let is_cursor_line =
            cursor <= line_end || (raw_line.ends_with('\n') && cursor == line_end + 1);
        let fence = fence_line(line);

        if is_cursor_line {
            if let Some((marker, minimum_len, language)) = block {
                if fence.as_ref().is_some_and(|f| {
                    f.marker == marker && f.marker_len >= minimum_len && f.language.is_empty()
                }) {
                    return None;
                }
                return Some(CodeContext { language });
            }
            return None;
        }

        match (block, fence) {
            (None, Some(fence)) if !fence.language.is_empty() => {
                block = Some((fence.marker, fence.marker_len, fence.language));
            }
            (Some((marker, minimum_len, _)), Some(fence))
                if fence.marker == marker
                    && fence.marker_len >= minimum_len
                    && fence.language.is_empty() =>
            {
                block = None;
            }
            _ => {}
        }

        offset += raw_line.len();
    }

    if let Some((_, _, language)) = block {
        return Some(CodeContext { language });
    }
    None
}

struct Fence<'a> {
    marker: u8,
    marker_len: usize,
    language: &'a str,
}

fn fence_line(line: &str) -> Option<Fence<'_>> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = *trimmed.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let marker_len = trimmed.bytes().take_while(|byte| *byte == marker).count();
    if marker_len < 3 {
        return None;
    }
    let rest = trimmed[marker_len..].trim();
    let language = rest.split_whitespace().next().unwrap_or("");
    Some(Fence {
        marker,
        marker_len,
        language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(content: &str, cursor: usize, text: &str) -> Option<CodeEdit> {
        smart_insert(content, cursor..cursor, text)
    }

    #[test]
    fn pairs_inside_code_block_only() {
        let doc = "```rust\nlet x = \n```";
        let cursor = doc.find("\n```").unwrap();
        assert_eq!(
            edit(doc, cursor, "("),
            Some(CodeEdit {
                range: cursor..cursor,
                text: "()".into(),
                cursor_after: cursor + 1,
            })
        );
        assert_eq!(smart_insert("plain", 5..5, "("), None);
    }

    #[test]
    fn skips_existing_close_pair() {
        let doc = "```rust\ncall()\n```";
        let cursor = doc.find(')').unwrap();
        assert_eq!(
            edit(doc, cursor, ")"),
            Some(CodeEdit {
                range: cursor..cursor,
                text: String::new(),
                cursor_after: cursor + 1,
            })
        );
    }

    #[test]
    fn backspace_deletes_empty_pair() {
        let doc = "```rust\ncall()\n```";
        let cursor = doc.find(')').unwrap();
        assert_eq!(
            smart_backspace(doc, cursor),
            Some(CodeEdit {
                range: cursor - 1..cursor + 1,
                text: String::new(),
                cursor_after: cursor - 1,
            })
        );
    }

    #[test]
    fn enter_splits_brace_pair() {
        let doc = "```rust\nfn main() {}\n```";
        let cursor = doc.find('}').unwrap();
        assert_eq!(
            edit(doc, cursor, "\n"),
            Some(CodeEdit {
                range: cursor..cursor,
                text: "\n    \n".into(),
                cursor_after: cursor + 5,
            })
        );
    }

    #[test]
    fn closing_brace_dedents_blank_line() {
        let doc = "```rust\n    \n```";
        let cursor = doc.find("\n```").unwrap();
        assert_eq!(
            edit(doc, cursor, "}"),
            Some(CodeEdit {
                range: cursor - 4..cursor,
                text: "}".into(),
                cursor_after: cursor - 3,
            })
        );
    }

    #[test]
    fn python_colon_indents() {
        let doc = "```python\nif True:\n```";
        let cursor = doc.find("\n```").unwrap();
        assert_eq!(
            edit(doc, cursor, "\n"),
            Some(CodeEdit {
                range: cursor..cursor,
                text: "\n    ".into(),
                cursor_after: cursor + 5,
            })
        );
    }
}
