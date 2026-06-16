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
pub use model::{Element, ExcalidrawFile};
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
    /// Unsaved changes pending.
    pub dirty: bool,
    /// World position where a pending text element will be created (set when
    /// the Text tool is clicked, consumed when the minibuffer confirms).
    pending_text_pos: Option<(f64, f64)>,
    /// Screen origin of the canvas, stashed each paint for hit-testing.
    origin_x: f32,
    origin_y: f32,
    pub focus_handle: FocusHandle,
}

impl DiagramState {
    pub fn new(path: &Path, file: ExcalidrawFile, cx: &mut App) -> Self {
        Self {
            path: path.to_path_buf(),
            file,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            tool: Tool::Select,
            selected: Vec::new(),
            dirty: false,
            pending_text_pos: None,
            origin_x: 0.0,
            origin_y: 0.0,
            focus_handle: cx.focus_handle(),
        }
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
        } else {
            self.select_only(index);
        }
        self.tool = Tool::Select;
        self.dirty = true;
    }

    /// Stash where the next text element will be placed (Text-tool click).
    pub fn set_pending_text(&mut self, wx: f64, wy: f64) {
        self.pending_text_pos = Some((wx, wy));
    }

    /// Create a text element from confirmed minibuffer input at the pending
    /// position. Returns to the Select tool.
    pub fn commit_pending_text(&mut self, text: &str) {
        let Some((wx, wy)) = self.pending_text_pos.take() else {
            return;
        };
        self.tool = Tool::Select;
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let font = 20.0;
        let lines = text.split('\n').count().max(1) as f64;
        let width = text
            .split('\n')
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0) as f64
            * font
            * 0.6;
        let mut el = Element::base(gen_id(), "text", wx, wy, width, lines * font * 1.25);
        el.text = Some(text.to_string());
        el.font_size = Some(font);
        self.file.elements.push(el);
        self.select_only(self.file.elements.len() - 1);
        self.dirty = true;
    }

    /// Mark the selected elements deleted (excalidraw `isDeleted`).
    pub fn delete_selected(&mut self) -> usize {
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
            "diagram-text-input" => vec![ItemAction::ActivateDelegate {
                id: "diagram-text".into(),
                prompt: "Text:".into(),
                highlight_input: false,
            }],
            _ => return CommandOutcome::Unhandled,
        };
        CommandOutcome::handled(actions)
    }

    fn set_tool(&mut self, tool: Tool) -> Vec<ItemAction> {
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
        delegate_id: &str,
        input: &str,
        _candidate: Option<&Candidate>,
        _cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        if delegate_id == "diagram-text" {
            self.commit_pending_text(input);
            return vec![ItemAction::Dismiss];
        }
        vec![]
    }

    pub fn on_input_changed(&mut self, _delegate_id: &str, _input: &str, _cx: &mut Context<Self>) {}
}

/// Generate an excalidraw-style random element id.
fn gen_id() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    (0..16)
        .map(|_| CHARS[fastrand::usize(..CHARS.len())] as char)
        .collect()
}
