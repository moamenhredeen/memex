mod action;
mod defaults;
mod grammar;
mod layer;

pub use action::*;
pub use defaults::*;
pub use grammar::*;
pub use layer::*;

/// The unified keymap system: owns layers, grammar engine, and resolution logic.
pub struct KeymapSystem {
    pub stack: LayerStack,
    pub grammar: VimGrammar,
    pub vim_enabled: bool,
}

impl KeymapSystem {
    pub fn new(vim_enabled: bool) -> Self {
        let mut stack = LayerStack::new();
        defaults::register_defaults(&mut stack);

        if vim_enabled {
            stack.activate_layer("vim:normal");
        }
        stack.activate_layer("markdown");
        stack.activate_layer("global");

        Self {
            stack,
            grammar: VimGrammar::new(),
            vim_enabled,
        }
    }

    /// Resolve a key event through the layer stack and grammar engine.
    /// Returns a GrammarResult describing what to do.
    pub fn process_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
        content: &str,
        cursor: usize,
    ) -> GrammarResult {
        // 1. Transient capture — if a transient layer (f/t/r/g) is waiting for input
        if let Some(transient_id) = self.stack.peek_transient() {
            self.stack.pop_transient();
            return self.handle_transient_capture(transient_id, key, content, cursor);
        }

        // 2. Count digit accumulation in vim normal/visual/op-pending
        if self.vim_enabled && !ctrl && !alt && key.chars().count() == 1 {
            if let Some(ch) = key.chars().next() {
                if ch.is_ascii_digit() && !self.is_insert_active() {
                    let digit = (ch as u8) - b'0';
                    // "0" is line-start unless count already started or operator pending
                    if digit > 0 || self.grammar.count.is_some() || self.grammar.pending_operator.is_some() {
                        self.grammar.push_count_digit(digit);
                        return GrammarResult::Pending;
                    }
                }
            }
        }

        // 3. Normal layer resolution
        let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
        let action = self.stack.resolve(&combo);

        // If no action found, check if it's a printable self-insert
        let action = action.unwrap_or_else(|| {
            if !ctrl && !alt && key.chars().count() == 1 {
                Action::SelfInsert
            } else {
                Action::Noop
            }
        });

        // 4. Pass through grammar engine
        self.grammar.process(action, key, content, cursor, &mut self.stack)
    }

    /// Handle a key captured by a transient layer (f/t/r/g prefix).
    fn handle_transient_capture(
        &mut self,
        transient_id: LayerId,
        key: &str,
        content: &str,
        cursor: usize,
    ) -> GrammarResult {
        // Escape or non-single-char cancels the transient
        let ch = match key.chars().next() {
            Some(c) if key.chars().count() == 1 && key != "escape" => c,
            _ => {
                self.grammar.clear_pending();
                return GrammarResult::Noop;
            }
        };

        let count = self.grammar.effective_count();

        match transient_id {
            "transient:find-char" => {
                self.grammar.last_char_search = Some((ch, "find-char"));
                let target = find_char_forward(content, cursor, ch, count);
                self.grammar.resolve_with_target(target, content, cursor)
            }
            "transient:til-char" => {
                self.grammar.last_char_search = Some((ch, "til-char"));
                let target = til_char_forward(content, cursor, ch, count);
                self.grammar.resolve_with_target(target, content, cursor)
            }
            "transient:find-char-back" => {
                self.grammar.last_char_search = Some((ch, "find-char-back"));
                let target = find_char_backward(content, cursor, ch, count);
                self.grammar.resolve_with_target(target, content, cursor)
            }
            "transient:til-char-back" => {
                self.grammar.last_char_search = Some((ch, "til-char-back"));
                let target = til_char_backward(content, cursor, ch, count);
                self.grammar.resolve_with_target(target, content, cursor)
            }
            "transient:replace-char" => {
                let c = count;
                self.grammar.clear_pending();
                GrammarResult::ReplaceChar { ch, count: c }
            }
            "transient:g-prefix" => {
                match ch {
                    'g' => {
                        // gg = go to doc start
                        self.grammar.resolve_with_target(0, content, cursor)
                    }
                    '_' => {
                        // g_ = last non-whitespace on line (treat as line-end for now)
                        let target = motion_line_end(content, cursor, 1);
                        self.grammar.resolve_with_target(target, content, cursor)
                    }
                    _ => {
                        self.grammar.clear_pending();
                        GrammarResult::Noop
                    }
                }
            }
            _ => {
                self.grammar.clear_pending();
                GrammarResult::Noop
            }
        }
    }

    /// Toggle vim mode on/off.
    pub fn set_vim_enabled(&mut self, enabled: bool) {
        self.vim_enabled = enabled;
        if enabled {
            self.stack.activate_layer("vim:normal");
        } else {
            self.stack.deactivate_group("vim-state");
        }
    }

    /// Get the label for the current vim state (for mode-line).
    pub fn active_vim_state(&self) -> Option<&str> {
        if !self.vim_enabled {
            return None;
        }
        for layer_id in self.stack.active_layers() {
            match *layer_id {
                "vim:normal" => return Some("NORMAL"),
                "vim:insert" => return Some("INSERT"),
                "vim:visual" => return Some("VISUAL"),
                "vim:visual-line" => return Some("V-LINE"),
                "vim:op-pending" => return Some("NORMAL"),
                _ => {}
            }
        }
        Some("NORMAL")
    }

    /// Check if insert-layer is active (for input handler decisions).
    pub fn is_insert_active(&self) -> bool {
        if !self.vim_enabled {
            return true;
        }
        self.stack.active_layers().iter().any(|id| *id == "vim:insert")
    }

    /// Check if a visual layer is active.
    pub fn is_visual_active(&self) -> bool {
        self.stack.active_layers().iter().any(|id| *id == "vim:visual" || *id == "vim:visual-line")
    }

    /// Check if visual-line layer is active.
    pub fn is_visual_line_active(&self) -> bool {
        self.stack.active_layers().iter().any(|id| *id == "vim:visual-line")
    }
}

// ─── Find/Til char helper functions ──────────────────────────────────────────

fn find_char_forward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let after = &content[cursor..];
    let mut found = 0usize;
    let mut pos = cursor;
    for (i, c) in after.char_indices().skip(1) {
        if c == ch {
            found += 1;
            if found == count {
                pos = cursor + i;
                break;
            }
        }
    }
    pos
}

fn til_char_forward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let target = find_char_forward(content, cursor, ch, count);
    if target > cursor {
        // Back up one char
        let mut p = target;
        if p > 0 {
            p -= 1;
            while p > cursor && !content.is_char_boundary(p) {
                p -= 1;
            }
        }
        p
    } else {
        cursor
    }
}

fn find_char_backward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let before = &content[..cursor];
    let mut found = 0usize;
    let mut pos = cursor;
    for (i, c) in before.char_indices().rev() {
        if c == ch {
            found += 1;
            if found == count {
                pos = i;
                break;
            }
        }
    }
    pos
}

fn til_char_backward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let target = find_char_backward(content, cursor, ch, count);
    if target < cursor {
        // Advance one char
        let mut p = target + 1;
        while p < cursor && !content.is_char_boundary(p) {
            p += 1;
        }
        p
    } else {
        cursor
    }
}

/// Repeat the last char search (;) — reuses last_char_search from grammar.
pub fn repeat_char_search(
    grammar: &VimGrammar,
    content: &str,
    cursor: usize,
    count: usize,
) -> Option<usize> {
    let (ch, kind) = grammar.last_char_search?;
    let target = match kind {
        "find-char" => find_char_forward(content, cursor, ch, count),
        "til-char" => til_char_forward(content, cursor, ch, count),
        "find-char-back" => find_char_backward(content, cursor, ch, count),
        "til-char-back" => til_char_backward(content, cursor, ch, count),
        _ => return None,
    };
    Some(target)
}

/// Repeat the last char search in reverse (,).
pub fn repeat_char_search_reverse(
    grammar: &VimGrammar,
    content: &str,
    cursor: usize,
    count: usize,
) -> Option<usize> {
    let (ch, kind) = grammar.last_char_search?;
    let target = match kind {
        "find-char" => find_char_backward(content, cursor, ch, count),
        "til-char" => til_char_backward(content, cursor, ch, count),
        "find-char-back" => find_char_forward(content, cursor, ch, count),
        "til-char-back" => til_char_forward(content, cursor, ch, count),
        _ => return None,
    };
    Some(target)
}
