use super::action::{Action, MotionImpl, OperatorImpl, OperatorOutput, TransientKind, VimMode};
use super::binding::BindingRegistry;
use super::context::when;

/// Register all default bindings, motions, and operators.
pub fn register_defaults(registry: &mut BindingRegistry) {
    register_motions(registry);
    register_operators(registry);
    register_editor_bindings(registry);
    register_vim_bindings(registry);
    register_markdown_bindings(registry);
    register_minibuffer_bindings(registry);
    register_pdf_bindings(registry);
    register_diagram_bindings(registry);
    register_graph_bindings(registry);
}

// ─── Motions ────────────────────────────────────────────────────────────────

fn register_motions(registry: &mut BindingRegistry) {
    registry.register_motion("left", MotionImpl::Native(motion_left));
    registry.register_motion("right", MotionImpl::Native(motion_right));
    registry.register_motion("up", MotionImpl::Native(motion_up));
    registry.register_motion("down", MotionImpl::Native(motion_down));
    registry.register_motion("line-start", MotionImpl::Native(motion_line_start));
    registry.register_motion("line-end", MotionImpl::Native(motion_line_end));
    registry.register_motion(
        "first-non-whitespace",
        MotionImpl::Native(motion_first_non_whitespace),
    );
    registry.register_motion(
        "line-first-non-whitespace",
        MotionImpl::Native(motion_line_first_non_whitespace),
    );
    registry.register_motion(
        "last-non-whitespace",
        MotionImpl::Native(motion_last_non_whitespace),
    );
    registry.register_motion("word-forward", MotionImpl::Native(motion_word_forward));
    registry.register_motion("word-backward", MotionImpl::Native(motion_word_backward));
    registry.register_motion("word-end", MotionImpl::Native(motion_word_end));
    registry.register_motion(
        "word-end-backward",
        MotionImpl::Native(motion_word_end_backward),
    );
    registry.register_motion(
        "big-word-forward",
        MotionImpl::Native(motion_big_word_forward),
    );
    registry.register_motion(
        "big-word-backward",
        MotionImpl::Native(motion_big_word_backward),
    );
    registry.register_motion("big-word-end", MotionImpl::Native(motion_big_word_end));
    registry.register_motion(
        "big-word-end-backward",
        MotionImpl::Native(motion_big_word_end_backward),
    );
    registry.register_motion(
        "next-line-first-non-whitespace",
        MotionImpl::Native(motion_next_line_first_non_whitespace),
    );
    registry.register_motion(
        "prev-line-first-non-whitespace",
        MotionImpl::Native(motion_prev_line_first_non_whitespace),
    );
    registry.register_motion("doc-start", MotionImpl::Native(motion_doc_start));
    registry.register_motion("doc-end", MotionImpl::Native(motion_doc_end));
    registry.register_motion(
        "paragraph-forward",
        MotionImpl::Native(motion_paragraph_forward),
    );
    registry.register_motion(
        "paragraph-backward",
        MotionImpl::Native(motion_paragraph_backward),
    );
    registry.register_motion(
        "matching-bracket",
        MotionImpl::Native(motion_matching_bracket),
    );
}

fn register_operators(registry: &mut BindingRegistry) {
    registry.register_operator("delete", OperatorImpl::Native(op_delete));
    registry.register_operator("change", OperatorImpl::Native(op_change));
    registry.register_operator("yank", OperatorImpl::Native(op_yank));
    registry.register_operator("indent", OperatorImpl::Native(op_indent));
    registry.register_operator("dedent", OperatorImpl::Native(op_dedent));
}

// ─── Motion implementations ────────────────────────────────────────────────

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p.min(s.len())
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

pub fn motion_left(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        pos = prev_char_boundary(content, pos);
    }
    pos
}

pub fn motion_right(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos < content.len() {
            pos = next_char_boundary(content, pos);
        }
    }
    pos
}

pub fn motion_up(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = pos - line_start;
        if line_start == 0 {
            pos = 0;
        } else {
            let prev_end = line_start - 1;
            let prev_start = content[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let prev_len = prev_end - prev_start;
            pos = prev_start + col.min(prev_len);
        }
    }
    pos
}

pub fn motion_down(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = pos - line_start;
        if let Some(nl) = content[pos..].find('\n') {
            let next_start = pos + nl + 1;
            let next_end = content[next_start..]
                .find('\n')
                .map(|p| next_start + p)
                .unwrap_or(content.len());
            let next_len = next_end - next_start;
            pos = next_start + col.min(next_len);
        }
    }
    pos
}

pub fn motion_line_start(content: &str, cursor: usize, _count: usize) -> usize {
    content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

pub fn motion_line_end(content: &str, cursor: usize, _count: usize) -> usize {
    content[cursor..]
        .find('\n')
        .map(|p| cursor + p)
        .unwrap_or(content.len())
}

pub fn motion_first_non_whitespace(content: &str, cursor: usize, _count: usize) -> usize {
    let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &content[line_start..];
    let byte_offset: usize = line
        .chars()
        .take_while(|c| c.is_whitespace() && *c != '\n')
        .map(|c| c.len_utf8())
        .sum();
    line_start + byte_offset
}

pub fn motion_line_first_non_whitespace(content: &str, cursor: usize, count: usize) -> usize {
    let target = motion_down(content, cursor, count.saturating_sub(1));
    motion_first_non_whitespace(content, target, 1)
}

pub fn motion_last_non_whitespace(content: &str, cursor: usize, _count: usize) -> usize {
    let line_start = motion_line_start(content, cursor, 1);
    let line_end = motion_line_end(content, cursor, 1);
    content[line_start..line_end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(offset, _)| line_start + offset)
        .unwrap_or(line_start)
}

pub fn motion_next_line_first_non_whitespace(content: &str, cursor: usize, count: usize) -> usize {
    motion_first_non_whitespace(content, motion_down(content, cursor, count), 1)
}

pub fn motion_prev_line_first_non_whitespace(content: &str, cursor: usize, count: usize) -> usize {
    motion_first_non_whitespace(content, motion_up(content, cursor, count), 1)
}

pub fn motion_word_forward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    let bytes = content.as_bytes();
    for _ in 0..count {
        // Skip current word chars
        if pos < bytes.len() {
            let ch = content[pos..].chars().next().unwrap_or(' ');
            if ch.is_alphanumeric() || ch == '_' {
                // word char — skip word chars
                while pos < content.len() {
                    let c = content[pos..].chars().next().unwrap_or(' ');
                    if c.is_alphanumeric() || c == '_' {
                        pos += c.len_utf8();
                    } else {
                        break;
                    }
                }
            } else if !ch.is_whitespace() {
                // punctuation — skip punctuation
                while pos < content.len() {
                    let c = content[pos..].chars().next().unwrap_or(' ');
                    if !c.is_alphanumeric() && c != '_' && !c.is_whitespace() {
                        pos += c.len_utf8();
                    } else {
                        break;
                    }
                }
            }
        }
        // Skip whitespace
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or('x');
            if c.is_whitespace() {
                pos += c.len_utf8();
            } else {
                break;
            }
        }
    }
    pos
}

pub fn motion_word_backward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos == 0 {
            break;
        }
        pos = prev_char_boundary(content, pos);
        // Skip whitespace backward
        while pos > 0 {
            let c = content[..pos].chars().last().unwrap_or('x');
            if c.is_whitespace() {
                pos = prev_char_boundary(content, pos);
            } else {
                break;
            }
        }
        // Skip word chars backward
        if pos > 0 {
            let c = content[..pos].chars().last().unwrap_or(' ');
            if c.is_alphanumeric() || c == '_' {
                while pos > 0 {
                    let prev = content[..pos].chars().last().unwrap_or(' ');
                    if prev.is_alphanumeric() || prev == '_' {
                        pos = prev_char_boundary(content, pos);
                    } else {
                        break;
                    }
                }
            } else {
                while pos > 0 {
                    let prev = content[..pos].chars().last().unwrap_or(' ');
                    if !prev.is_alphanumeric() && prev != '_' && !prev.is_whitespace() {
                        pos = prev_char_boundary(content, pos);
                    } else {
                        break;
                    }
                }
            }
        }
    }
    pos
}

pub fn motion_word_end(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos < content.len() {
            pos = next_char_boundary(content, pos);
        }
        // Skip whitespace
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or('x');
            if c.is_whitespace() {
                pos += c.len_utf8();
            } else {
                break;
            }
        }
        // Skip to end of word
        if pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or(' ');
            if c.is_alphanumeric() || c == '_' {
                while pos < content.len() {
                    let next_pos = next_char_boundary(content, pos);
                    if next_pos < content.len() {
                        let nc = content[next_pos..].chars().next().unwrap_or(' ');
                        if nc.is_alphanumeric() || nc == '_' {
                            pos = next_pos;
                        } else {
                            break;
                        }
                    } else {
                        pos = next_pos.min(content.len());
                        break;
                    }
                }
            } else {
                while pos < content.len() {
                    let next_pos = next_char_boundary(content, pos);
                    if next_pos < content.len() {
                        let nc = content[next_pos..].chars().next().unwrap_or(' ');
                        if !nc.is_alphanumeric() && nc != '_' && !nc.is_whitespace() {
                            pos = next_pos;
                        } else {
                            break;
                        }
                    } else {
                        pos = next_pos.min(content.len());
                        break;
                    }
                }
            }
        }
    }
    pos
}

fn char_class(ch: char) -> u8 {
    if ch.is_whitespace() {
        0
    } else if ch.is_alphanumeric() || ch == '_' {
        1
    } else {
        2
    }
}

fn motion_end_backward_by_class(
    content: &str,
    cursor: usize,
    count: usize,
    class: impl Fn(char) -> u8,
) -> usize {
    let mut pos = cursor.min(content.len());
    for _ in 0..count {
        if pos == 0 {
            break;
        }
        if pos < content.len() {
            let current_class = class(content[pos..].chars().next().unwrap());
            if current_class != 0 {
                while pos > 0 {
                    let prev = content[..pos].chars().last().unwrap();
                    if class(prev) != current_class {
                        break;
                    }
                    pos = prev_char_boundary(content, pos);
                }
            }
        }
        pos = prev_char_boundary(content, pos);
        while pos > 0 && class(content[pos..].chars().next().unwrap()) == 0 {
            pos = prev_char_boundary(content, pos);
        }
    }
    pos
}

pub fn motion_word_end_backward(content: &str, cursor: usize, count: usize) -> usize {
    motion_end_backward_by_class(content, cursor, count, char_class)
}

pub fn motion_big_word_forward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or(' ');
            if !c.is_whitespace() {
                pos += c.len_utf8();
            } else {
                break;
            }
        }
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or('x');
            if c.is_whitespace() {
                pos += c.len_utf8();
            } else {
                break;
            }
        }
    }
    pos
}

pub fn motion_big_word_backward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos == 0 {
            break;
        }
        pos = prev_char_boundary(content, pos);
        while pos > 0 {
            let c = content[..pos].chars().last().unwrap_or('x');
            if c.is_whitespace() {
                pos = prev_char_boundary(content, pos);
            } else {
                break;
            }
        }
        while pos > 0 {
            let c = content[..pos].chars().last().unwrap_or(' ');
            if !c.is_whitespace() {
                pos = prev_char_boundary(content, pos);
            } else {
                break;
            }
        }
    }
    pos
}

pub fn motion_big_word_end(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos < content.len() {
            pos = next_char_boundary(content, pos);
        }
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or('x');
            if c.is_whitespace() {
                pos += c.len_utf8();
            } else {
                break;
            }
        }
        while pos < content.len() {
            let next = next_char_boundary(content, pos);
            if next < content.len() {
                let nc = content[next..].chars().next().unwrap_or(' ');
                if !nc.is_whitespace() {
                    pos = next;
                } else {
                    break;
                }
            } else {
                pos = next.min(content.len());
                break;
            }
        }
    }
    pos
}

pub fn motion_big_word_end_backward(content: &str, cursor: usize, count: usize) -> usize {
    motion_end_backward_by_class(content, cursor, count, |ch| u8::from(!ch.is_whitespace()))
}

pub fn motion_doc_start(_content: &str, _cursor: usize, _count: usize) -> usize {
    0
}

pub fn motion_doc_end(content: &str, _cursor: usize, _count: usize) -> usize {
    content.len()
}

pub fn motion_paragraph_forward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        // Find next empty line
        while pos < content.len() {
            if let Some(nl) = content[pos..].find('\n') {
                pos = pos + nl + 1;
                if pos < content.len() && content[pos..].starts_with('\n') {
                    break;
                }
            } else {
                pos = content.len();
                break;
            }
        }
    }
    pos
}

pub fn motion_paragraph_backward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos == 0 {
            break;
        }
        pos = pos.saturating_sub(1);
        // Find previous empty line
        while pos > 0 {
            if content[..pos].ends_with("\n\n") {
                break;
            }
            pos -= 1;
        }
    }
    pos
}

pub fn motion_matching_bracket(content: &str, cursor: usize, _count: usize) -> usize {
    let ch = content[cursor..].chars().next().unwrap_or(' ');
    let (target, forward) = match ch {
        '(' => (')', true),
        '[' => (']', true),
        '{' => ('}', true),
        ')' => ('(', false),
        ']' => ('[', false),
        '}' => ('{', false),
        _ => return cursor,
    };

    let mut depth = 0i32;
    if forward {
        for (i, c) in content[cursor..].char_indices() {
            if c == ch {
                depth += 1;
            }
            if c == target {
                depth -= 1;
            }
            if depth == 0 {
                return cursor + i;
            }
        }
    } else {
        for (i, c) in content[..=cursor].char_indices().rev() {
            if c == ch {
                depth += 1;
            }
            if c == target {
                depth -= 1;
            }
            if depth == 0 {
                return i;
            }
        }
    }
    cursor
}

// ─── Operator implementations ───────────────────────────────────────────────

fn op_delete(content: &str, start: usize, end: usize) -> OperatorOutput {
    OperatorOutput {
        delete_range: Some(start..end),
        yanked: content.get(start..end).unwrap_or("").to_string(),
        enter_insert: false,
    }
}

fn op_change(content: &str, start: usize, end: usize) -> OperatorOutput {
    OperatorOutput {
        delete_range: Some(start..end),
        yanked: content.get(start..end).unwrap_or("").to_string(),
        enter_insert: true,
    }
}

fn op_yank(content: &str, start: usize, end: usize) -> OperatorOutput {
    OperatorOutput {
        delete_range: None,
        yanked: content.get(start..end).unwrap_or("").to_string(),
        enter_insert: false,
    }
}

fn op_indent(_content: &str, _start: usize, _end: usize) -> OperatorOutput {
    OperatorOutput {
        delete_range: None,
        yanked: String::new(),
        enter_insert: false,
    }
}

fn op_dedent(_content: &str, _start: usize, _end: usize) -> OperatorOutput {
    OperatorOutput {
        delete_range: None,
        yanked: String::new(),
        enter_insert: false,
    }
}

// ─── Binding definitions ───────────────────────────────────────────────────

fn bind(
    registry: &mut BindingRegistry,
    context: super::context::KeyPredicate,
    key: &str,
    action: Action,
) {
    registry.bind(key, context, action);
}

fn editor() -> super::context::KeyPredicate {
    when().require("Editor")
}

fn vim(mode: VimMode) -> super::context::KeyPredicate {
    editor().require_value("vim_mode", mode.as_context_value())
}

fn register_editor_bindings(registry: &mut BindingRegistry) {
    for (key, motion) in [
        ("left", "left"),
        ("right", "right"),
        ("up", "up"),
        ("down", "down"),
        ("home", "line-start"),
        ("end", "line-end"),
    ] {
        bind(registry, editor(), key, Action::Motion(motion));
    }

    for (key, command) in [
        ("backspace", "delete-backward"),
        ("delete", "delete-forward"),
        ("enter", "insert-newline"),
        ("tab", "insert-tab"),
        ("ctrl-z", "undo"),
        ("ctrl-shift-z", "redo"),
        ("shift-left", "select-left"),
        ("shift-right", "select-right"),
        ("shift-up", "select-up"),
        ("shift-down", "select-down"),
    ] {
        bind(registry, editor(), key, Action::Command(command));
    }
}

fn register_vim_bindings(registry: &mut BindingRegistry) {
    let normal = vim(VimMode::Normal);
    for (key, motion) in [
        ("h", "left"),
        ("l", "right"),
        ("j", "down"),
        ("k", "up"),
        ("w", "word-forward"),
        ("b", "word-backward"),
        ("e", "word-end"),
        ("shift-w", "big-word-forward"),
        ("shift-b", "big-word-backward"),
        ("shift-e", "big-word-end"),
        ("0", "line-start"),
        ("$", "line-end"),
        ("^", "first-non-whitespace"),
        ("_", "line-first-non-whitespace"),
        ("+", "next-line-first-non-whitespace"),
        ("enter", "next-line-first-non-whitespace"),
        ("-", "prev-line-first-non-whitespace"),
        ("shift-g", "doc-end"),
        ("}", "paragraph-forward"),
        ("{", "paragraph-backward"),
        ("%", "matching-bracket"),
    ] {
        bind(registry, normal.clone(), key, Action::Motion(motion));
    }

    for (key, op) in [
        ("d", "delete"),
        ("c", "change"),
        ("y", "yank"),
        (">", "indent"),
        ("<", "dedent"),
    ] {
        bind(registry, normal.clone(), key, Action::Operator(op));
    }

    bind(
        registry,
        normal.clone(),
        "i",
        Action::SetVimMode(VimMode::Insert),
    );
    bind(
        registry,
        normal.clone(),
        "v",
        Action::SetVimMode(VimMode::Visual),
    );
    bind(
        registry,
        normal.clone(),
        "shift-v",
        Action::SetVimMode(VimMode::VisualLine),
    );

    for (key, command) in [
        ("u", "undo"),
        ("ctrl-r", "redo"),
        ("x", "delete-char-forward"),
        ("shift-x", "delete-char-backward"),
        ("shift-i", "insert-at-line-start"),
        ("shift-a", "insert-at-line-end"),
        ("a", "append-after"),
        ("o", "open-line-below"),
        ("shift-o", "open-line-above"),
        ("p", "paste-after"),
        ("shift-p", "paste-before"),
        ("shift-j", "join-lines"),
        ("shift-d", "delete-to-end"),
        ("shift-c", "change-to-end"),
        ("shift-s", "change-line"),
        ("s", "substitute-char"),
        ("shift-y", "yank-line"),
        ("~", "toggle-case"),
        (":", "command-palette"),
        (".", "dot-repeat"),
    ] {
        bind(registry, normal.clone(), key, Action::Command(command));
    }

    for (key, action) in [
        ("g g", Action::Command("goto-doc-start")),
        ("g e", Action::Motion("word-end-backward")),
        ("g shift-e", Action::Motion("big-word-end-backward")),
        ("g 0", Action::Motion("line-start")),
        ("g ^", Action::Motion("first-non-whitespace")),
        ("g $", Action::Motion("line-end")),
        ("g _", Action::Motion("last-non-whitespace")),
    ] {
        bind(registry, normal.clone(), key, action);
    }

    for (key, transient) in [
        ("f", TransientKind::FindChar),
        ("t", TransientKind::TilChar),
        ("shift-f", TransientKind::FindCharBack),
        ("shift-t", TransientKind::TilCharBack),
        ("r", TransientKind::ReplaceChar),
    ] {
        bind(
            registry,
            normal.clone(),
            key,
            Action::PushTransient(transient),
        );
    }

    for (key, command) in [
        ("ctrl-d", "scroll-half-down"),
        ("ctrl-u", "scroll-half-up"),
        ("ctrl-f", "scroll-page-down"),
        ("ctrl-b", "scroll-page-up"),
        ("ctrl-e", "scroll-line-down"),
        ("ctrl-y", "scroll-line-up"),
        (";", "repeat-char-search"),
        (",", "repeat-char-search-reverse"),
    ] {
        bind(registry, normal.clone(), key, Action::Command(command));
    }

    bind(
        registry,
        vim(VimMode::Insert),
        "escape",
        Action::SetVimMode(VimMode::Normal),
    );

    let visual = when().require("Editor").require_value("vim_mode", "visual");
    register_visual_bindings(registry, visual, false);
    let visual_line = vim(VimMode::VisualLine);
    bind(
        registry,
        visual_line.clone(),
        "j",
        Action::Command("visual-line-down"),
    );
    bind(
        registry,
        visual_line.clone(),
        "k",
        Action::Command("visual-line-up"),
    );
    for (key, command) in [
        ("d", "delete-selection"),
        ("y", "yank-selection"),
        ("c", "change-selection"),
        (">", "indent-selection"),
        ("<", "dedent-selection"),
    ] {
        bind(registry, visual_line.clone(), key, Action::Command(command));
    }
    bind(
        registry,
        visual_line.clone(),
        "escape",
        Action::SetVimMode(VimMode::Normal),
    );
    bind(
        registry,
        visual_line,
        "shift-v",
        Action::SetVimMode(VimMode::Normal),
    );

    for (key, command) in [
        ("space f", "find-note"),
        ("space s", "save"),
        ("space b", "find-note"),
        ("space n", "find-note"),
        ("space v s", "vault-switch"),
        ("space v o", "vault-open"),
        ("space space", "command-palette"),
        ("space q q", "quit"),
    ] {
        bind(registry, normal.clone(), key, Action::Command(command));
    }
}

fn register_visual_bindings(
    registry: &mut BindingRegistry,
    visual: super::context::KeyPredicate,
    _linewise: bool,
) {
    for (key, motion) in [
        ("h", "left"),
        ("l", "right"),
        ("j", "down"),
        ("k", "up"),
        ("w", "word-forward"),
        ("b", "word-backward"),
        ("e", "word-end"),
        ("shift-w", "big-word-forward"),
        ("shift-b", "big-word-backward"),
        ("shift-e", "big-word-end"),
        ("0", "line-start"),
        ("$", "line-end"),
        ("^", "first-non-whitespace"),
        ("_", "line-first-non-whitespace"),
        ("+", "next-line-first-non-whitespace"),
        ("enter", "next-line-first-non-whitespace"),
        ("-", "prev-line-first-non-whitespace"),
        ("shift-g", "doc-end"),
        ("}", "paragraph-forward"),
        ("{", "paragraph-backward"),
        ("%", "matching-bracket"),
        ("g g", "doc-start"),
        ("g e", "word-end-backward"),
        ("g shift-e", "big-word-end-backward"),
        ("g 0", "line-start"),
        ("g ^", "first-non-whitespace"),
        ("g $", "line-end"),
        ("g _", "last-non-whitespace"),
    ] {
        bind(registry, visual.clone(), key, Action::Motion(motion));
    }

    for (key, transient) in [
        ("f", TransientKind::FindChar),
        ("t", TransientKind::TilChar),
        ("shift-f", TransientKind::FindCharBack),
        ("shift-t", TransientKind::TilCharBack),
    ] {
        bind(
            registry,
            visual.clone(),
            key,
            Action::PushTransient(transient),
        );
    }

    for (key, command) in [
        (";", "repeat-char-search"),
        (",", "repeat-char-search-reverse"),
        ("d", "delete-selection"),
        ("x", "delete-selection"),
        ("y", "yank-selection"),
        ("c", "change-selection"),
        (">", "indent-selection"),
        ("<", "dedent-selection"),
        ("~", "toggle-case-selection"),
        ("shift-u", "uppercase-selection"),
        ("u", "lowercase-selection"),
        ("shift-j", "join-selection"),
        (":", "command-palette"),
    ] {
        bind(registry, visual.clone(), key, Action::Command(command));
    }
    bind(
        registry,
        visual.clone(),
        "escape",
        Action::SetVimMode(VimMode::Normal),
    );
    bind(registry, visual, "v", Action::SetVimMode(VimMode::Normal));
}

fn register_markdown_bindings(registry: &mut BindingRegistry) {
    bind(
        registry,
        editor().require("table"),
        "tab",
        Action::Command("table-next-cell"),
    );
    bind(
        registry,
        editor()
            .require("heading")
            .forbid("code_block")
            .forbid("table"),
        "tab",
        Action::Command("outline-cycle-fold"),
    );
    bind(
        registry,
        editor().require("table"),
        "shift-tab",
        Action::Command("table-prev-cell"),
    );
    bind(
        registry,
        editor().forbid("code_block"),
        "shift-tab",
        Action::Command("outline-global-cycle"),
    );
    for (key, command) in [
        ("alt-left", "outline-promote"),
        ("alt-right", "outline-demote"),
        ("alt-up", "outline-move-up"),
        ("alt-down", "outline-move-down"),
        ("alt-n", "outline-next-heading"),
        ("alt-p", "outline-prev-heading"),
    ] {
        bind(registry, editor(), key, Action::Command(command));
    }
}

fn register_minibuffer_bindings(registry: &mut BindingRegistry) {
    let minibuffer = when().require("Minibuffer");
    for (key, command) in [
        ("enter", "minibuffer-confirm"),
        ("escape", "minibuffer-dismiss"),
        ("tab", "minibuffer-complete"),
        ("ctrl-n", "minibuffer-next"),
        ("ctrl-p", "minibuffer-prev"),
        ("ctrl-a", "minibuffer-start"),
        ("ctrl-e", "minibuffer-end"),
        ("ctrl-u", "minibuffer-kill-to-start"),
        ("ctrl-k", "minibuffer-kill-to-end"),
        ("ctrl-w", "minibuffer-kill-word-back"),
        ("backspace", "minibuffer-delete-backward"),
        ("left", "minibuffer-cursor-left"),
        ("right", "minibuffer-cursor-right"),
    ] {
        bind(registry, minibuffer.clone(), key, Action::Command(command));
    }
}

fn register_pdf_bindings(registry: &mut BindingRegistry) {
    let pdf = when().require("Pdf");
    for (key, command) in [
        ("j", "pdf-scroll-down"),
        ("k", "pdf-scroll-up"),
        ("down", "pdf-scroll-down"),
        ("up", "pdf-scroll-up"),
        ("ctrl-d", "pdf-half-page-down"),
        ("ctrl-u", "pdf-half-page-up"),
        ("g", "pdf-goto-first"),
        ("shift-g", "pdf-goto-last"),
        ("+", "pdf-zoom-in"),
        ("=", "pdf-zoom-in"),
        ("-", "pdf-zoom-out"),
        ("o", "pdf-toc"),
        ("shift-p", "pdf-goto-page"),
        ("w", "pdf-fit-width"),
        ("shift-w", "pdf-fit-page"),
        ("r", "pdf-rotate-cw"),
        ("shift-r", "pdf-rotate-ccw"),
        ("h", "pdf-highlight-selection"),
        ("x", "pdf-delete-annotation"),
        ("escape", "pdf-clear-selection"),
        ("y", "pdf-copy-link"),
        ("shift-y", "pdf-extract-text"),
        ("/", "pdf-search"),
        ("n", "pdf-search-next"),
        ("shift-n", "pdf-search-prev"),
        ("q", "quit"),
    ] {
        bind(registry, pdf.clone(), key, Action::Command(command));
    }
}

fn register_graph_bindings(registry: &mut BindingRegistry) {
    let graph = when().require("Graph");
    for (key, command) in [
        ("+", "zoom-in"),
        ("=", "zoom-in"),
        ("-", "zoom-out"),
        ("0", "reset-zoom"),
        ("c", "center-graph"),
        ("l", "toggle-local-graph"),
        ("q", "quit"),
    ] {
        bind(registry, graph.clone(), key, Action::Command(command));
    }
}

fn register_diagram_bindings(registry: &mut BindingRegistry) {
    let diagram = when().require("Diagram");
    for (key, command) in [
        ("+", "diagram.zoom-in"),
        ("=", "diagram.zoom-in"),
        ("-", "diagram.zoom-out"),
        ("0", "diagram.reset-zoom"),
        ("shift-1", "diagram.fit"),
        ("ctrl-a", "diagram.select-all"),
        ("q", "quit"),
    ] {
        bind(registry, diagram.clone(), key, Action::Command(command));
    }
}
