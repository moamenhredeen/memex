#[allow(dead_code)]
mod action;
#[allow(dead_code)]
pub mod defaults;
#[allow(dead_code)]
mod grammar;
#[allow(dead_code)]
mod layer;

pub use action::*;
pub use defaults::*;
pub use grammar::*;
pub use layer::*;

// ─── ResolvedKey ─────────────────────────────────────────────────────────────

/// Result of resolving a key through the keymap system.
/// This is context-free — no buffer content or cursor needed.
#[derive(Clone, Debug)]
pub enum ResolvedKey {
    /// A bound action was matched, with accumulated count.
    Action(Action, usize),
    /// Transient layer captured a character (f/t/r prefix in vim).
    TransientCapture {
        transient_id: LayerId,
        ch: char,
        count: usize,
    },
    /// Count digit accumulated or multi-key sequence in progress.
    Pending,
    /// No binding found — key was not consumed.
    Unhandled,
}

// ─── KeymapSystem ────────────────────────────────────────────────────────────

/// App-level keymap system: owns layers, resolves keys to actions.
///
/// Following the Emacs model: keymaps are pure data (key → action).
/// The keymap system knows nothing about buffer content, cursor positions,
/// or view-specific state. Views receive resolved actions and handle them.
///
/// Lookup order (highest → lowest priority):
/// 1. Transient layers (f/t/r char captures — auto-pop after one key)
/// 2. Active layers in stack order (e.g., vim:normal > leader > markdown > global)
/// 3. Unhandled fallback
pub struct KeymapSystem {
    pub stack: LayerStack,
    pub vim_enabled: bool,
    /// Count digit accumulator (e.g., `3j` → count=3, action=Motion("down")).
    count: Option<usize>,
    /// When a multi-key sequence is in progress, holds the trie node we're at.
    pending_trie: Option<std::collections::HashMap<KeyCombo, KeyTrie>>,
}

impl KeymapSystem {
    pub fn new(vim_enabled: bool) -> Self {
        let mut stack = LayerStack::new();
        defaults::register_defaults(&mut stack);

        if vim_enabled {
            stack.activate_layer("vim:normal");
            stack.activate_layer("leader");
        }
        stack.activate_layer("global");
        stack.activate_layer("markdown");

        Self {
            stack,
            vim_enabled,
            count: None,
            pending_trie: None,
        }
    }

    /// Check if a multi-key sequence is in progress.
    pub fn has_pending_keys(&self) -> bool {
        self.pending_trie.is_some()
    }

    /// Cancel any pending multi-key sequence.
    pub fn cancel_pending(&mut self) {
        self.pending_trie = None;
    }

    /// Consume and return the accumulated count, defaulting to 1.
    pub fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1)
    }

    /// Push a digit to the count accumulator.
    fn push_count_digit(&mut self, digit: u8) {
        let current = self.count.unwrap_or(0);
        self.count = Some(current * 10 + digit as usize);
    }

    /// Resolve a key event through the layer stack.
    /// Returns a `ResolvedKey` — context-free, no buffer state needed.
    pub fn resolve_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> ResolvedKey {
        // 1. Multi-key sequence continuation
        if let Some(pending_map) = self.pending_trie.take() {
            let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
            if let Some(trie_node) = pending_map.get(&combo) {
                match trie_node {
                    KeyTrie::Leaf(action) => {
                        let count = self.take_count();
                        return ResolvedKey::Action(action.clone(), count);
                    }
                    KeyTrie::Node(map) => {
                        self.pending_trie = Some(map.clone());
                        return ResolvedKey::Pending;
                    }
                }
            } else {
                // Key doesn't match any continuation — cancel sequence
                self.count = None;
                return ResolvedKey::Unhandled;
            }
        }

        // 2. Transient capture (f/t/r) — layer is waiting for a character
        if let Some(transient_id) = self.stack.peek_transient() {
            self.stack.pop_transient();
            let ch = match key.chars().next() {
                Some(c) if key.chars().count() == 1 && key != "escape" => c,
                _ => {
                    self.count = None;
                    return ResolvedKey::Unhandled;
                }
            };
            let count = self.take_count();
            return ResolvedKey::TransientCapture { transient_id, ch, count };
        }

        // 3. Count digit accumulation (vim normal/visual/op-pending)
        if self.vim_enabled && !ctrl && !alt && key.chars().count() == 1 {
            if let Some(ch) = key.chars().next() {
                if ch.is_ascii_digit() && !self.is_insert_active() {
                    let digit = (ch as u8) - b'0';
                    // "0" alone is a motion (line-start); digits after first are count
                    if digit > 0 || self.count.is_some() {
                        self.push_count_digit(digit);
                        return ResolvedKey::Pending;
                    }
                }
            }
        }

        // 4. Layer stack resolution
        let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
        let resolved = self.stack.resolve(&combo).cloned();

        match resolved {
            Some(KeyTrie::Leaf(action)) => {
                let count = self.take_count();
                ResolvedKey::Action(action, count)
            }
            Some(KeyTrie::Node(map)) => {
                // In insert mode, trie prefixes (leader keys) should not activate —
                // let the key fall through as unhandled (OS input handler inserts it).
                if self.is_insert_active() {
                    self.count = None;
                    return ResolvedKey::Unhandled;
                }
                // Start of a multi-key sequence — enter pending state
                self.pending_trie = Some(map.clone());
                ResolvedKey::Pending
            }
            None => {
                // In vim non-insert modes, unbound single-char keys become SelfInsert
                // so the grammar can handle them (e.g. i→insert, a→append, o→open-line)
                if self.vim_enabled && !self.is_insert_active() && !ctrl && !alt {
                    if let Some(_ch) = key.chars().next() {
                        if key.chars().count() == 1 {
                            let count = self.take_count();
                            return ResolvedKey::Action(Action::SelfInsert, count);
                        }
                    }
                }
                self.count = None;
                ResolvedKey::Unhandled
            }
        }
    }

    /// Toggle vim mode on/off.
    pub fn set_vim_enabled(&mut self, enabled: bool) {
        self.vim_enabled = enabled;
        if enabled {
            self.stack.activate_layer("vim:normal");
            self.stack.activate_layer("leader");
        } else {
            self.stack.deactivate_group("vim-state");
            self.stack.deactivate_layer("leader");
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
    #[allow(dead_code)]
    pub fn is_visual_line_active(&self) -> bool {
        self.stack.active_layers().iter().any(|id| *id == "vim:visual-line")
    }
}

// ─── Find/Til char helper functions ──────────────────────────────────────────

pub fn find_char_forward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let mut cur = cursor.min(content.len());
    while cur > 0 && !content.is_char_boundary(cur) { cur -= 1; }
    let after = &content[cur..];
    let mut found = 0usize;
    let mut pos = cur;
    for (i, c) in after.char_indices().skip(1) {
        if c == ch {
            found += 1;
            if found == count {
                pos = cur + i;
                break;
            }
        }
    }
    pos
}

pub fn til_char_forward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let target = find_char_forward(content, cursor, ch, count);
    if target > cursor {
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

pub fn find_char_backward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let mut c = cursor.min(content.len());
    while c > 0 && !content.is_char_boundary(c) { c -= 1; }
    let before = &content[..c];
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

pub fn til_char_backward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let target = find_char_backward(content, cursor, ch, count);
    if target < cursor {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_i_enters_insert() {
        let mut ks = KeymapSystem::new(true);
        let result = ks.resolve_key("i", false, false, false);
        match result {
            ResolvedKey::Action(Action::ActivateLayer("vim:insert"), _) => {}
            other => panic!("Expected ActivateLayer(vim:insert), got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_unbound_key_falls_to_self_insert() {
        let mut ks = KeymapSystem::new(true);
        // 'z' is not bound as a leaf in any active layer, should become SelfInsert
        let result = ks.resolve_key("z", false, false, false);
        match result {
            ResolvedKey::Action(Action::SelfInsert, _) => {}
            other => panic!("Expected SelfInsert fallback, got {:?}", other),
        }
    }
}
