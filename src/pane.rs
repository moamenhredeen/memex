//! Pane item system — Zed-style architecture for view-specific behavior.
//!
//! Each pane holds one `ActiveItem` (editor, PDF, graph, etc.).
//! The item owns its view, commands, keybindings, and minibuffer delegates.
//! The app shell dispatches to the active item — no central match arms.
//!
//! To add a new item type:
//! 1. Add a variant to `ActiveItem`
//! 2. Implement the same methods on the new state type
//! 3. Add dispatch arms in ActiveItem methods
//! 4. Done — no changes to app.rs needed

use gpui::*;

use crate::command::Command;
use crate::editor::{EditorState, EditorView};
use crate::minibuffer::Candidate;
use crate::pdf::{PdfState, PdfView};

/// Side effects an item wants the app shell to perform.
///
/// Items can't directly access the minibuffer or clipboard — they return
/// actions and the app processes them. This keeps items decoupled from the shell.
#[derive(Clone, Debug)]
pub enum ItemAction {
    /// Show a message in the minibuffer echo area.
    SetMessage(String),
    /// Open a minibuffer delegate owned by this item.
    ActivateDelegate {
        /// Delegate identifier (e.g., "pdf-toc", "pdf-search").
        id: String,
        /// Prompt text shown in minibuffer.
        prompt: String,
        /// Whether to highlight the input query in candidate labels.
        #[allow(dead_code)]
        highlight_input: bool,
    },
    /// Dismiss the minibuffer.
    Dismiss,
    /// Copy text to the system clipboard.
    WriteClipboard(String),
}

/// The active content in a pane. Each variant wraps a state+view entity pair.
///
/// When you add a new item type (graph view, backlinks, etc.), add a variant
/// here and implement the same methods on the state type.
pub enum ActiveItem {
    Editor {
        state: Entity<EditorState>,
        view: Entity<EditorView>,
    },
    Pdf {
        state: Entity<PdfState>,
        view: Entity<PdfView>,
    },
}

impl ActiveItem {
    /// Display name for the mode-line badge.
    #[allow(dead_code)]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Editor { .. } => "Markdown",
            Self::Pdf { .. } => "PDF",
        }
    }

    /// Keymap layers to activate when this item gains focus.
    pub fn keymap_layers(&self) -> Vec<&'static str> {
        match self {
            Self::Editor { .. } => vec!["vim:normal", "vim:motion", "leader", "markdown"],
            Self::Pdf { .. } => vec!["pdf"],
        }
    }

    /// Keymap layers to deactivate when this item gains focus.
    pub fn deactivate_layers(&self) -> Vec<&'static str> {
        match self {
            Self::Editor { .. } => vec!["pdf"],
            Self::Pdf { .. } => vec![
                "vim:normal", "vim:motion", "vim:insert", "vim:visual",
                "vim:operator-pending", "leader", "markdown",
            ],
        }
    }

    /// Commands available in the command palette when this item is active.
    pub fn commands(&self) -> Vec<Command> {
        match self {
            Self::Editor { .. } => EditorState::commands(),
            Self::Pdf { .. } => PdfState::commands(),
        }
    }

    /// Execute a command, returning side-effect actions for the app.
    pub fn execute_command(
        &self,
        cmd_id: &str,
        viewport: (f32, f32),
        vim_enabled: bool,
        cx: &mut Context<crate::app::Memex>,
    ) -> Vec<ItemAction> {
        match self {
            Self::Editor { state, .. } => {
                let state = state.clone();
                state.update(cx, |s, cx| s.item_execute_command(cmd_id, viewport, cx))
            }
            Self::Pdf { state, .. } => {
                let state = state.clone();
                state.update(cx, |s, cx| s.execute_command(cmd_id, viewport, vim_enabled, cx))
            }
        }
    }

    /// Get candidates for a mode-owned minibuffer delegate.
    pub fn get_candidates(&self, delegate_id: &str, input: &str, cx: &App) -> Vec<Candidate> {
        match self {
            Self::Editor { state, .. } => {
                state.read(cx).item_get_candidates(delegate_id, input)
            }
            Self::Pdf { state, .. } => {
                state.read(cx).get_candidates(delegate_id, input)
            }
        }
    }

    /// Handle confirm for a mode-owned minibuffer delegate.
    pub fn handle_confirm(
        &self,
        delegate_id: &str,
        input: &str,
        candidate: Option<&Candidate>,
        cx: &mut Context<crate::app::Memex>,
    ) -> Vec<ItemAction> {
        match self {
            Self::Editor { state, .. } => {
                let state = state.clone();
                state.update(cx, |s, _cx| s.item_handle_confirm(delegate_id, input, candidate))
            }
            Self::Pdf { state, .. } => {
                let state = state.clone();
                state.update(cx, |s, cx| s.handle_confirm(delegate_id, input, candidate, cx))
            }
        }
    }

    /// Called when the minibuffer input changes (for live search, etc.).
    pub fn on_input_changed(
        &self,
        delegate_id: &str,
        input: &str,
        cx: &mut Context<crate::app::Memex>,
    ) {
        match self {
            Self::Editor { .. } => {}
            Self::Pdf { state, .. } => {
                let state = state.clone();
                state.update(cx, |s, cx| s.on_input_changed(delegate_id, input, cx));
            }
        }
    }

    /// Whether the given delegate highlights the query in candidate labels.
    pub fn highlight_input(&self, delegate_id: &str) -> bool {
        match self {
            Self::Editor { .. } => false,
            Self::Pdf { .. } => delegate_id == "pdf-search",
        }
    }

    /// Whether this item is a PDF viewer.
    pub fn is_pdf(&self) -> bool {
        matches!(self, Self::Pdf { .. })
    }

    /// Whether this item is the editor.
    pub fn is_editor(&self) -> bool {
        matches!(self, Self::Editor { .. })
    }

    /// Get the view element for rendering.
    pub fn view_element(&self) -> AnyView {
        match self {
            Self::Editor { view, .. } => view.clone().into(),
            Self::Pdf { view, .. } => view.clone().into(),
        }
    }

    /// Focus this item's view.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        match self {
            Self::Editor { state, .. } => state.read(cx).focus(window),
            Self::Pdf { state, .. } => state.read(cx).focus(window),
        }
    }

    /// Get position text for the mode-line (e.g., "PDF 3/10 120%" or "L5 C12").
    pub fn position_text(&self, viewport_height: f32, cx: &App) -> String {
        match self {
            Self::Editor { state, .. } => {
                let es = state.read(cx);
                let content = es.content();
                let cursor = es.cursor;
                let mut pos = cursor.min(content.len());
                while pos > 0 && !content.is_char_boundary(pos) {
                    pos -= 1;
                }
                let before = &content[..pos];
                let line_num = before.matches('\n').count() + 1;
                let col_num = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
                format!("L{} C{}", line_num, col_num)
            }
            Self::Pdf { state, .. } => {
                let ps = state.read(cx);
                let (first, _) = ps.visible_range(viewport_height);
                let current = first + 1;
                let zoom_pct = (ps.zoom * 100.0) as u32;
                format!("PDF {}/{} {}%", current, ps.page_count, zoom_pct)
            }
        }
    }

    /// Mode-line badge info: (label, background_color).
    pub fn mode_badge(&self) -> (&'static str, u32) {
        match self {
            Self::Editor { .. } => ("EDI", 0x268BD2),  // solarized blue (overridden by vim state)
            Self::Pdf { .. } => ("PDF", 0xCB4B16),      // solarized orange
        }
    }
}
