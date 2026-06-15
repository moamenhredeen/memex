use std::collections::HashMap;
use std::hash::Hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: WindowId,
    pub buffer: BufferId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WindowLayout {
    Window(Window),
    Split {
        axis: SplitAxis,
        children: Vec<WindowLayout>,
        weights: Vec<f32>,
    },
}

impl WindowLayout {
    pub fn single(window: Window) -> Self {
        Self::Window(window)
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        match self {
            Self::Window(window) => (window.id == id).then_some(window),
            Self::Split { children, .. } => children.iter().find_map(|child| child.window(id)),
        }
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        match self {
            Self::Window(window) => (window.id == id).then_some(window),
            Self::Split { children, .. } => {
                children.iter_mut().find_map(|child| child.window_mut(id))
            }
        }
    }

    pub fn window_ids(&self) -> Vec<WindowId> {
        let mut ids = Vec::new();
        self.collect_window_ids(&mut ids);
        ids
    }

    fn collect_window_ids(&self, ids: &mut Vec<WindowId>) {
        match self {
            Self::Window(window) => ids.push(window.id),
            Self::Split { children, .. } => {
                for child in children {
                    child.collect_window_ids(ids);
                }
            }
        }
    }

    /// Replace a live window with a split containing the old and new windows.
    pub fn split(
        &mut self,
        target: WindowId,
        axis: SplitAxis,
        new_window: Window,
    ) -> bool {
        match self {
            Self::Window(window) if window.id == target => {
                let old_window = window.clone();
                *self = Self::Split {
                    axis,
                    children: vec![Self::Window(old_window), Self::Window(new_window)],
                    weights: vec![1.0, 1.0],
                };
                true
            }
            Self::Window(_) => false,
            Self::Split { children, .. } => children
                .iter_mut()
                .any(|child| child.split(target, axis, new_window.clone())),
        }
    }

    /// Remove a live window and collapse structural splits with one child.
    /// The final live window cannot be removed.
    pub fn close(&mut self, target: WindowId) -> bool {
        if matches!(self, Self::Window(window) if window.id == target) {
            return false;
        }
        Self::remove_from_children(self, target)
    }

    fn remove_from_children(node: &mut Self, target: WindowId) -> bool {
        let Self::Split {
            children,
            weights,
            ..
        } = node
        else {
            return false;
        };

        if let Some(index) = children
            .iter()
            .position(|child| matches!(child, Self::Window(window) if window.id == target))
        {
            children.remove(index);
            weights.remove(index);
            Self::collapse(node);
            return true;
        }

        for child in children.iter_mut() {
            if Self::remove_from_children(child, target) {
                Self::collapse(node);
                return true;
            }
        }
        false
    }

    fn collapse(node: &mut Self) {
        let replacement = match node {
            Self::Split { children, .. } if children.len() == 1 => Some(children.remove(0)),
            _ => None,
        };
        if let Some(replacement) = replacement {
            *node = replacement;
        }
    }

    /// Keep only one window without affecting any buffers.
    pub fn only(&mut self, target: WindowId) -> bool {
        let Some(window) = self.window(target).cloned() else {
            return false;
        };
        *self = Self::Window(window);
        true
    }
}

/// Open buffers indexed both by stable ID and by resource identity.
pub struct BufferStore<K, B> {
    next_id: u64,
    buffers: HashMap<BufferId, B>,
    resources: HashMap<K, BufferId>,
    mru: Vec<BufferId>,
}

/// Presentation state owned by live windows, separate from retained buffers.
pub struct WindowStore<W> {
    windows: HashMap<WindowId, W>,
}

impl<W> Default for WindowStore<W> {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }
}

impl<W> WindowStore<W> {
    pub fn insert(&mut self, id: WindowId, state: W) -> Option<W> {
        self.windows.insert(id, state)
    }

    pub fn get(&self, id: WindowId) -> Option<&W> {
        self.windows.get(&id)
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.windows.contains_key(&id)
    }

    pub fn remove(&mut self, id: WindowId) -> Option<W> {
        self.windows.remove(&id)
    }

    pub fn values(&self) -> impl Iterator<Item = &W> {
        self.windows.values()
    }

    pub fn retain_only(&mut self, id: WindowId) {
        self.windows.retain(|window_id, _| *window_id == id);
    }
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

    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut B> {
        self.touch(id);
        self.buffers.get_mut(&id)
    }

    pub fn delete(&mut self, id: BufferId) -> Option<B> {
        self.resources.retain(|_, buffer_id| *buffer_id != id);
        self.mru.retain(|buffer_id| *buffer_id != id);
        self.buffers.remove(&id)
    }

    pub fn rekey(&mut self, id: BufferId, resource: K) -> bool {
        if !self.buffers.contains_key(&id) {
            return false;
        }
        self.resources.retain(|_, buffer_id| *buffer_id != id);
        self.resources.insert(resource, id);
        true
    }

    pub fn mru(&self) -> &[BufferId] {
        &self.mru
    }

    fn touch(&mut self, id: BufferId) {
        self.mru.retain(|buffer_id| *buffer_id != id);
        self.mru.insert(0, id);
    }
}

pub struct Workspace<K, B> {
    next_window_id: u64,
    pub buffers: BufferStore<K, B>,
    pub layout: WindowLayout,
    pub focused_window: WindowId,
}

impl<K: Eq + Hash, B> Workspace<K, B> {
    pub fn new(resource: K, buffer: B) -> Self {
        let mut buffers = BufferStore::default();
        let buffer = buffers.open_with(resource, || buffer);
        let window = Window {
            id: WindowId(1),
            buffer,
        };
        Self {
            next_window_id: 2,
            buffers,
            layout: WindowLayout::single(window),
            focused_window: WindowId(1),
        }
    }

    pub fn split_focused(&mut self, axis: SplitAxis, buffer: BufferId) -> Option<WindowId> {
        self.buffers.get(buffer)?;
        let id = WindowId(self.next_window_id);
        let window = Window { id, buffer };
        if !self.layout.split(self.focused_window, axis, window) {
            return None;
        }
        self.next_window_id += 1;
        self.focused_window = id;
        Some(id)
    }

    pub fn buffer_for_window(&self, window: WindowId) -> Option<BufferId> {
        self.layout.window(window).map(|window| window.buffer)
    }

    pub fn focused_buffer(&self) -> BufferId {
        self.buffer_for_window(self.focused_window)
            .expect("focused window must be live")
    }

    pub fn switch_window_buffer(&mut self, window: WindowId, buffer: BufferId) -> bool {
        if self.buffers.get(buffer).is_none() {
            return false;
        }
        let Some(window) = self.layout.window_mut(window) else {
            return false;
        };
        window.buffer = buffer;
        self.buffers.touch(buffer);
        true
    }

    pub fn focus(&mut self, id: WindowId) -> bool {
        if self.layout.window(id).is_none() {
            return false;
        }
        self.focused_window = id;
        let buffer = self.layout.window(id).expect("window checked").buffer;
        self.buffers.touch(buffer);
        true
    }

    pub fn close_focused(&mut self) -> bool {
        let ids = self.layout.window_ids();
        let Some(index) = ids.iter().position(|id| *id == self.focused_window) else {
            return false;
        };
        if !self.layout.close(self.focused_window) {
            return false;
        }
        let remaining = self.layout.window_ids();
        self.focused_window = remaining[index.min(remaining.len() - 1)];
        true
    }

    pub fn only_focused(&mut self) -> bool {
        self.layout.only(self.focused_window)
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
    fn closing_window_keeps_buffer_open() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        workspace
            .split_focused(SplitAxis::Horizontal, pdf)
            .unwrap();

        assert!(workspace.close_focused());
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
        assert_eq!(workspace.layout.window_ids().len(), 1);
    }

    #[test]
    fn deleting_buffer_is_separate_from_closing_window() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        workspace
            .split_focused(SplitAxis::Horizontal, pdf)
            .unwrap();
        workspace.close_focused();

        assert_eq!(workspace.buffers.delete(pdf), Some("pdf"));
        assert!(workspace.buffers.get(pdf).is_none());
    }

    #[test]
    fn nested_split_collapses_after_close() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        let graph = workspace.buffers.open_with("graph", || "graph");
        workspace
            .split_focused(SplitAxis::Horizontal, pdf)
            .unwrap();
        workspace
            .split_focused(SplitAxis::Vertical, graph)
            .unwrap();

        assert_eq!(workspace.layout.window_ids().len(), 3);
        assert!(workspace.close_focused());
        assert_eq!(workspace.layout.window_ids().len(), 2);
        assert!(matches!(workspace.layout, WindowLayout::Split { .. }));
    }

    #[test]
    fn only_window_preserves_hidden_buffers() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        workspace
            .split_focused(SplitAxis::Horizontal, pdf)
            .unwrap();

        assert!(workspace.only_focused());
        assert_eq!(workspace.layout.window_ids().len(), 1);
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
        assert_eq!(workspace.buffers.mru()[0], pdf);
    }

    #[test]
    fn switching_window_buffer_keeps_previous_buffer_open() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        let graph = workspace.buffers.open_with("graph", || "graph");
        let tool_window = workspace
            .split_focused(SplitAxis::Horizontal, pdf)
            .unwrap();

        assert!(workspace.switch_window_buffer(tool_window, graph));
        assert_eq!(workspace.buffer_for_window(tool_window), Some(graph));
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
        assert_eq!(workspace.buffers.get(graph), Some(&"graph"));
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

    #[test]
    fn window_state_is_replaced_without_deleting_buffers() {
        let mut workspace = Workspace::new("note.md", "note");
        let pdf = workspace.buffers.open_with("manual.pdf", || "pdf");
        let tool_window = workspace
            .split_focused(SplitAxis::Horizontal, pdf)
            .unwrap();
        let mut windows = WindowStore::default();
        windows.insert(WindowId(1), "editor viewport");
        windows.insert(tool_window, "pdf viewport");

        assert_eq!(windows.insert(tool_window, "graph viewport"), Some("pdf viewport"));
        assert_eq!(windows.get(tool_window), Some(&"graph viewport"));
        assert_eq!(workspace.buffers.get(pdf), Some(&"pdf"));
    }

    #[test]
    fn retaining_one_window_drops_only_other_presentations() {
        let mut windows = WindowStore::default();
        windows.insert(WindowId(1), "editor viewport");
        windows.insert(WindowId(2), "pdf viewport");

        windows.retain_only(WindowId(2));

        assert!(windows.get(WindowId(1)).is_none());
        assert_eq!(windows.get(WindowId(2)), Some(&"pdf viewport"));
    }
}
