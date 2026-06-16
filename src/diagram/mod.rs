//! Diagram view -- a native excalidraw-class diagram editor.
//!
//! Diagrams are stored as `.excalidraw` JSON files under the vault's
//! `diagrams/` folder and linked from notes via `[[name.excalidraw]]`.
//! Clicking such a link opens the diagram in the secondary pane (same slot the
//! PDF viewer uses).
//!
//! Phase 0: load + render skeleton + pan/zoom scaffolding. Drawing tools,
//! selection, editing, and import live in later phases (see PLAN.md).

mod model;
mod view;

use std::path::{Path, PathBuf};

use gpui::*;

use crate::command::Command;
use crate::minibuffer::Candidate;
use crate::pane::{CommandOutcome, ItemAction};

pub use model::{Element, ExcalidrawFile};
pub use view::{DiagramView, DiagramViewEvent};

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
            _ => return CommandOutcome::Unhandled,
        };
        CommandOutcome::handled(actions)
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
