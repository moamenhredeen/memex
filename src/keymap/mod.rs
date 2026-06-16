#[allow(dead_code)]
mod action;
#[allow(dead_code)]
mod binding;
mod context;
#[allow(dead_code)]
pub mod defaults;
#[allow(dead_code)]
mod grammar;

use serde::Deserialize;

pub use action::*;
pub use binding::*;
pub use context::*;
pub use defaults::*;
pub use grammar::*;

// ─── ResolvedKey ─────────────────────────────────────────────────────────────

/// Result of resolving a key through the keymap system.
#[derive(Clone, Debug)]
pub enum ResolvedKey {
    /// A bound action was matched, with accumulated count.
    Action(Action, usize),
    /// Transient vim action captured a character (f/t/r prefix).
    TransientCapture {
        transient: TransientKind,
        ch: char,
        count: usize,
    },
    /// Count digit accumulated or multi-key sequence in progress.
    Pending,
    /// No binding found — key was not consumed.
    Unhandled,
}

// ─── KeymapSystem ────────────────────────────────────────────────────────────

/// App-level keymap system: owns bindings, Vim mode state, and pending key state.
pub struct KeymapSystem {
    pub registry: BindingRegistry,
    pub vim_enabled: bool,
    vim_mode: VimMode,
    transient: Option<TransientKind>,
    /// Count digit accumulator (e.g., `3j` → count=3, action=Motion("down")).
    count: Option<usize>,
    /// When a multi-key sequence is in progress, holds the trie node we're at.
    pending_trie: Option<std::collections::HashMap<KeyCombo, KeyTrie>>,
}

impl KeymapSystem {
    pub fn new(vim_enabled: bool) -> Self {
        let mut registry = BindingRegistry::new();
        defaults::register_defaults(&mut registry);
        load_user_keymap(&mut registry);

        Self {
            registry,
            vim_enabled,
            vim_mode: if vim_enabled {
                VimMode::Normal
            } else {
                VimMode::Insert
            },
            transient: None,
            count: None,
            pending_trie: None,
        }
    }

    /// Check if a multi-key sequence is in progress.
    #[allow(dead_code)]
    pub fn has_pending_keys(&self) -> bool {
        self.pending_trie.is_some()
    }

    /// Cancel any pending multi-key sequence.
    #[allow(dead_code)]
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

    /// Resolve a key event through the active key context.
    /// Returns a `ResolvedKey`; buffer state is only represented by context.
    pub fn resolve_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
        context: &KeyContext,
    ) -> ResolvedKey {
        // 1. Multi-key sequence continuation
        if let Some(pending_map) = self.pending_trie.take() {
            let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
            if let Some(trie_node) = pending_map.get(&combo) {
                match trie_node {
                    KeyTrie::Leaf(action, _, _) => {
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

        // 2. Transient capture (f/t/r) — grammar is waiting for a character
        if let Some(transient) = self.transient.take() {
            let ch = match key.chars().next() {
                Some(c) if key.chars().count() == 1 && key != "escape" => c,
                _ => {
                    self.count = None;
                    return ResolvedKey::Unhandled;
                }
            };
            let count = self.take_count();
            return ResolvedKey::TransientCapture {
                transient,
                ch,
                count,
            };
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

        // 4. Context binding resolution
        let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
        let resolved = self.registry.resolve(&combo, context);

        match resolved {
            Some(KeyTrie::Leaf(action, _, _)) => {
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
                // Unknown keys in vim command modes are consumed as no-ops.
                // Text insertion belongs exclusively to the OS input path while
                // insert mode is active.
                if self.vim_enabled && !self.is_insert_active() {
                    self.count = None;
                    return ResolvedKey::Action(Action::Noop, 1);
                }
                self.count = None;
                ResolvedKey::Unhandled
            }
        }
    }

    /// Toggle vim mode on/off.
    pub fn set_vim_enabled(&mut self, enabled: bool) {
        self.vim_enabled = enabled;
        if !enabled {
            self.vim_mode = VimMode::Insert;
            self.pending_trie = None;
            self.transient = None;
        } else if matches!(self.vim_mode, VimMode::Insert) {
            self.vim_mode = VimMode::Normal;
        }
    }

    pub fn set_vim_mode(&mut self, mode: VimMode) {
        self.vim_mode = mode;
        self.pending_trie = None;
        self.transient = None;
    }

    pub fn push_transient(&mut self, transient: TransientKind) {
        self.transient = Some(transient);
    }

    /// Get the label for the current vim state (for mode-line).
    pub fn active_vim_state(&self) -> Option<&str> {
        if !self.vim_enabled {
            return None;
        }
        Some(self.vim_mode.label())
    }

    pub fn active_vim_mode(&self) -> &'static str {
        if !self.vim_enabled {
            return "insert";
        }
        self.vim_mode.as_context_value()
    }

    /// Check if insert mode is active (for input handler decisions).
    pub fn is_insert_active(&self) -> bool {
        if !self.vim_enabled {
            return true;
        }
        self.vim_mode == VimMode::Insert
    }

    /// Check if a visual mode is active.
    pub fn is_visual_active(&self) -> bool {
        matches!(self.vim_mode, VimMode::Visual | VimMode::VisualLine)
    }

    /// Check if visual-line mode is active.
    #[allow(dead_code)]
    pub fn is_visual_line_active(&self) -> bool {
        self.vim_mode == VimMode::VisualLine
    }
}

#[derive(Debug, Deserialize)]
struct KeymapFile {
    #[serde(default)]
    bindings: Vec<UserBinding>,
}

#[derive(Debug, Deserialize)]
struct UserBinding {
    context: Option<String>,
    key: String,
    command: String,
}

fn load_user_keymap(registry: &mut BindingRegistry) {
    let Some(config_dir) = dirs::config_dir() else {
        return;
    };
    let path = config_dir.join("memex").join("keymap.toml");
    if !path.exists() {
        return;
    }
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("keymap error: failed to read {}: {error}", path.display());
            return;
        }
    };
    load_user_keymap_source(registry, &source, &path.display().to_string());
}

fn load_user_keymap_source(registry: &mut BindingRegistry, source: &str, label: &str) {
    let file: KeymapFile = match toml::from_str(source) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("keymap error: invalid TOML in {label}: {error}");
            return;
        }
    };
    for binding in file.bindings {
        let predicate = match binding.context.as_deref() {
            Some(context) => match KeyPredicate::parse(context) {
                Ok(predicate) => predicate,
                Err(error) => {
                    eprintln!("keymap error: invalid context {context:?}: {error}");
                    continue;
                }
            },
            None => KeyPredicate::always(),
        };
        registry.bind(
            &binding.key,
            predicate,
            Action::Command(Box::leak(binding.command.into_boxed_str())),
        );
    }
}

// ─── Find/Til char helper functions ──────────────────────────────────────────

pub fn find_char_forward(content: &str, cursor: usize, ch: char, count: usize) -> usize {
    let mut cur = cursor.min(content.len());
    while cur > 0 && !content.is_char_boundary(cur) {
        cur -= 1;
    }
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
    while c > 0 && !content.is_char_boundary(c) {
        c -= 1;
    }
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

    fn editor_context(mode: &'static str) -> KeyContext {
        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", mode);
        context
    }

    #[test]
    fn test_resolve_i_enters_insert() {
        let mut ks = KeymapSystem::new(true);
        let context = editor_context("normal");
        let result = ks.resolve_key("i", false, false, false, &context);
        match result {
            ResolvedKey::Action(Action::SetVimMode(VimMode::Insert), _) => {}
            other => panic!("Expected SetVimMode(insert), got {:?}", other),
        }
    }

    #[test]
    fn test_unbound_normal_key_is_consumed_without_inserting() {
        let mut ks = KeymapSystem::new(true);
        let context = editor_context("normal");
        let result = ks.resolve_key("z", false, false, false, &context);
        match result {
            ResolvedKey::Action(Action::Noop, _) => {}
            other => panic!("Expected Noop fallback, got {:?}", other),
        }
    }

    #[test]
    fn first_printable_key_after_entering_insert_is_unhandled() {
        let mut ks = KeymapSystem::new(true);
        let context = editor_context("normal");
        assert!(matches!(
            ks.resolve_key("i", false, false, false, &context),
            ResolvedKey::Action(Action::SetVimMode(VimMode::Insert), _)
        ));
        ks.set_vim_mode(VimMode::Insert);
        let context = editor_context("insert");

        assert!(matches!(
            ks.resolve_key("x", false, false, false, &context),
            ResolvedKey::Unhandled
        ));
    }

    /// The editor's keymap has PDF and graph bindings registered globally, but
    /// they require different contexts and must not leak into editor-focused
    /// resolution.
    #[test]
    fn test_editor_keymap_does_not_resolve_pdf_bindings() {
        let mut ks = KeymapSystem::new(true);
        // 'j' in a PDF-only context maps to pdf-scroll-down. In the editor,
        // vim:normal binds 'j' to Motion("down"). Make sure we get the
        // motion, never the pdf command.
        let context = editor_context("normal");
        let result = ks.resolve_key("j", false, false, false, &context);
        match result {
            ResolvedKey::Action(Action::Motion("down"), _) => {}
            ResolvedKey::Action(Action::Command(cmd), _) if cmd.starts_with("pdf-") => {
                panic!("editor keymap resolved to a pdf command: {}", cmd);
            }
            other => panic!("expected Motion(down), got {:?}", other),
        }
    }

    /// In insert mode, the editor keymap must not consume `+` / `c` / `q` —
    /// those are graph keys. An unbound key in insert mode should stay
    /// unhandled so the OS input path can insert the character.
    #[test]
    fn test_editor_insert_does_not_eat_graph_keys() {
        let mut ks = KeymapSystem::new(true);
        ks.set_vim_mode(VimMode::Insert);
        let context = editor_context("insert");
        for key in ["+", "c", "q", "l"] {
            let result = ks.resolve_key(key, false, false, false, &context);
            match result {
                ResolvedKey::Unhandled => {}
                ResolvedKey::Action(Action::Command(cmd), _)
                    if cmd == "zoom-in" || cmd == "center-graph" =>
                {
                    panic!("editor insert keymap leaked graph command {}", cmd);
                }
                _ => {} // fine — other bindings are editor-specific
            }
        }
    }

    #[test]
    fn tab_context_prefers_code_block_indent_over_markdown_outline() {
        let mut ks = KeymapSystem::new(true);
        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", "normal");
        context.add("code_block");
        context.add("heading");

        let result = ks.resolve_key("tab", false, false, false, &context);
        assert!(matches!(
            result,
            ResolvedKey::Action(Action::Command("insert-tab"), _)
        ));
    }

    #[test]
    fn tab_context_routes_tables_before_outline() {
        let mut ks = KeymapSystem::new(true);
        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", "normal");
        context.add("table");
        context.add("heading");

        let result = ks.resolve_key("tab", false, false, false, &context);
        assert!(matches!(
            result,
            ResolvedKey::Action(Action::Command("table-next-cell"), _)
        ));
    }

    #[test]
    fn tab_context_routes_headings_to_outline() {
        let mut ks = KeymapSystem::new(true);
        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", "normal");
        context.add("heading");

        let result = ks.resolve_key("tab", false, false, false, &context);
        assert!(matches!(
            result,
            ResolvedKey::Action(Action::Command("outline-cycle-fold"), _)
        ));
    }

    #[test]
    fn context_expression_supports_flags_values_and_boolean_ops() {
        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", "normal");
        context.add("code_block");

        assert!(
            KeyPredicate::parse("Editor && vim_mode == normal")
                .unwrap()
                .matches(&context)
        );
        assert!(
            KeyPredicate::parse("Editor && vim_mode != insert")
                .unwrap()
                .matches(&context)
        );
        assert!(
            KeyPredicate::parse("Pdf || (Editor && !table)")
                .unwrap()
                .matches(&context)
        );
        assert!(
            !KeyPredicate::parse("Editor && table")
                .unwrap()
                .matches(&context)
        );
    }

    #[test]
    fn more_specific_context_binding_beats_later_generic_binding() {
        let mut registry = BindingRegistry::new();
        registry.bind(
            "tab",
            KeyPredicate::parse("Editor && code_block").unwrap(),
            Action::Command("insert-tab"),
        );
        registry.bind(
            "tab",
            KeyPredicate::parse("Editor").unwrap(),
            Action::Command("outline-cycle-fold"),
        );

        let mut context = KeyContext::new();
        context.add("Editor");
        context.add("code_block");

        let combo = KeyCombo::parse("tab");
        assert!(matches!(
            registry.resolve(&combo, &context),
            Some(KeyTrie::Leaf(Action::Command("insert-tab"), _, _))
        ));
    }

    #[test]
    fn later_same_specificity_binding_wins() {
        let mut registry = BindingRegistry::new();
        registry.bind("ctrl-p", when().require("Editor"), Action::Command("old"));
        registry.bind("ctrl-p", when().require("Editor"), Action::Command("new"));

        let mut context = KeyContext::new();
        context.add("Editor");

        let combo = KeyCombo::parse("ctrl-p");
        assert!(matches!(
            registry.resolve(&combo, &context),
            Some(KeyTrie::Leaf(Action::Command("new"), _, _))
        ));
    }

    #[test]
    fn user_keymap_toml_adds_contextual_command_binding() {
        let mut registry = BindingRegistry::new();
        load_user_keymap_source(
            &mut registry,
            r#"
                [[bindings]]
                context = "Editor && vim_mode == normal"
                key = "space x"
                command = "custom-command"
            "#,
            "test keymap",
        );

        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", "normal");

        let combo = KeyCombo::parse("space");
        let Some(KeyTrie::Node(node)) = registry.resolve(&combo, &context) else {
            panic!("expected space prefix");
        };
        let x = KeyCombo::parse("x");
        assert!(matches!(
            node.get(&x),
            Some(KeyTrie::Leaf(Action::Command("custom-command"), _, _))
        ));
    }
}
