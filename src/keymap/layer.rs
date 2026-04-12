use std::collections::HashMap;

use super::action::{Action, KeyCombo, LayerGroupId, LayerId, MotionId, MotionImpl, OperatorId, OperatorImpl};

/// A trie node: either a leaf (action) or a branch (subtree for multi-key sequences).
#[derive(Clone, Debug)]
pub enum KeyTrie {
    /// Terminal: this key sequence maps to an action.
    Leaf(Action),
    /// Non-terminal: more keys are expected. Maps next key → child node.
    Node(HashMap<KeyCombo, KeyTrie>),
}

impl KeyTrie {
    /// Get the list of valid next keys (for which-key popup).
    pub fn children(&self) -> Vec<&KeyCombo> {
        match self {
            KeyTrie::Leaf(_) => vec![],
            KeyTrie::Node(map) => map.keys().collect(),
        }
    }
}

/// Result of resolving a key through the layer stack.
#[derive(Clone, Debug)]
pub enum ResolveResult {
    /// Resolved to an action — ready for grammar processing.
    Action(Action),
    /// Partial match — waiting for more keys. Contains valid next keys for display.
    Pending(Vec<KeyCombo>),
    /// No match found in any layer.
    NoMatch,
}

/// A named keymap layer — maps key combos to trie nodes (actions or subtrees).
pub struct Layer {
    pub id: LayerId,
    pub group: Option<LayerGroupId>,
    pub bindings: HashMap<KeyCombo, KeyTrie>,
    /// Transient layers auto-pop after one key resolution.
    pub transient: bool,
}

impl Layer {
    pub fn new(id: LayerId) -> Self {
        Self {
            id,
            group: None,
            bindings: HashMap::new(),
            transient: false,
        }
    }

    pub fn with_group(mut self, group: LayerGroupId) -> Self {
        self.group = Some(group);
        self
    }

    pub fn transient(mut self) -> Self {
        self.transient = true;
        self
    }

    /// Bind a single key to an action (leaf node).
    pub fn bind(mut self, key: &str, action: Action) -> Self {
        self.bindings.insert(KeyCombo::parse(key), KeyTrie::Leaf(action));
        self
    }

    /// Bind a single key combo to an action (leaf node).
    pub fn bind_combo(&mut self, combo: KeyCombo, action: Action) {
        self.bindings.insert(combo, KeyTrie::Leaf(action));
    }

    /// Bind a multi-key sequence to an action. The sequence is given as
    /// space-separated keys, e.g. `"space f"` or `"g g"`.
    pub fn bind_seq(mut self, seq: &str, action: Action) -> Self {
        let keys: Vec<KeyCombo> = seq.split_whitespace().map(KeyCombo::parse).collect();
        assert!(!keys.is_empty(), "bind_seq: empty key sequence");

        if keys.len() == 1 {
            self.bindings.insert(keys.into_iter().next().unwrap(), KeyTrie::Leaf(action));
            return self;
        }

        // Navigate/create trie nodes for the prefix, insert leaf at the end.
        let first = keys[0].clone();
        let rest = &keys[1..];

        // Ensure root entry is a Node
        let root_entry = self.bindings.entry(first)
            .or_insert_with(|| KeyTrie::Node(HashMap::new()));

        Self::insert_into_trie(root_entry, rest, action);

        self
    }

    /// Recursively insert an action at a trie path.
    fn insert_into_trie(node: &mut KeyTrie, keys: &[KeyCombo], action: Action) {
        if keys.is_empty() {
            return;
        }

        // Ensure current node is a Node (promote Leaf if necessary)
        if matches!(node, KeyTrie::Leaf(_)) {
            *node = KeyTrie::Node(HashMap::new());
        }

        let map = match node {
            KeyTrie::Node(map) => map,
            _ => unreachable!(),
        };

        if keys.len() == 1 {
            // Last key — insert leaf
            map.insert(keys[0].clone(), KeyTrie::Leaf(action));
        } else {
            // Intermediate — recurse
            let child = map.entry(keys[0].clone())
                .or_insert_with(|| KeyTrie::Node(HashMap::new()));
            Self::insert_into_trie(child, &keys[1..], action);
        }
    }

    /// Look up a single key in this layer's bindings.
    pub fn lookup(&self, combo: &KeyCombo) -> Option<&KeyTrie> {
        self.bindings.get(combo)
    }
}

/// The layer stack: manages active layers and resolves key lookups.
pub struct LayerStack {
    /// All registered layers.
    registry: HashMap<LayerId, Layer>,
    /// Currently active layer IDs, in priority order (first = highest).
    active: Vec<LayerId>,
    /// Layer groups: maps group name → member layer IDs.
    groups: HashMap<LayerGroupId, Vec<LayerId>>,
    /// Motion registry: name → implementation.
    motions: HashMap<MotionId, MotionImpl>,
    /// Operator registry: name → implementation.
    operators: HashMap<OperatorId, OperatorImpl>,
}

impl LayerStack {
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
            active: Vec::new(),
            groups: HashMap::new(),
            motions: HashMap::new(),
            operators: HashMap::new(),
        }
    }

    /// Register a layer. Does not activate it.
    pub fn register_layer(&mut self, layer: Layer) {
        if let Some(group) = layer.group {
            self.groups.entry(group).or_default().push(layer.id);
        }
        self.registry.insert(layer.id, layer);
    }

    /// Activate a layer (push to front of active stack).
    /// If the layer belongs to a group, deactivate other layers in that group first.
    pub fn activate_layer(&mut self, id: LayerId) {
        if let Some(layer) = self.registry.get(id) {
            if let Some(group) = layer.group {
                // Deactivate other layers in the same group
                if let Some(members) = self.groups.get(group) {
                    let to_remove: Vec<LayerId> = members.iter()
                        .filter(|m| **m != id)
                        .copied()
                        .collect();
                    self.active.retain(|a| !to_remove.contains(a));
                }
            }
        }

        // Remove if already active (to re-push at front)
        self.active.retain(|a| *a != id);
        self.active.insert(0, id);
    }

    /// Deactivate a specific layer.
    pub fn deactivate_layer(&mut self, id: LayerId) {
        self.active.retain(|a| *a != id);
    }

    /// Deactivate all layers in a group.
    pub fn deactivate_group(&mut self, group: LayerGroupId) {
        if let Some(members) = self.groups.get(group) {
            let members: Vec<LayerId> = members.clone();
            self.active.retain(|a| !members.contains(a));
        }
    }

    /// Push a transient layer to the front. It will be auto-popped after resolution.
    pub fn push_transient(&mut self, id: LayerId) {
        // Remove if already active
        self.active.retain(|a| *a != id);
        self.active.insert(0, id);
    }

    /// Pop the frontmost transient layer (if any).
    pub fn pop_transient(&mut self) {
        if let Some(first) = self.active.first().copied() {
            if self.registry.get(first).map_or(false, |l| l.transient) {
                self.active.remove(0);
            }
        }
    }

    /// Peek at the frontmost layer; return its ID if it is transient.
    pub fn peek_transient(&self) -> Option<LayerId> {
        self.active.first().and_then(|id| {
            if self.registry.get(id).map_or(false, |l| l.transient) {
                Some(*id)
            } else {
                None
            }
        })
    }

    /// Resolve a key combo through the active layer stack.
    /// Returns the first matching trie node (highest priority layer wins).
    pub fn resolve(&self, combo: &KeyCombo) -> Option<&KeyTrie> {
        for layer_id in &self.active {
            if let Some(layer) = self.registry.get(layer_id) {
                if let Some(trie) = layer.lookup(combo) {
                    return Some(trie);
                }
            }
        }
        None
    }

    /// Resolve a key combo within an existing trie node (for multi-key sequence walking).
    pub fn resolve_in_trie<'a>(&'a self, trie: &'a KeyTrie, combo: &KeyCombo) -> Option<&'a KeyTrie> {
        match trie {
            KeyTrie::Node(map) => map.get(combo),
            KeyTrie::Leaf(_) => None,
        }
    }

    /// Get the ordered list of active layer IDs.
    pub fn active_layers(&self) -> &[LayerId] {
        &self.active
    }

    /// Check if a specific layer is active.
    pub fn is_active(&self, id: LayerId) -> bool {
        self.active.contains(&id)
    }

    /// Register a motion implementation.
    pub fn register_motion(&mut self, id: MotionId, imp: MotionImpl) {
        self.motions.insert(id, imp);
    }

    /// Register an operator implementation.
    pub fn register_operator(&mut self, id: OperatorId, imp: OperatorImpl) {
        self.operators.insert(id, imp);
    }

    /// Look up a motion implementation.
    pub fn get_motion(&self, id: MotionId) -> Option<&MotionImpl> {
        self.motions.get(id)
    }

    /// Look up an operator implementation.
    pub fn get_operator(&self, id: OperatorId) -> Option<&OperatorImpl> {
        self.operators.get(id)
    }

    /// Runtime bind: add/override a single-key binding in an existing layer.
    pub fn bind(&mut self, layer_id: LayerId, key: &str, action: Action) {
        if let Some(layer) = self.registry.get_mut(layer_id) {
            layer.bind_combo(KeyCombo::parse(key), action);
        }
    }

    /// Runtime bind: add/override a multi-key sequence in an existing layer.
    pub fn bind_seq(&mut self, layer_id: LayerId, seq: &str, action: Action) {
        if let Some(layer) = self.registry.get_mut(layer_id) {
            let keys: Vec<KeyCombo> = seq.split_whitespace().map(KeyCombo::parse).collect();
            if keys.is_empty() {
                return;
            }
            if keys.len() == 1 {
                layer.bindings.insert(keys.into_iter().next().unwrap(), KeyTrie::Leaf(action));
                return;
            }

            let first = keys[0].clone();
            let rest = &keys[1..];
            let root = layer.bindings.entry(first)
                .or_insert_with(|| KeyTrie::Node(HashMap::new()));
            Layer::insert_into_trie(root, rest, action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_stack() -> LayerStack {
        let mut stack = LayerStack::new();

        let global = Layer::new("global")
            .bind("a", Action::SelfInsert)
            .bind("ctrl-z", Action::Command("undo"));

        let vim_normal = Layer::new("vim:normal")
            .with_group("vim-state")
            .bind("h", Action::Motion("left"))
            .bind("l", Action::Motion("right"))
            .bind("d", Action::Operator("delete"))
            .bind("a", Action::Command("append-after")); // overrides global "a"

        let vim_insert = Layer::new("vim:insert")
            .with_group("vim-state")
            .bind("escape", Action::ActivateLayer("vim:normal"));

        stack.register_layer(global);
        stack.register_layer(vim_normal);
        stack.register_layer(vim_insert);
        stack
    }

    #[test]
    fn test_resolve_single_layer() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");

        let combo = KeyCombo::parse("ctrl-z");
        let result = stack.resolve(&combo);
        assert!(matches!(result, Some(KeyTrie::Leaf(Action::Command("undo")))));
    }

    #[test]
    fn test_resolve_no_match() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");

        let combo = KeyCombo::parse("f12");
        assert!(stack.resolve(&combo).is_none());
    }

    #[test]
    fn test_layer_priority_top_wins() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");

        // "a" is bound in both global (SelfInsert) and vim:normal (Command)
        // vim:normal is higher priority → its binding wins
        let combo = KeyCombo::parse("a");
        assert!(matches!(stack.resolve(&combo), Some(KeyTrie::Leaf(Action::Command("append-after")))));
    }

    #[test]
    fn test_fallthrough_to_lower_layer() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");

        // ctrl-z is only in global — falls through from vim:normal
        let combo = KeyCombo::parse("ctrl-z");
        assert!(matches!(stack.resolve(&combo), Some(KeyTrie::Leaf(Action::Command("undo")))));
    }

    #[test]
    fn test_group_mutual_exclusion() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");

        assert!(stack.is_active("vim:normal"));
        assert!(!stack.is_active("vim:insert"));

        // Activate vim:insert — should deactivate vim:normal (same group)
        stack.activate_layer("vim:insert");

        assert!(stack.is_active("vim:insert"));
        assert!(!stack.is_active("vim:normal"));

        // "escape" should resolve from vim:insert
        let combo = KeyCombo::parse("escape");
        assert!(matches!(stack.resolve(&combo), Some(KeyTrie::Leaf(Action::ActivateLayer("vim:normal")))));

        // "h" should NOT resolve (vim:normal is deactivated)
        let combo = KeyCombo::parse("h");
        assert!(stack.resolve(&combo).is_none());
    }

    #[test]
    fn test_deactivate_group() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");

        stack.deactivate_group("vim-state");
        assert!(!stack.is_active("vim:normal"));
        assert!(!stack.is_active("vim:insert"));
    }

    #[test]
    fn test_transient_layer() {
        let mut stack = LayerStack::new();

        let global = Layer::new("global")
            .bind("f", Action::PushTransient("transient:find-char"));

        let transient = Layer::new("transient:find-char")
            .transient()
            .bind("a", Action::Motion("find-char-a"));

        stack.register_layer(global);
        stack.register_layer(transient);
        stack.activate_layer("global");

        // Push transient
        stack.push_transient("transient:find-char");
        assert!(stack.is_active("transient:find-char"));

        // "a" resolves from transient
        let combo = KeyCombo::parse("a");
        assert!(matches!(stack.resolve(&combo), Some(KeyTrie::Leaf(Action::Motion("find-char-a")))));

        // Pop transient
        stack.pop_transient();
        assert!(!stack.is_active("transient:find-char"));
    }

    #[test]
    fn test_runtime_bind() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");

        // Override ctrl-z in global
        stack.bind("global", "ctrl-z", Action::Command("custom-undo"));

        let combo = KeyCombo::parse("ctrl-z");
        assert!(matches!(stack.resolve(&combo), Some(KeyTrie::Leaf(Action::Command("custom-undo")))));
    }

    #[test]
    fn test_active_layers_order() {
        let mut stack = make_test_stack();
        stack.activate_layer("global");
        stack.activate_layer("vim:normal");

        let active = stack.active_layers();
        assert_eq!(active[0], "vim:normal");
        assert_eq!(active[1], "global");
    }

    #[test]
    fn test_motion_registry() {
        let mut stack = LayerStack::new();
        stack.register_motion("left", MotionImpl::Native(|_content, cursor, count| {
            cursor.saturating_sub(count)
        }));

        let m = stack.get_motion("left").unwrap();
        match m {
            MotionImpl::Native(f) => {
                assert_eq!(f("hello", 3, 1), 2);
                assert_eq!(f("hello", 0, 1), 0);
            }
            _ => panic!("Expected native motion"),
        }
    }

    #[test]
    fn test_bind_seq_two_keys() {
        let mut stack = LayerStack::new();
        let layer = Layer::new("test")
            .bind_seq("g g", Action::Command("doc-start"))
            .bind_seq("g _", Action::Command("line-end"));
        stack.register_layer(layer);
        stack.activate_layer("test");

        // "g" should resolve to a Node (pending)
        let g = KeyCombo::parse("g");
        let trie = stack.resolve(&g).unwrap();
        assert!(matches!(trie, KeyTrie::Node(_)));

        // "g g" should resolve to doc-start
        let g2 = KeyCombo::parse("g");
        let leaf = stack.resolve_in_trie(trie, &g2).unwrap();
        assert!(matches!(leaf, KeyTrie::Leaf(Action::Command("doc-start"))));

        // "g _" should resolve to line-end
        let underscore = KeyCombo::parse("_");
        let leaf2 = stack.resolve_in_trie(trie, &underscore).unwrap();
        assert!(matches!(leaf2, KeyTrie::Leaf(Action::Command("line-end"))));
    }

    #[test]
    fn test_bind_seq_three_keys() {
        let mut stack = LayerStack::new();
        let layer = Layer::new("test")
            .bind_seq("space g s", Action::Command("git-status"))
            .bind_seq("space f", Action::Command("find-file"));
        stack.register_layer(layer);
        stack.activate_layer("test");

        // "space" → Node
        let space = KeyCombo::parse("space");
        let trie = stack.resolve(&space).unwrap();
        assert!(matches!(trie, KeyTrie::Node(_)));

        // "space f" → find-file
        let f = KeyCombo::parse("f");
        let leaf = stack.resolve_in_trie(trie, &f).unwrap();
        assert!(matches!(leaf, KeyTrie::Leaf(Action::Command("find-file"))));

        // "space g" → Node
        let g = KeyCombo::parse("g");
        let node = stack.resolve_in_trie(trie, &g).unwrap();
        assert!(matches!(node, KeyTrie::Node(_)));

        // "space g s" → git-status
        let s = KeyCombo::parse("s");
        let leaf2 = stack.resolve_in_trie(node, &s).unwrap();
        assert!(matches!(leaf2, KeyTrie::Leaf(Action::Command("git-status"))));
    }

    #[test]
    fn test_runtime_bind_seq() {
        let mut stack = LayerStack::new();
        let layer = Layer::new("test");
        stack.register_layer(layer);
        stack.activate_layer("test");

        stack.bind_seq("test", "space b", Action::Command("switch-buffer"));

        let space = KeyCombo::parse("space");
        let trie = stack.resolve(&space).unwrap();
        let b = KeyCombo::parse("b");
        let leaf = stack.resolve_in_trie(trie, &b).unwrap();
        assert!(matches!(leaf, KeyTrie::Leaf(Action::Command("switch-buffer"))));
    }
}
