use std::collections::HashMap;

use super::action::{Action, KeyCombo, MotionId, MotionImpl, OperatorId, OperatorImpl};
use super::context::{KeyContext, KeyPredicate};

#[derive(Clone, Debug)]
pub enum KeyTrie {
    Leaf(Action, usize, usize),
    Node(HashMap<KeyCombo, KeyTrie>),
}

#[derive(Clone, Debug)]
pub struct KeyBinding {
    pub sequence: Vec<KeyCombo>,
    pub predicate: KeyPredicate,
    pub action: Action,
    pub order: usize,
}

#[derive(Default)]
pub struct BindingRegistry {
    bindings: Vec<KeyBinding>,
    motions: HashMap<MotionId, MotionImpl>,
    operators: HashMap<OperatorId, OperatorImpl>,
    next_order: usize,
}

impl BindingRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, key: &str, predicate: KeyPredicate, action: Action) {
        let sequence = key.split_whitespace().map(KeyCombo::parse).collect();
        self.bind_sequence(sequence, predicate, action);
    }

    pub fn bind_sequence(
        &mut self,
        sequence: Vec<KeyCombo>,
        predicate: KeyPredicate,
        action: Action,
    ) {
        assert!(
            !sequence.is_empty(),
            "key binding sequence must not be empty"
        );
        let order = self.next_order;
        self.next_order += 1;
        self.bindings.push(KeyBinding {
            sequence,
            predicate,
            action,
            order,
        });
    }

    pub fn resolve(&self, combo: &KeyCombo, context: &KeyContext) -> Option<KeyTrie> {
        let mut root = KeyTrie::Node(HashMap::new());
        for binding in self
            .bindings
            .iter()
            .filter(|binding| binding.predicate.matches(context))
        {
            insert_binding(&mut root, binding, 0);
        }
        match root {
            KeyTrie::Node(map) => map.get(combo).cloned(),
            KeyTrie::Leaf(_, _, _) => None,
        }
    }

    pub fn register_motion(&mut self, id: MotionId, imp: MotionImpl) {
        self.motions.insert(id, imp);
    }

    pub fn register_operator(&mut self, id: OperatorId, imp: OperatorImpl) {
        self.operators.insert(id, imp);
    }

    pub fn get_motion(&self, id: MotionId) -> Option<&MotionImpl> {
        self.motions.get(id)
    }

    pub fn get_operator(&self, id: OperatorId) -> Option<&OperatorImpl> {
        self.operators.get(id)
    }
}

fn insert_binding(node: &mut KeyTrie, binding: &KeyBinding, depth: usize) {
    let KeyTrie::Node(map) = node else {
        return;
    };
    let combo = binding.sequence[depth].clone();
    if depth == binding.sequence.len() - 1 {
        let specificity = binding.predicate.specificity();
        let new_leaf = KeyTrie::Leaf(binding.action.clone(), specificity, binding.order);
        match map.get(&combo) {
            Some(KeyTrie::Leaf(_, old_specificity, old_order))
                if (*old_specificity, *old_order) > (specificity, binding.order) => {}
            Some(KeyTrie::Node(_)) => {}
            _ => {
                map.insert(combo, new_leaf);
            }
        }
        return;
    }

    let child = map
        .entry(combo)
        .or_insert_with(|| KeyTrie::Node(HashMap::new()));
    if matches!(child, KeyTrie::Leaf(_, _, _)) {
        *child = KeyTrie::Node(HashMap::new());
    }
    insert_binding(child, binding, depth + 1);
}
