//! Diagram view -- a native excalidraw-class diagram editor.
//!
//! Diagrams are stored as `.excalidraw` JSON files under the vault's
//! `diagrams/` folder and linked from notes via `[[name.excalidraw]]`.
//! Clicking such a link opens the diagram in the secondary pane (same slot the
//! PDF viewer uses).
//!
//! Phase 0: load + render skeleton + pan/zoom scaffolding. Drawing tools,
//! selection, editing, and import live in later phases (see PLAN.md).

mod import;
mod model;
mod view;

use std::path::{Path, PathBuf};

use gpui::*;

use crate::command::Command;
use crate::minibuffer::Candidate;
use crate::pane::{CommandOutcome, ItemAction};

pub use import::import_file;
pub use model::{Binding, Element, ExcalidrawFile};
pub use view::{DiagramView, DiagramViewEvent};

/// The active editing tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Diamond,
    Arrow,
    Line,
    Draw,
    Text,
}

impl Tool {
    /// excalidraw element type string this tool creates (None for Select).
    #[allow(dead_code)] // used by creation in Phase 3b
    pub fn element_type(self) -> Option<&'static str> {
        match self {
            Tool::Select => None,
            Tool::Rectangle => Some("rectangle"),
            Tool::Ellipse => Some("ellipse"),
            Tool::Diamond => Some("diamond"),
            Tool::Arrow => Some("arrow"),
            Tool::Line => Some("line"),
            Tool::Draw => Some("freedraw"),
            Tool::Text => Some("text"),
        }
    }
}

/// Live state for an open diagram.
pub struct DiagramState {
    /// On-disk path of the `.excalidraw` file.
    pub path: PathBuf,
    /// Parsed document (the in-memory model).
    pub file: ExcalidrawFile,
    /// Camera zoom factor (1.0 == 100%).
    pub zoom: f32,
    /// Camera pan offset in screen pixels.
    pub pan_x: f32,
    pub pan_y: f32,
    /// Active editing tool.
    pub tool: Tool,
    /// Indices into `file.elements` of the current selection.
    pub selected: Vec<usize>,
    /// Whether to draw the background grid.
    pub show_grid: bool,
    /// Whether move/create/resize snap to the grid (Alt at drag time inverts).
    pub snap_enabled: bool,
    /// Unsaved changes pending.
    pub dirty: bool,
    /// Index of the text element currently being typed into (inline editing).
    editing_text: Option<usize>,
    /// Undo/redo history of whole-document snapshots.
    undo_stack: Vec<ExcalidrawFile>,
    redo_stack: Vec<ExcalidrawFile>,
    /// Screen origin of the canvas, stashed each paint for hit-testing.
    origin_x: f32,
    origin_y: f32,
    pub focus_handle: FocusHandle,
}

impl DiagramState {
    pub fn new(path: &Path, file: ExcalidrawFile, cx: &mut App) -> Self {
        let mut state = Self {
            path: path.to_path_buf(),
            file,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            tool: Tool::Select,
            selected: Vec::new(),
            show_grid: true,
            snap_enabled: true,
            dirty: false,
            editing_text: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            origin_x: 0.0,
            origin_y: 0.0,
            focus_handle: cx.focus_handle(),
        };
        // Pin bound connectors to their shapes (no-op for files without
        // memex bindings; corrects geometry if shapes were moved elsewhere).
        state.reroute_bindings();
        state
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    /// Number of non-deleted elements.
    pub fn element_count(&self) -> usize {
        self.file.elements.iter().filter(|e| !e.is_deleted).count()
    }

    /// Bounding box of all non-deleted elements in world coordinates as
    /// `(min_x, min_y, max_x, max_y)`. `None` when the diagram is empty.
    pub fn content_bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        let mut any = false;
        for e in &self.file.elements {
            if e.is_deleted {
                continue;
            }
            any = true;
            min_x = min_x.min(e.x);
            min_y = min_y.min(e.y);
            max_x = max_x.max(e.x + e.width);
            max_y = max_y.max(e.y + e.height);
        }
        any.then_some((min_x, min_y, max_x, max_y))
    }

    /// Set the camera so the content bounding box is centered and fits within
    /// the given viewport (with padding). No-op when the diagram is empty.
    pub fn fit_to_content(&mut self, viewport_w: f32, viewport_h: f32) {
        let Some((min_x, min_y, max_x, max_y)) = self.content_bounds() else {
            return;
        };
        const PAD: f32 = 48.0;
        let bw = (max_x - min_x) as f32;
        let bh = (max_y - min_y) as f32;
        let zx = if bw > 0.0 {
            (viewport_w - PAD) / bw
        } else {
            1.0
        };
        let zy = if bh > 0.0 {
            (viewport_h - PAD) / bh
        } else {
            1.0
        };
        self.zoom = zx.min(zy).clamp(0.05, 1.0);
        let center_x = ((min_x + max_x) / 2.0) as f32;
        let center_y = ((min_y + max_y) / 2.0) as f32;
        self.pan_x = viewport_w / 2.0 - center_x * self.zoom;
        self.pan_y = viewport_h / 2.0 - center_y * self.zoom;
    }

    // ─── Camera transforms ──────────────────────────────────────────────

    /// Record the canvas screen origin (called from the paint pass) so mouse
    /// handlers can map window coordinates into world space.
    pub fn set_viewport_origin(&mut self, x: f32, y: f32) {
        self.origin_x = x;
        self.origin_y = y;
    }

    /// Map a screen (window) point to world coordinates.
    pub fn screen_to_world(&self, sx: f32, sy: f32) -> (f64, f64) {
        let wx = (sx - self.origin_x - self.pan_x) / self.zoom;
        let wy = (sy - self.origin_y - self.pan_y) / self.zoom;
        (wx as f64, wy as f64)
    }

    /// Map a world point to screen (window) coordinates.
    pub fn world_to_screen(&self, wx: f64, wy: f64) -> (f32, f32) {
        (
            wx as f32 * self.zoom + self.pan_x + self.origin_x,
            wy as f32 * self.zoom + self.pan_y + self.origin_y,
        )
    }

    /// Grid spacing in world units. Reads `appState.gridSize` when set to a
    /// number, else the excalidraw default of 20.
    pub fn grid_size(&self) -> f64 {
        self.file
            .app_state
            .get("gridSize")
            .and_then(serde_json::Value::as_f64)
            .filter(|g| *g > 0.0)
            .unwrap_or(20.0)
    }

    /// Round a world coordinate to the nearest grid line.
    pub fn snap_coord(&self, v: f64) -> f64 {
        let g = self.grid_size();
        (v / g).round() * g
    }

    /// If exactly one box-like element (rectangle/ellipse/diamond/text/image)
    /// is selected, return its index. Resize handles apply only to these.
    pub fn selected_single_box(&self) -> Option<usize> {
        if self.selected.len() != 1 {
            return None;
        }
        let i = self.selected[0];
        let el = self.file.elements.get(i)?;
        if el.is_deleted {
            return None;
        }
        matches!(
            el.element_type.as_str(),
            "rectangle" | "ellipse" | "diamond" | "text" | "image" | "frame"
        )
        .then_some(i)
    }

    /// Whether the element at `index` is an editable text element.
    pub fn is_text_element(&self, index: usize) -> bool {
        self.file
            .elements
            .get(index)
            .is_some_and(|el| el.element_type == "text" && !el.is_deleted)
    }

    /// World-space `(x, y, width, height)` of an element.
    pub fn element_bounds(&self, index: usize) -> Option<(f64, f64, f64, f64)> {
        self.file
            .elements
            .get(index)
            .map(|el| (el.x, el.y, el.width, el.height))
    }

    /// Set an element's full box (used by resize).
    pub fn set_element_box(&mut self, index: usize, x: f64, y: f64, w: f64, h: f64) {
        if let Some(el) = self.file.elements.get_mut(index) {
            el.x = x;
            el.y = y;
            el.width = w;
            el.height = h;
        }
    }

    // ─── Undo / redo ────────────────────────────────────────────────────

    /// Snapshot the document before a mutating operation. Clears the redo
    /// stack (a new edit forks history).
    pub fn push_undo(&mut self) {
        const CAP: usize = 100;
        self.undo_stack.push(self.file.clone());
        if self.undo_stack.len() > CAP {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Drop the most recent undo snapshot (used to cancel a no-op operation,
    /// e.g. a click that created nothing or a drag that did not move).
    pub fn discard_last_undo(&mut self) {
        self.undo_stack.pop();
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.file, prev);
        self.redo_stack.push(current);
        self.selected.clear();
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.file, next);
        self.undo_stack.push(current);
        self.selected.clear();
        self.dirty = true;
        true
    }

    // ─── Selection & editing ────────────────────────────────────────────

    /// Topmost non-deleted element at the given world point, if any.
    pub fn hit_test(&self, wx: f64, wy: f64) -> Option<usize> {
        // World-space click tolerance (constant in screen pixels).
        let pad = 6.0 / self.zoom.max(0.01) as f64;
        for (i, el) in self.file.elements.iter().enumerate().rev() {
            if el.is_deleted {
                continue;
            }
            let (x0, y0, x1, y1) = (
                el.x.min(el.x + el.width),
                el.y.min(el.y + el.height),
                el.x.max(el.x + el.width),
                el.y.max(el.y + el.height),
            );
            if wx >= x0 - pad && wx <= x1 + pad && wy >= y0 - pad && wy <= y1 + pad {
                return Some(i);
            }
        }
        None
    }

    pub fn select_only(&mut self, index: usize) {
        self.selected = vec![index];
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Toggle an element's membership in the selection (Ctrl+click).
    pub fn toggle_select(&mut self, index: usize) {
        if let Some(pos) = self.selected.iter().position(|&i| i == index) {
            self.selected.remove(pos);
        } else {
            self.selected.push(index);
        }
    }

    /// Whether an element index is currently selected.
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    /// Select every non-deleted element whose bounding box overlaps the world
    /// rectangle `(x0, y0)`..`(x1, y1)` (rubber-band). When `additive`, union
    /// the hits onto `base` (the selection at drag start); otherwise replace.
    pub fn select_in_rect(
        &mut self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        additive: bool,
        base: &[usize],
    ) {
        let mut hits = Vec::new();
        for (i, el) in self.file.elements.iter().enumerate() {
            if el.is_deleted {
                continue;
            }
            let ex0 = el.x.min(el.x + el.width);
            let ey0 = el.y.min(el.y + el.height);
            let ex1 = el.x.max(el.x + el.width);
            let ey1 = el.y.max(el.y + el.height);
            if ex0 <= x1 && ex1 >= x0 && ey0 <= y1 && ey1 >= y0 {
                hits.push(i);
            }
        }
        if additive {
            self.selected = base.to_vec();
            for h in hits {
                if !self.selected.contains(&h) {
                    self.selected.push(h);
                }
            }
        } else {
            self.selected = hits;
        }
    }

    /// First non-deleted selected element (drives the properties panel's
    /// displayed/active values).
    pub fn primary_selected(&self) -> Option<&Element> {
        self.selected
            .iter()
            .filter_map(|&i| self.file.elements.get(i))
            .find(|el| !el.is_deleted)
    }

    /// Apply a mutation to every selected, non-deleted element as one undo
    /// step. No-op (and no undo entry) when nothing is selected/changed.
    fn mutate_selected(&mut self, f: impl Fn(&mut Element)) {
        if self.selected.is_empty() {
            return;
        }
        self.push_undo();
        let mut changed = false;
        for &i in &self.selected {
            if let Some(el) = self.file.elements.get_mut(i)
                && !el.is_deleted
            {
                f(el);
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
        } else {
            self.discard_last_undo();
        }
    }

    pub fn set_selected_stroke_color(&mut self, color: &str) {
        let c = color.to_string();
        self.mutate_selected(|el| el.stroke_color = c.clone());
    }

    pub fn set_selected_background(&mut self, color: &str) {
        let c = color.to_string();
        self.mutate_selected(|el| el.background_color = c.clone());
    }

    pub fn set_selected_fill_style(&mut self, style: &str) {
        let s = style.to_string();
        self.mutate_selected(|el| el.fill_style = s.clone());
    }

    pub fn set_selected_stroke_style(&mut self, style: &str) {
        let s = style.to_string();
        self.mutate_selected(|el| el.stroke_style = s.clone());
    }

    pub fn set_selected_stroke_width(&mut self, width: f64) {
        self.mutate_selected(|el| el.stroke_width = width);
    }

    // ─── Connector bindings ─────────────────────────────────────────────

    /// Find the nearest shape connection point to `(wx, wy)` within `tol`
    /// (world units), skipping `exclude`. Returns the bound element's id, the
    /// relative anchor `(rx, ry)`, and the anchor's world position.
    pub fn connection_point_at(
        &self,
        wx: f64,
        wy: f64,
        tol: f64,
        exclude: Option<usize>,
    ) -> Option<(String, f64, f64, f64, f64)> {
        let mut best: Option<(f64, (String, f64, f64, f64, f64))> = None;
        for (i, el) in self.file.elements.iter().enumerate() {
            if el.is_deleted || Some(i) == exclude {
                continue;
            }
            let Some(rels) = anchor_rels(&el.element_type) else {
                continue;
            };
            for (rx, ry) in rels {
                let ax = el.x + rx * el.width;
                let ay = el.y + ry * el.height;
                let d = ((ax - wx).powi(2) + (ay - wy).powi(2)).sqrt();
                if d <= tol && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, (el.id.clone(), rx, ry, ax, ay)));
                }
            }
        }
        best.map(|(_, v)| v)
    }

    /// Set or clear an arrow endpoint binding.
    pub fn set_binding(&mut self, index: usize, is_start: bool, binding: Option<Binding>) {
        if let Some(el) = self.file.elements.get_mut(index) {
            if is_start {
                el.memex_start_binding = binding;
            } else {
                el.memex_end_binding = binding;
            }
        }
    }

    /// Recompute the endpoints of every bound connector from its shapes'
    /// current geometry. Interior waypoints are preserved (in absolute terms).
    pub fn reroute_bindings(&mut self) {
        use std::collections::HashMap;
        let boxes: HashMap<String, (f64, f64, f64, f64)> = self
            .file
            .elements
            .iter()
            .filter(|e| !e.is_deleted)
            .map(|e| (e.id.clone(), (e.x, e.y, e.width, e.height)))
            .collect();
        for el in self.file.elements.iter_mut() {
            if el.is_deleted
                || (el.memex_start_binding.is_none() && el.memex_end_binding.is_none())
            {
                continue;
            }
            let Some(points) = el.points.clone() else {
                continue;
            };
            if points.len() < 2 {
                continue;
            }
            let mut abs: Vec<(f64, f64)> =
                points.iter().map(|p| (el.x + p[0], el.y + p[1])).collect();
            if let Some(b) = &el.memex_start_binding
                && let Some(&(bx, by, bw, bh)) = boxes.get(&b.element_id)
            {
                abs[0] = (bx + b.rx * bw, by + b.ry * bh);
            }
            if let Some(b) = &el.memex_end_binding
                && let Some(&(bx, by, bw, bh)) = boxes.get(&b.element_id)
            {
                let n = abs.len() - 1;
                abs[n] = (bx + b.rx * bw, by + b.ry * bh);
            }
            let (ox, oy) = abs[0];
            el.x = ox;
            el.y = oy;
            let rel: Vec<[f64; 2]> = abs.iter().map(|(x, y)| [x - ox, y - oy]).collect();
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for p in &rel {
                min_x = min_x.min(p[0]);
                min_y = min_y.min(p[1]);
                max_x = max_x.max(p[0]);
                max_y = max_y.max(p[1]);
            }
            el.width = max_x - min_x;
            el.height = max_y - min_y;
            el.points = Some(rel);
        }
    }

    /// World-space (x, y) of each selected element, for drag anchoring.
    pub fn selected_origins(&self) -> Vec<(usize, f64, f64)> {
        self.selected
            .iter()
            .filter_map(|&i| self.file.elements.get(i).map(|el| (i, el.x, el.y)))
            .collect()
    }

    /// Move a specific element to an absolute world position.
    pub fn set_element_position(&mut self, index: usize, x: f64, y: f64) {
        if let Some(el) = self.file.elements.get_mut(index) {
            el.x = x;
            el.y = y;
        }
    }

    // ─── Creation ───────────────────────────────────────────────────────

    /// Create a zero-size element for a shape/line/arrow/freedraw tool at the
    /// given world point, append it, and return its index. Returns `None` for
    /// the Select and Text tools (text is created via the minibuffer).
    pub fn create_element(&mut self, tool: Tool, wx: f64, wy: f64) -> Option<usize> {
        let ty = tool.element_type()?;
        if tool == Tool::Text {
            return None;
        }
        let mut el = Element::base(gen_id(), ty, wx, wy, 0.0, 0.0);
        match tool {
            Tool::Arrow => {
                el.points = Some(vec![[0.0, 0.0], [0.0, 0.0]]);
                el.end_arrowhead = Some(serde_json::Value::String("arrow".to_string()));
            }
            Tool::Line => el.points = Some(vec![[0.0, 0.0], [0.0, 0.0]]),
            Tool::Draw => el.points = Some(vec![[0.0, 0.0]]),
            _ => {}
        }
        self.file.elements.push(el);
        self.dirty = true;
        Some(self.file.elements.len() - 1)
    }

    /// Update an in-progress shape/line element as the pointer drags.
    pub fn update_creation(&mut self, index: usize, tool: Tool, start: (f64, f64), cur: (f64, f64)) {
        let Some(el) = self.file.elements.get_mut(index) else {
            return;
        };
        match tool {
            Tool::Line | Tool::Arrow => {
                el.x = start.0;
                el.y = start.1;
                el.points = Some(vec![[0.0, 0.0], [cur.0 - start.0, cur.1 - start.1]]);
                el.width = (cur.0 - start.0).abs();
                el.height = (cur.1 - start.1).abs();
            }
            _ => {
                el.x = start.0.min(cur.0);
                el.y = start.1.min(cur.1);
                el.width = (cur.0 - start.0).abs();
                el.height = (cur.1 - start.1).abs();
            }
        }
    }

    /// Append a point to an in-progress freedraw element.
    pub fn append_freedraw_point(&mut self, index: usize, start: (f64, f64), cur: (f64, f64)) {
        let Some(el) = self.file.elements.get_mut(index) else {
            return;
        };
        let pts = el.points.get_or_insert_with(Vec::new);
        pts.push([cur.0 - start.0, cur.1 - start.1]);
        // Recompute bbox (relative to el origin = start).
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for p in pts.iter() {
            min_x = min_x.min(p[0]);
            min_y = min_y.min(p[1]);
            max_x = max_x.max(p[0]);
            max_y = max_y.max(p[1]);
        }
        el.x = start.0 + min_x;
        el.y = start.1 + min_y;
        el.width = max_x - min_x;
        el.height = max_y - min_y;
    }

    /// Finish an in-progress creation: drop degenerate (click-without-drag)
    /// shapes, otherwise select the new element. Returns to the Select tool.
    pub fn finish_creation(&mut self, index: usize) {
        let degenerate = self
            .file
            .elements
            .get(index)
            .map(|el| el.element_type != "freedraw" && el.width < 3.0 && el.height < 3.0)
            .unwrap_or(false);
        if degenerate && index + 1 == self.file.elements.len() {
            self.file.elements.pop();
            self.selected.clear();
            // Cancel the undo snapshot pushed before this (no-op) creation.
            self.discard_last_undo();
        } else {
            self.select_only(index);
        }
        self.tool = Tool::Select;
        self.dirty = true;
    }

    // ─── Inline text editing ────────────────────────────────────────────

    /// Index of the text element being edited, if any.
    pub fn editing_text(&self) -> Option<usize> {
        self.editing_text
    }

    /// Begin a new text element at the world point and enter editing mode.
    pub fn start_text(&mut self, wx: f64, wy: f64) {
        self.push_undo();
        let font = 20.0;
        let mut el = Element::base(gen_id(), "text", wx, wy, font * 0.6, font * 1.25);
        el.text = Some(String::new());
        el.font_size = Some(font);
        self.file.elements.push(el);
        let idx = self.file.elements.len() - 1;
        self.select_only(idx);
        self.editing_text = Some(idx);
        self.dirty = true;
    }

    /// Begin editing an existing text element.
    pub fn edit_existing_text(&mut self, index: usize) {
        if self
            .file
            .elements
            .get(index)
            .is_some_and(|el| el.element_type == "text" && !el.is_deleted)
        {
            self.push_undo();
            self.select_only(index);
            self.editing_text = Some(index);
        }
    }

    /// Append typed text to the element being edited.
    pub fn text_input(&mut self, s: &str) {
        if let Some(idx) = self.editing_text {
            if let Some(el) = self.file.elements.get_mut(idx) {
                el.text.get_or_insert_with(String::new).push_str(s);
            }
            self.recompute_text_metrics(idx);
            self.dirty = true;
        }
    }

    /// Delete the last character of the element being edited.
    pub fn text_backspace(&mut self) {
        if let Some(idx) = self.editing_text {
            if let Some(el) = self.file.elements.get_mut(idx) {
                el.text.get_or_insert_with(String::new).pop();
            }
            self.recompute_text_metrics(idx);
            self.dirty = true;
        }
    }

    /// Finish text editing: drop an empty element (cancelling its undo step),
    /// otherwise keep it. Always leaves editing mode.
    pub fn finish_text_editing(&mut self) {
        let Some(idx) = self.editing_text.take() else {
            return;
        };
        let empty = self
            .file
            .elements
            .get(idx)
            .map(|el| el.text.as_deref().unwrap_or("").is_empty())
            .unwrap_or(true);
        if empty && idx + 1 == self.file.elements.len() {
            self.file.elements.pop();
            self.selected.clear();
            self.discard_last_undo();
        }
    }

    fn recompute_text_metrics(&mut self, index: usize) {
        if let Some(el) = self.file.elements.get_mut(index) {
            let font = el.font_size.unwrap_or(20.0);
            let text = el.text.as_deref().unwrap_or("");
            let lines = text.split('\n').count().max(1) as f64;
            let cols = text
                .split('\n')
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0) as f64;
            el.width = (cols * font * 0.6).max(font * 0.6);
            el.height = lines * font * 1.25;
        }
    }

    /// Mark the selected elements deleted (excalidraw `isDeleted`).
    pub fn delete_selected(&mut self) -> usize {
        let has_live = self
            .selected
            .iter()
            .any(|&i| self.file.elements.get(i).is_some_and(|el| !el.is_deleted));
        if has_live {
            self.push_undo();
        }
        let mut n = 0;
        for &i in &self.selected {
            if let Some(el) = self.file.elements.get_mut(i)
                && !el.is_deleted
            {
                el.is_deleted = true;
                n += 1;
            }
        }
        self.selected.clear();
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    /// Persist the document to disk, clearing the dirty flag on success.
    pub fn save(&mut self) -> Result<(), String> {
        self.file.save(&self.path)?;
        self.dirty = false;
        Ok(())
    }

    // ─── PaneItem interface ─────────────────────────────────────────────

    pub fn commands() -> Vec<Command> {
        vec![
            Command {
                id: "diagram-zoom-in",
                name: "Diagram: Zoom In",
                description: "Zoom into the diagram",
                aliases: &[],
                binding: Some("+"),
            },
            Command {
                id: "diagram-zoom-out",
                name: "Diagram: Zoom Out",
                description: "Zoom out of the diagram",
                aliases: &[],
                binding: Some("-"),
            },
            Command {
                id: "diagram-reset-zoom",
                name: "Diagram: Reset Zoom",
                description: "Reset zoom to 100%",
                aliases: &[],
                binding: Some("0"),
            },
            Command {
                id: "diagram-center",
                name: "Diagram: Center",
                description: "Reset the diagram pan to origin",
                aliases: &[],
                binding: Some("c"),
            },
            Command {
                id: "diagram-save",
                name: "Diagram: Save",
                description: "Write the diagram to disk",
                aliases: &[],
                binding: Some("s"),
            },
            Command {
                id: "diagram-delete",
                name: "Diagram: Delete Selection",
                description: "Delete the selected elements",
                aliases: &[],
                binding: Some("d"),
            },
            Command {
                id: "diagram-tool-select",
                name: "Diagram: Select Tool",
                description: "Switch to the selection tool",
                aliases: &[],
                binding: Some("v"),
            },
            Command {
                id: "diagram-tool-rectangle",
                name: "Diagram: Rectangle Tool",
                description: "Draw rectangles",
                aliases: &[],
                binding: Some("r"),
            },
            Command {
                id: "diagram-tool-ellipse",
                name: "Diagram: Ellipse Tool",
                description: "Draw ellipses",
                aliases: &[],
                binding: Some("o"),
            },
            Command {
                id: "diagram-tool-diamond",
                name: "Diagram: Diamond Tool",
                description: "Draw diamonds",
                aliases: &[],
                binding: None,
            },
            Command {
                id: "diagram-tool-arrow",
                name: "Diagram: Arrow Tool",
                description: "Draw arrows",
                aliases: &[],
                binding: Some("a"),
            },
            Command {
                id: "diagram-tool-line",
                name: "Diagram: Line Tool",
                description: "Draw lines",
                aliases: &[],
                binding: Some("l"),
            },
            Command {
                id: "diagram-tool-draw",
                name: "Diagram: Freedraw Tool",
                description: "Freehand drawing",
                aliases: &[],
                binding: Some("p"),
            },
            Command {
                id: "diagram-tool-text",
                name: "Diagram: Text Tool",
                description: "Add text",
                aliases: &[],
                binding: Some("t"),
            },
            Command {
                id: "diagram-undo",
                name: "Diagram: Undo",
                description: "Undo the last change",
                aliases: &[],
                binding: Some("u"),
            },
            Command {
                id: "diagram-redo",
                name: "Diagram: Redo",
                description: "Redo the last undone change",
                aliases: &[],
                binding: None,
            },
        ]
    }

    pub fn execute_command(
        &mut self,
        cmd_id: &str,
        _viewport: (f32, f32),
        _vim_enabled: bool,
        _cx: &mut Context<Self>,
    ) -> CommandOutcome {
        let actions = match cmd_id {
            "diagram-zoom-in" => {
                self.zoom = (self.zoom * 1.2).min(5.0);
                vec![]
            }
            "diagram-zoom-out" => {
                self.zoom = (self.zoom / 1.2).max(0.1);
                vec![]
            }
            "diagram-reset-zoom" => {
                self.zoom = 1.0;
                vec![]
            }
            "diagram-center" => {
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                vec![]
            }
            "diagram-save" => match self.save() {
                Ok(()) => vec![ItemAction::SetMessage("Diagram saved".into())],
                Err(e) => vec![ItemAction::SetMessage(format!("Save failed: {}", e))],
            },
            "diagram-delete" => {
                let n = self.delete_selected();
                if n > 0 {
                    vec![ItemAction::SetMessage(format!("Deleted {} element(s)", n))]
                } else {
                    vec![]
                }
            }
            "diagram-tool-select" => self.set_tool(Tool::Select),
            "diagram-tool-rectangle" => self.set_tool(Tool::Rectangle),
            "diagram-tool-ellipse" => self.set_tool(Tool::Ellipse),
            "diagram-tool-diamond" => self.set_tool(Tool::Diamond),
            "diagram-tool-arrow" => self.set_tool(Tool::Arrow),
            "diagram-tool-line" => self.set_tool(Tool::Line),
            "diagram-tool-draw" => self.set_tool(Tool::Draw),
            "diagram-tool-text" => self.set_tool(Tool::Text),
            "diagram-undo" => {
                if self.undo() {
                    vec![ItemAction::SetMessage("Undo".into())]
                } else {
                    vec![ItemAction::SetMessage("Nothing to undo".into())]
                }
            }
            "diagram-redo" => {
                if self.redo() {
                    vec![ItemAction::SetMessage("Redo".into())]
                } else {
                    vec![ItemAction::SetMessage("Nothing to redo".into())]
                }
            }
            _ => return CommandOutcome::Unhandled,
        };
        CommandOutcome::handled(actions)
    }

    fn set_tool(&mut self, tool: Tool) -> Vec<ItemAction> {
        self.finish_text_editing();
        self.tool = tool;
        if tool != Tool::Select {
            self.selected.clear();
        }
        vec![]
    }

    pub fn get_candidates(&self, _delegate_id: &str, _input: &str) -> Vec<Candidate> {
        vec![]
    }

    pub fn handle_confirm(
        &mut self,
        _delegate_id: &str,
        _input: &str,
        _candidate: Option<&Candidate>,
        _cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        vec![]
    }

    pub fn on_input_changed(&mut self, _delegate_id: &str, _input: &str, _cx: &mut Context<Self>) {}
}

/// Relative connection anchors (center + 4 edge midpoints) for a box-like
/// element type, or `None` for connectors/freedraw.
fn anchor_rels(element_type: &str) -> Option<[(f64, f64); 5]> {
    matches!(
        element_type,
        "rectangle" | "ellipse" | "diamond" | "text" | "image" | "frame"
    )
    .then_some([
        (0.5, 0.5),
        (0.5, 0.0),
        (1.0, 0.5),
        (0.5, 1.0),
        (0.0, 0.5),
    ])
}

/// Generate an excalidraw-style random element id.
fn gen_id() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    (0..16)
        .map(|_| CHARS[fastrand::usize(..CHARS.len())] as char)
        .collect()
}
