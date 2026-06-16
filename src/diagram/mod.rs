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

pub use model::ExcalidrawFile;
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
