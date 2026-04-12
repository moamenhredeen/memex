/// Identifies a registered command.
pub type CommandId = &'static str;
/// Identifies a registered motion.
pub type MotionId = &'static str;
/// Identifies a registered operator.
pub type OperatorId = &'static str;
/// Identifies a layer.
pub type LayerId = &'static str;
/// Identifies a layer group (mutually exclusive layers).
pub type LayerGroupId = &'static str;

/// What a key binding produces when matched.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Execute a registered command immediately.
    Command(CommandId),
    /// A cursor motion (participates in vim grammar — composable with operators).
    Motion(MotionId),
    /// An operator that needs a motion/range to complete (d, c, y, >, <).
    Operator(OperatorId),
    /// Insert the key's character at cursor.
    SelfInsert,
    /// Switch to a different layer (deactivating others in the same group).
    ActivateLayer(LayerId),
    /// Push a transient layer that auto-pops after one key resolution.
    /// The string is the transient layer id to push.
    PushTransient(LayerId),
    /// Call a Rhai script function by name.
    Script(String),
    /// Execute multiple actions in sequence.
    Sequence(Vec<Action>),
    /// Consume the key but do nothing.
    Noop,
}

/// A key combination: key name + modifier flags.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    /// Create from gpui keystroke event data.
    pub fn from_keystroke(key: &str, ctrl: bool, shift: bool, alt: bool) -> Self {
        Self {
            key: key.to_string(),
            ctrl,
            shift,
            alt,
        }
    }

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
                        // Not a modifier — rest is the key name (e.g. "shift-left")
                        key = parts[i..].join("-");
                        return Self { key, ctrl, shift, alt };
                    }
                }
            } else {
                key = part.to_string();
            }
        }

        Self { key, ctrl, shift, alt }
    }
}

/// Motion function signature: (content, cursor, count) → target byte offset.
pub type NativeMotionFn = fn(&str, usize, usize) -> usize;

/// Operator function signature: (content, range_start, range_end) → OperatorOutput.
pub type NativeOperatorFn = fn(&str, usize, usize) -> OperatorOutput;

/// Result of applying an operator to a range.
#[derive(Clone, Debug)]
pub struct OperatorOutput {
    /// Range to delete (empty if yank-only).
    pub delete_range: Option<std::ops::Range<usize>>,
    /// Text that was yanked/copied.
    pub yanked: String,
    /// Whether to enter insert mode after the operation.
    pub enter_insert: bool,
}

/// A motion implementation — native Rust or Rhai script.
#[derive(Clone)]
pub enum MotionImpl {
    Native(NativeMotionFn),
    Script(String),
}

impl std::fmt::Debug for MotionImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MotionImpl::Native(_) => write!(f, "MotionImpl::Native(fn)"),
            MotionImpl::Script(s) => write!(f, "MotionImpl::Script({:?})", s),
        }
    }
}

/// An operator implementation — native Rust or Rhai script.
#[derive(Clone)]
pub enum OperatorImpl {
    Native(NativeOperatorFn),
    Script(String),
}

impl std::fmt::Debug for OperatorImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperatorImpl::Native(_) => write!(f, "OperatorImpl::Native(fn)"),
            OperatorImpl::Script(s) => write!(f, "OperatorImpl::Script({:?})", s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_combo_parse_simple() {
        let k = KeyCombo::parse("a");
        assert_eq!(k.key, "a");
        assert!(!k.ctrl && !k.shift && !k.alt);
    }

    #[test]
    fn test_key_combo_parse_ctrl() {
        let k = KeyCombo::parse("ctrl-z");
        assert_eq!(k.key, "z");
        assert!(k.ctrl && !k.shift);
    }

    #[test]
    fn test_key_combo_parse_ctrl_shift() {
        let k = KeyCombo::parse("ctrl-shift-z");
        assert_eq!(k.key, "z");
        assert!(k.ctrl && k.shift);
    }

    #[test]
    fn test_key_combo_parse_shift_left() {
        let k = KeyCombo::parse("shift-left");
        assert_eq!(k.key, "left");
        assert!(!k.ctrl && k.shift);
    }

    #[test]
    fn test_key_combo_parse_alt() {
        let k = KeyCombo::parse("alt-x");
        assert_eq!(k.key, "x");
        assert!(k.alt && !k.ctrl);
    }

    #[test]
    fn test_key_combo_from_keystroke() {
        let k = KeyCombo::from_keystroke("enter", false, false, false);
        assert_eq!(k.key, "enter");
        assert!(!k.ctrl && !k.shift && !k.alt);
    }

    #[test]
    fn test_action_clone() {
        let a = Action::Command("move-left");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_action_sequence() {
        let a = Action::Sequence(vec![
            Action::Command("move-to-offset"),
            Action::Command("insert-text"),
        ]);
        match a {
            Action::Sequence(v) => assert_eq!(v.len(), 2),
            _ => panic!("Expected Sequence"),
        }
    }
}
