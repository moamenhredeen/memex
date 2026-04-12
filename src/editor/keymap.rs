use std::collections::HashMap;

use super::commands::EditorCommand;

/// Editor mode — determines which keybindings are active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorMode {
    /// Default editing mode (current behavior).
    Insert,
    /// Vim normal mode — commands, motions, operators.
    Normal,
    /// Vim visual mode — character-wise selection.
    Visual,
    /// Vim visual line mode — line-wise selection.
    VisualLine,
}

impl EditorMode {
    pub fn label(&self) -> &'static str {
        match self {
            EditorMode::Insert => "INSERT",
            EditorMode::Normal => "NORMAL",
            EditorMode::Visual => "VISUAL",
            EditorMode::VisualLine => "V-LINE",
        }
    }
}

/// A key combination (e.g. "ctrl-z", "shift-left", "d", "g").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    /// Parse from a string like "ctrl-z", "shift-left", "a", "enter".
    pub fn parse(s: &str) -> Self {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key = String::new();

        let parts: Vec<&str> = s.split('-').collect();
        for (i, part) in parts.iter().enumerate() {
            if i < parts.len() - 1 {
                match part.to_lowercase().as_str() {
                    "ctrl" | "control" => ctrl = true,
                    "shift" => shift = true,
                    "alt" | "meta" => alt = true,
                    _ => {
                        // Not a modifier, must be part of key name (e.g. "shift-left")
                        // Handle by joining the rest
                        key = parts[i..].join("-");
                        return Self {
                            key,
                            ctrl,
                            shift,
                            alt,
                        };
                    }
                }
            } else {
                key = part.to_string();
            }
        }

        Self {
            key,
            ctrl,
            shift,
            alt,
        }
    }

    /// Create from gpui keystroke event data.
    pub fn from_keystroke(key: &str, ctrl: bool, shift: bool, alt: bool) -> Self {
        Self {
            key: key.to_string(),
            ctrl,
            shift,
            alt,
        }
    }
}

/// A single key binding: a key combo mapped to a command in a specific mode.
#[derive(Clone, Debug)]
pub struct KeyBinding {
    pub key: KeyCombo,
    pub command: EditorCommand,
    /// Optional context guard (e.g. "in_table"). Future use.
    pub context: Option<String>,
}

/// The keymap: maps modes to their key bindings.
pub struct Keymap {
    bindings: HashMap<EditorMode, Vec<KeyBinding>>,
}

impl Keymap {
    pub fn new() -> Self {
        let mut km = Self {
            bindings: HashMap::new(),
        };
        km.load_defaults();
        km
    }

    /// Load the default keybindings for Insert mode.
    fn load_defaults(&mut self) {
        let insert = vec![
            // Undo/Redo
            kb("ctrl-z", EditorCommand::Undo),
            kb("ctrl-shift-z", EditorCommand::Redo),
            // Movement
            kb("left", EditorCommand::MoveLeft),
            kb("right", EditorCommand::MoveRight),
            kb("up", EditorCommand::MoveUp),
            kb("down", EditorCommand::MoveDown),
            kb("home", EditorCommand::MoveLineStart),
            kb("end", EditorCommand::MoveLineEnd),
            // Selection
            kb("shift-left", EditorCommand::SelectLeft),
            kb("shift-right", EditorCommand::SelectRight),
            // Editing
            kb("backspace", EditorCommand::DeleteBackward),
            kb("delete", EditorCommand::DeleteForward),
            kb("enter", EditorCommand::InsertNewline),
            // Vim toggle
            kb("ctrl-shift-v", EditorCommand::ToggleVimMode),
        ];

        self.bindings.insert(EditorMode::Insert, insert);
    }

    /// Look up a command for a key combo in a given mode.
    pub fn resolve(
        &self,
        mode: EditorMode,
        key: &KeyCombo,
    ) -> Option<EditorCommand> {
        if let Some(bindings) = self.bindings.get(&mode) {
            for binding in bindings {
                if binding.key == *key {
                    return Some(binding.command.clone());
                }
            }
        }
        None
    }

    /// Add or override a binding for a mode.
    pub fn bind(&mut self, mode: EditorMode, key_str: &str, command: EditorCommand) {
        let key = KeyCombo::parse(key_str);
        let bindings = self.bindings.entry(mode).or_default();
        // Remove existing binding for this key if present
        bindings.retain(|b| b.key != key);
        bindings.push(KeyBinding {
            key,
            command,
            context: None,
        });
    }

    /// Remove a binding for a mode.
    pub fn unbind(&mut self, mode: EditorMode, key_str: &str) {
        let key = KeyCombo::parse(key_str);
        if let Some(bindings) = self.bindings.get_mut(&mode) {
            bindings.retain(|b| b.key != key);
        }
    }
}

/// Helper to create a KeyBinding from a key string.
fn kb(key_str: &str, command: EditorCommand) -> KeyBinding {
    KeyBinding {
        key: KeyCombo::parse(key_str),
        command,
        context: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_combo_parse_simple() {
        let k = KeyCombo::parse("a");
        assert_eq!(k.key, "a");
        assert!(!k.ctrl);
        assert!(!k.shift);
    }

    #[test]
    fn test_key_combo_parse_ctrl() {
        let k = KeyCombo::parse("ctrl-z");
        assert_eq!(k.key, "z");
        assert!(k.ctrl);
        assert!(!k.shift);
    }

    #[test]
    fn test_key_combo_parse_ctrl_shift() {
        let k = KeyCombo::parse("ctrl-shift-z");
        assert_eq!(k.key, "z");
        assert!(k.ctrl);
        assert!(k.shift);
    }

    #[test]
    fn test_key_combo_parse_shift_left() {
        let k = KeyCombo::parse("shift-left");
        assert_eq!(k.key, "left");
        assert!(!k.ctrl);
        assert!(k.shift);
    }

    #[test]
    fn test_default_keymap_resolves() {
        let km = Keymap::new();
        let key = KeyCombo::parse("ctrl-z");
        let cmd = km.resolve(EditorMode::Insert, &key);
        assert_eq!(cmd, Some(EditorCommand::Undo));
    }

    #[test]
    fn test_bind_override() {
        let mut km = Keymap::new();
        km.bind(EditorMode::Insert, "ctrl-z", EditorCommand::Redo);
        let key = KeyCombo::parse("ctrl-z");
        let cmd = km.resolve(EditorMode::Insert, &key);
        assert_eq!(cmd, Some(EditorCommand::Redo));
    }

    #[test]
    fn test_unbind() {
        let mut km = Keymap::new();
        km.unbind(EditorMode::Insert, "ctrl-z");
        let key = KeyCombo::parse("ctrl-z");
        let cmd = km.resolve(EditorMode::Insert, &key);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_no_match_returns_none() {
        let km = Keymap::new();
        let key = KeyCombo::parse("f12");
        let cmd = km.resolve(EditorMode::Insert, &key);
        assert_eq!(cmd, None);
    }
}
