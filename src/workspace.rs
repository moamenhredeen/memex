use std::collections::HashMap;
use std::hash::Hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceFocus {
    Editor,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceDisplay {
    EditorOnly,
    SideBySide,
    SecondaryOnly,
}

/// Open buffers indexed both by stable ID and by resource identity.
pub struct BufferStore<K, B> {
    next_id: u64,
    buffers: HashMap<BufferId, B>,
    resources: HashMap<K, BufferId>,
    mru: Vec<BufferId>,
}

impl<K, B> Default for BufferStore<K, B> {
    fn default() -> Self {
        Self {
            next_id: 1,
            buffers: HashMap::new(),
            resources: HashMap::new(),
            mru: Vec::new(),
        }
    }
}

impl<K: Eq + Hash, B> BufferStore<K, B> {
    pub fn id_for_resource(&self, resource: &K) -> Option<BufferId> {
        self.resources.get(resource).copied()
    }

    pub fn open_with(&mut self, resource: K, create: impl FnOnce() -> B) -> BufferId {
        if let Some(&id) = self.resources.get(&resource) {
            self.touch(id);
            return id;
        }

        let id = BufferId(self.next_id);
        self.next_id += 1;
        self.resources.insert(resource, id);
        self.buffers.insert(id, create());
        self.touch(id);
        id
    }

    pub fn get(&self, id: BufferId) -> Option<&B> {
        self.buffers.get(&id)
    }

    #[allow(dead_code)]
    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut B> {
        self.touch(id);
        self.buffers.get_mut(&id)
    }

    pub fn delete(&mut self, id: BufferId) -> Option<B> {
        self.resources.retain(|_, buffer_id| *buffer_id != id);
        self.mru.retain(|buffer_id| *buffer_id != id);
        self.buffers.remove(&id)
    }

    #[allow(dead_code)]
    pub fn rekey(&mut self, id: BufferId, resource: K) -> bool {
        if !self.buffers.contains_key(&id) {
            return false;
        }
        self.resources.retain(|_, buffer_id| *buffer_id != id);
        self.resources.insert(resource, id);
        true
    }

    #[allow(dead_code)]
    pub fn mru(&self) -> &[BufferId] {
        &self.mru
    }

    fn touch(&mut self, id: BufferId) {
        self.mru.retain(|buffer_id| *buffer_id != id);
        self.mru.insert(0, id);
    }
}

pub struct Workspace<K, B> {
    pub buffers: BufferStore<K, B>,
    pub editor_buffer: BufferId,
    pub secondary_buffer: Option<BufferId>,
    pub focus: WorkspaceFocus,
    pub display: WorkspaceDisplay,
}

impl<K: Eq + Hash, B> Workspace<K, B> {
    pub fn new(resource: K, buffer: B) -> Self {
        let mut buffers = BufferStore::default();
        let buffer = buffers.open_with(resource, || buffer);
        Self {
            buffers,
            editor_buffer: buffer,
            secondary_buffer: None,
            focus: WorkspaceFocus::Editor,
            display: WorkspaceDisplay::EditorOnly,
        }
    }

    pub fn show_editor(&mut self, buffer: BufferId) -> bool {
        if self.buffers.get(buffer).is_none() {
            return false;
        }
        self.editor_buffer = buffer;
        self.focus = WorkspaceFocus::Editor;
        if self.display == WorkspaceDisplay::SecondaryOnly {
            self.display = WorkspaceDisplay::SideBySide;
        }
        self.buffers.touch(buffer);
        true
    }

    pub fn show_secondary(&mut self, buffer: Option<BufferId>) -> bool {
        if buffer.is_some_and(|id| self.buffers.get(id).is_none()) {
            return false;
        }
        self.secondary_buffer = buffer;
        self.focus = WorkspaceFocus::Secondary;
        self.display = WorkspaceDisplay::SideBySide;
        if let Some(buffer) = buffer {
            self.buffers.touch(buffer);
        }
        true
    }

    pub fn close_secondary(&mut self) {
        self.secondary_buffer = None;
        self.focus = WorkspaceFocus::Editor;
        self.display = WorkspaceDisplay::EditorOnly;
    }

    pub fn toggle_secondary_maximized(&mut self) -> bool {
        if self.secondary_buffer.is_none() && self.display == WorkspaceDisplay::EditorOnly {
            return false;
        }
        self.display = match self.display {
            WorkspaceDisplay::SecondaryOnly => WorkspaceDisplay::SideBySide,
            _ => WorkspaceDisplay::SecondaryOnly,
        };
        self.focus = WorkspaceFocus::Secondary;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_same_resource_reuses_buffer() {
        let mut store = BufferStore::default();
        let first = store.open_with("note.md", || 1);
        let second = store.open_with("note.md", || 2);

        assert_eq!(first, second);
        assert_eq!(store.get(first), Some(&1));
    }

    #[test]
    fn closing_secondary_keeps_buffer_open() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        assert!(workspace.show_secondary(Some(pdf)));

        workspace.close_secondary();
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
        assert_eq!(workspace.secondary_buffer, None);
        assert_eq!(workspace.display, WorkspaceDisplay::EditorOnly);
    }

    #[test]
    fn deleting_buffer_is_separate_from_closing_window() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        workspace.show_secondary(Some(pdf));
        workspace.close_secondary();

        assert_eq!(workspace.buffers.delete(pdf), Some("pdf"));
        assert!(workspace.buffers.get(pdf).is_none());
    }

    #[test]
    fn secondary_content_is_replaced_without_deleting_buffers() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        let graph = workspace.buffers.open_with("graph", || "graph");
        workspace.show_secondary(Some(pdf));
        workspace.show_secondary(Some(graph));

        assert_eq!(workspace.secondary_buffer, Some(graph));
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
    }

    #[test]
    fn maximizing_secondary_preserves_buffers() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        workspace.show_secondary(Some(pdf));

        assert!(workspace.toggle_secondary_maximized());
        assert_eq!(workspace.display, WorkspaceDisplay::SecondaryOnly);
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
        assert!(workspace.toggle_secondary_maximized());
        assert_eq!(workspace.display, WorkspaceDisplay::SideBySide);
    }

    #[test]
    fn opening_editor_content_restores_side_by_side_from_maximized_secondary() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        let other = workspace.buffers.open_with("other.md", || "other");
        workspace.show_secondary(Some(pdf));
        workspace.toggle_secondary_maximized();

        assert!(workspace.show_editor(other));
        assert_eq!(workspace.editor_buffer, other);
        assert_eq!(workspace.display, WorkspaceDisplay::SideBySide);
        assert_eq!(workspace.focus, WorkspaceFocus::Editor);
    }

    #[test]
    fn opening_another_resource_preserves_the_previous_buffer() {
        let mut store = BufferStore::default();
        let first = store.open_with("first.md", || "unsaved first");
        let second = store.open_with("second.md", || "second");

        assert_ne!(first, second);
        assert_eq!(store.get(first), Some(&"unsaved first"));
        assert_eq!(store.get(second), Some(&"second"));
    }
}
