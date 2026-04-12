use super::action::{Action, MotionImpl, OperatorImpl, OperatorOutput};
use super::layer::{Layer, LayerStack};

/// Register all default layers, motions, and operators.
pub fn register_defaults(stack: &mut LayerStack) {
    register_motions(stack);
    register_operators(stack);
    register_global_layer(stack);
    register_vim_layers(stack);
    register_markdown_layer(stack);
    register_minibuffer_layer(stack);
}

// ─── Motions ────────────────────────────────────────────────────────────────

fn register_motions(stack: &mut LayerStack) {
    stack.register_motion("left", MotionImpl::Native(motion_left));
    stack.register_motion("right", MotionImpl::Native(motion_right));
    stack.register_motion("up", MotionImpl::Native(motion_up));
    stack.register_motion("down", MotionImpl::Native(motion_down));
    stack.register_motion("line-start", MotionImpl::Native(motion_line_start));
    stack.register_motion("line-end", MotionImpl::Native(motion_line_end));
    stack.register_motion("first-non-whitespace", MotionImpl::Native(motion_first_non_whitespace));
    stack.register_motion("word-forward", MotionImpl::Native(motion_word_forward));
    stack.register_motion("word-backward", MotionImpl::Native(motion_word_backward));
    stack.register_motion("word-end", MotionImpl::Native(motion_word_end));
    stack.register_motion("big-word-forward", MotionImpl::Native(motion_big_word_forward));
    stack.register_motion("big-word-backward", MotionImpl::Native(motion_big_word_backward));
    stack.register_motion("big-word-end", MotionImpl::Native(motion_big_word_end));
    stack.register_motion("doc-start", MotionImpl::Native(motion_doc_start));
    stack.register_motion("doc-end", MotionImpl::Native(motion_doc_end));
    stack.register_motion("paragraph-forward", MotionImpl::Native(motion_paragraph_forward));
    stack.register_motion("paragraph-backward", MotionImpl::Native(motion_paragraph_backward));
    stack.register_motion("matching-bracket", MotionImpl::Native(motion_matching_bracket));
}

fn register_operators(stack: &mut LayerStack) {
    stack.register_operator("delete", OperatorImpl::Native(op_delete));
    stack.register_operator("change", OperatorImpl::Native(op_change));
    stack.register_operator("yank", OperatorImpl::Native(op_yank));
    stack.register_operator("indent", OperatorImpl::Native(op_indent));
    stack.register_operator("dedent", OperatorImpl::Native(op_dedent));
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
    if pos == 0 { return 0; }
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
            let next_end = content[next_start..].find('\n')
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
    content[cursor..].find('\n').map(|p| cursor + p).unwrap_or(content.len())
}

pub fn motion_first_non_whitespace(content: &str, cursor: usize, _count: usize) -> usize {
    let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &content[line_start..];
    let offset = line.chars().take_while(|c| c.is_whitespace() && *c != '\n').count();
    // Need byte offset not char count
    let byte_offset: usize = line.chars()
        .take_while(|c| c.is_whitespace() && *c != '\n')
        .map(|c| c.len_utf8())
        .sum();
    line_start + byte_offset
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
        if pos == 0 { break; }
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

pub fn motion_big_word_forward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or(' ');
            if !c.is_whitespace() { pos += c.len_utf8(); } else { break; }
        }
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or('x');
            if c.is_whitespace() { pos += c.len_utf8(); } else { break; }
        }
    }
    pos
}

pub fn motion_big_word_backward(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos == 0 { break; }
        pos = prev_char_boundary(content, pos);
        while pos > 0 {
            let c = content[..pos].chars().last().unwrap_or('x');
            if c.is_whitespace() { pos = prev_char_boundary(content, pos); } else { break; }
        }
        while pos > 0 {
            let c = content[..pos].chars().last().unwrap_or(' ');
            if !c.is_whitespace() { pos = prev_char_boundary(content, pos); } else { break; }
        }
    }
    pos
}

pub fn motion_big_word_end(content: &str, cursor: usize, count: usize) -> usize {
    let mut pos = cursor;
    for _ in 0..count {
        if pos < content.len() { pos = next_char_boundary(content, pos); }
        while pos < content.len() {
            let c = content[pos..].chars().next().unwrap_or('x');
            if c.is_whitespace() { pos += c.len_utf8(); } else { break; }
        }
        while pos < content.len() {
            let next = next_char_boundary(content, pos);
            if next < content.len() {
                let nc = content[next..].chars().next().unwrap_or(' ');
                if !nc.is_whitespace() { pos = next; } else { break; }
            } else {
                pos = next.min(content.len());
                break;
            }
        }
    }
    pos
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
        if pos == 0 { break; }
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
            if c == ch { depth += 1; }
            if c == target { depth -= 1; }
            if depth == 0 { return cursor + i; }
        }
    } else {
        for (i, c) in content[..=cursor].char_indices().rev() {
            if c == ch { depth += 1; }
            if c == target { depth -= 1; }
            if depth == 0 { return i; }
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

// ─── Layer definitions ──────────────────────────────────────────────────────

fn register_global_layer(stack: &mut LayerStack) {
    let mut layer = Layer::new("global");

    // Movement
    layer = layer
        .bind("left", Action::Motion("left"))
        .bind("right", Action::Motion("right"))
        .bind("up", Action::Motion("up"))
        .bind("down", Action::Motion("down"))
        .bind("home", Action::Motion("line-start"))
        .bind("end", Action::Motion("line-end"));

    // Editing
    layer = layer
        .bind("backspace", Action::Command("delete-backward"))
        .bind("delete", Action::Command("delete-forward"))
        .bind("enter", Action::Command("insert-newline"))
        .bind("tab", Action::Command("insert-tab"));

    // History
    layer = layer
        .bind("ctrl-z", Action::Command("undo"))
        .bind("ctrl-shift-z", Action::Command("redo"));

    // Selection
    layer = layer
        .bind("shift-left", Action::Command("select-left"))
        .bind("shift-right", Action::Command("select-right"))
        .bind("shift-up", Action::Command("select-up"))
        .bind("shift-down", Action::Command("select-down"));

    // App-level
    layer = layer
        .bind("ctrl-s", Action::Command("save"))
        .bind("ctrl-p", Action::Command("find-note"))
        .bind("alt-x", Action::Command("command-palette"))
        .bind("ctrl-shift-v", Action::Command("toggle-vim"));

    stack.register_layer(layer);
}

fn register_vim_layers(stack: &mut LayerStack) {
    // ── vim:normal ──
    let mut normal = Layer::new("vim:normal").with_group("vim-state");

    // Motions
    normal = normal
        .bind("h", Action::Motion("left"))
        .bind("l", Action::Motion("right"))
        .bind("j", Action::Motion("down"))
        .bind("k", Action::Motion("up"))
        .bind("w", Action::Motion("word-forward"))
        .bind("b", Action::Motion("word-backward"))
        .bind("e", Action::Motion("word-end"))
        .bind("shift-w", Action::Motion("big-word-forward"))
        .bind("shift-b", Action::Motion("big-word-backward"))
        .bind("shift-e", Action::Motion("big-word-end"))
        .bind("0", Action::Motion("line-start"))
        .bind("$", Action::Motion("line-end"))
        .bind("^", Action::Motion("first-non-whitespace"))
        .bind("shift-g", Action::Motion("doc-end"))
        .bind("}", Action::Motion("paragraph-forward"))
        .bind("{", Action::Motion("paragraph-backward"))
        .bind("%", Action::Motion("matching-bracket"));

    // Operators
    normal = normal
        .bind("d", Action::Operator("delete"))
        .bind("c", Action::Operator("change"))
        .bind("y", Action::Operator("yank"))
        .bind(">", Action::Operator("indent"))
        .bind("<", Action::Operator("dedent"));

    // Mode switches
    normal = normal
        .bind("i", Action::ActivateLayer("vim:insert"))
        .bind("v", Action::ActivateLayer("vim:visual"))
        .bind("shift-v", Action::ActivateLayer("vim:visual-line"));

    // Commands
    normal = normal
        .bind("u", Action::Command("undo"))
        .bind("ctrl-r", Action::Command("redo"))
        .bind("x", Action::Command("delete-char-forward"))
        .bind("shift-x", Action::Command("delete-char-backward"))
        .bind("shift-i", Action::Command("insert-at-line-start"))
        .bind("shift-a", Action::Command("insert-at-line-end"))
        .bind("a", Action::Command("append-after"))
        .bind("o", Action::Command("open-line-below"))
        .bind("shift-o", Action::Command("open-line-above"))
        .bind("p", Action::Command("paste-after"))
        .bind("shift-p", Action::Command("paste-before"))
        .bind("shift-j", Action::Command("join-lines"))
        .bind("shift-d", Action::Command("delete-to-end"))
        .bind("shift-c", Action::Command("change-to-end"))
        .bind("shift-s", Action::Command("change-line"))
        .bind("~", Action::Command("toggle-case"))
        .bind(":", Action::Command("command-palette"))
        .bind(".", Action::Command("dot-repeat"));

    // Transient waits
    normal = normal
        .bind("f", Action::PushTransient("transient:find-char"))
        .bind("t", Action::PushTransient("transient:til-char"))
        .bind("shift-f", Action::PushTransient("transient:find-char-back"))
        .bind("shift-t", Action::PushTransient("transient:til-char-back"))
        .bind("r", Action::PushTransient("transient:replace-char"))
        .bind("g", Action::PushTransient("transient:g-prefix"));

    // Scrolling
    normal = normal
        .bind("ctrl-d", Action::Command("scroll-half-down"))
        .bind("ctrl-u", Action::Command("scroll-half-up"));

    // Char search repeat
    normal = normal
        .bind(";", Action::Command("repeat-char-search"))
        .bind(",", Action::Command("repeat-char-search-reverse"));

    stack.register_layer(normal);

    // ── vim:insert ──
    let insert = Layer::new("vim:insert")
        .with_group("vim-state")
        .bind("escape", Action::ActivateLayer("vim:normal"));

    stack.register_layer(insert);

    // ── vim:visual ──
    let mut visual = Layer::new("vim:visual").with_group("vim-state");

    // Motions (extend selection)
    visual = visual
        .bind("h", Action::Motion("left"))
        .bind("l", Action::Motion("right"))
        .bind("j", Action::Motion("down"))
        .bind("k", Action::Motion("up"))
        .bind("w", Action::Motion("word-forward"))
        .bind("b", Action::Motion("word-backward"))
        .bind("e", Action::Motion("word-end"))
        .bind("0", Action::Motion("line-start"))
        .bind("$", Action::Motion("line-end"))
        .bind("^", Action::Motion("first-non-whitespace"))
        .bind("shift-g", Action::Motion("doc-end"));

    // Operators (act on selection)
    visual = visual
        .bind("d", Action::Command("delete-selection"))
        .bind("x", Action::Command("delete-selection"))
        .bind("y", Action::Command("yank-selection"))
        .bind("c", Action::Command("change-selection"))
        .bind(">", Action::Command("indent-selection"))
        .bind("<", Action::Command("dedent-selection"))
        .bind("~", Action::Command("toggle-case-selection"))
        .bind("shift-u", Action::Command("uppercase-selection"))
        .bind("u", Action::Command("lowercase-selection"))
        .bind("shift-j", Action::Command("join-selection"));

    visual = visual
        .bind("escape", Action::ActivateLayer("vim:normal"))
        .bind(":", Action::Command("command-palette"))
        .bind("v", Action::ActivateLayer("vim:normal"));

    stack.register_layer(visual);

    // ── vim:visual-line ──
    let mut vline = Layer::new("vim:visual-line").with_group("vim-state");
    vline = vline
        .bind("j", Action::Motion("down"))
        .bind("k", Action::Motion("up"))
        .bind("d", Action::Command("delete-selection"))
        .bind("y", Action::Command("yank-selection"))
        .bind("c", Action::Command("change-selection"))
        .bind(">", Action::Command("indent-selection"))
        .bind("<", Action::Command("dedent-selection"))
        .bind("escape", Action::ActivateLayer("vim:normal"))
        .bind("shift-v", Action::ActivateLayer("vim:normal"));

    stack.register_layer(vline);

    // ── vim:op-pending ──
    // Motions only (for operator+motion composition)
    // This layer is pushed automatically by grammar when an operator is pending
    // For now, motions are resolved from vim:normal which stays in the stack

    // ── Transient layers ──
    register_transient_layers(stack);
}

fn register_transient_layers(stack: &mut LayerStack) {
    // These are stub layers — the grammar engine handles transient key capture
    // by reading the next key and resolving the motion with the captured char.
    // The transient layer just signals to the grammar what kind of wait this is.

    stack.register_layer(Layer::new("transient:find-char").transient());
    stack.register_layer(Layer::new("transient:til-char").transient());
    stack.register_layer(Layer::new("transient:find-char-back").transient());
    stack.register_layer(Layer::new("transient:til-char-back").transient());
    stack.register_layer(Layer::new("transient:replace-char").transient());
    stack.register_layer(Layer::new("transient:g-prefix").transient());
}

fn register_markdown_layer(stack: &mut LayerStack) {
    let layer = Layer::new("markdown")
        // Outline mode (org-mode style)
        .bind("tab", Action::Command("outline-cycle-fold"))
        .bind("shift-tab", Action::Command("outline-global-cycle"))
        .bind("alt-left", Action::Command("outline-promote"))
        .bind("alt-right", Action::Command("outline-demote"))
        .bind("alt-up", Action::Command("outline-move-up"))
        .bind("alt-down", Action::Command("outline-move-down"))
        .bind("alt-n", Action::Command("outline-next-heading"))
        .bind("alt-p", Action::Command("outline-prev-heading"));
    stack.register_layer(layer);
}

fn register_minibuffer_layer(stack: &mut LayerStack) {
    let layer = Layer::new("minibuffer")
        .bind("enter", Action::Command("minibuffer-confirm"))
        .bind("escape", Action::Command("minibuffer-dismiss"))
        .bind("tab", Action::Command("minibuffer-complete"))
        .bind("ctrl-n", Action::Command("minibuffer-next"))
        .bind("ctrl-p", Action::Command("minibuffer-prev"))
        .bind("ctrl-a", Action::Command("minibuffer-start"))
        .bind("ctrl-e", Action::Command("minibuffer-end"))
        .bind("ctrl-u", Action::Command("minibuffer-kill-to-start"))
        .bind("ctrl-k", Action::Command("minibuffer-kill-to-end"))
        .bind("ctrl-w", Action::Command("minibuffer-kill-word-back"))
        .bind("backspace", Action::Command("minibuffer-delete-backward"))
        .bind("left", Action::Command("minibuffer-cursor-left"))
        .bind("right", Action::Command("minibuffer-cursor-right"));

    stack.register_layer(layer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_defaults_builds_all_layers() {
        let mut stack = LayerStack::new();
        register_defaults(&mut stack);

        // Check key layers exist and can activate
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");
        assert!(stack.is_active("global"));
        assert!(stack.is_active("vim:normal"));
    }

    #[test]
    fn test_vim_normal_h_resolves_to_motion() {
        let mut stack = LayerStack::new();
        register_defaults(&mut stack);
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");

        let combo = super::super::action::KeyCombo::parse("h");
        let action = stack.resolve(&combo);
        assert_eq!(action, Some(Action::Motion("left")));
    }

    #[test]
    fn test_vim_insert_escape_resolves() {
        let mut stack = LayerStack::new();
        register_defaults(&mut stack);
        stack.activate_layer("global");
        stack.activate_layer("vim:insert");

        let combo = super::super::action::KeyCombo::parse("escape");
        let action = stack.resolve(&combo);
        assert_eq!(action, Some(Action::ActivateLayer("vim:normal")));
    }

    #[test]
    fn test_global_ctrl_z_resolves() {
        let mut stack = LayerStack::new();
        register_defaults(&mut stack);
        stack.activate_layer("global");

        let combo = super::super::action::KeyCombo::parse("ctrl-z");
        let action = stack.resolve(&combo);
        assert_eq!(action, Some(Action::Command("undo")));
    }

    #[test]
    fn test_motion_left() {
        assert_eq!(motion_left("hello", 3, 1), 2);
        assert_eq!(motion_left("hello", 0, 1), 0);
    }

    #[test]
    fn test_motion_right() {
        assert_eq!(motion_right("hello", 0, 1), 1);
        assert_eq!(motion_right("hello", 4, 1), 5);
        assert_eq!(motion_right("hello", 5, 1), 5); // at end
    }

    #[test]
    fn test_motion_up_down() {
        let content = "first\nsecond\nthird";
        // From "second" line (pos 8 = 'o'), up should go to "first" line
        let up = motion_up(content, 8, 1);
        assert!(up < 6); // should be in first line
        // From first line, down
        let down = motion_down(content, 2, 1);
        assert!(down >= 6 && down <= 12); // should be in second line
    }

    #[test]
    fn test_motion_word_forward() {
        assert_eq!(motion_word_forward("hello world", 0, 1), 6);
        assert_eq!(motion_word_forward("hello  world", 0, 1), 7);
    }

    #[test]
    fn test_motion_line_start_end() {
        let content = "hello\nworld";
        assert_eq!(motion_line_start(content, 8, 1), 6);
        assert_eq!(motion_line_end(content, 0, 1), 5);
    }

    #[test]
    fn test_motion_doc_start_end() {
        assert_eq!(motion_doc_start("hello", 3, 1), 0);
        assert_eq!(motion_doc_end("hello", 0, 1), 5);
    }

    #[test]
    fn test_matching_bracket() {
        assert_eq!(motion_matching_bracket("(hello)", 0, 1), 6);
        assert_eq!(motion_matching_bracket("(hello)", 6, 1), 0);
        // No bracket at cursor
        assert_eq!(motion_matching_bracket("hello", 0, 1), 0);
    }
}
