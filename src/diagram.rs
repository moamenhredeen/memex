use std::path::Path;

use diagram_view::diagram_core::{
    Graph, commands as diagram_commands, interop::import_drawio, io::native,
};
use diagram_view::{CommandOutcome, DiagramEvent, DiagramTheme};

use crate::command::Command;
use crate::pane::ItemAction;
use crate::theme::Theme;

pub use diagram_view::{ChromeConfig, DiagramState, DiagramView, Mode};

pub type DiagramViewEvent = DiagramEvent;

pub fn theme_from_memex(theme: Theme) -> DiagramTheme {
    DiagramTheme {
        background: theme.background,
        surface: theme.surface,
        grid: theme.selection,
        selection: theme.selection,
        border: theme.border,
        text: theme.text,
        text_muted: theme.text_muted,
        accent: theme.accent,
    }
}

pub fn commands() -> Vec<Command> {
    diagram_commands()
        .iter()
        .filter(|cmd| {
            !cmd.id.contains(".tool.")
                && !matches!(
                    cmd.id,
                    "diagram.delete"
                        | "diagram.undo"
                        | "diagram.redo"
                        | "diagram.group"
                        | "diagram.ungroup"
                        | "diagram.connector.straight"
                        | "diagram.connector.orthogonal"
                        | "diagram.connector.curved"
                )
        })
        .map(|cmd| Command {
            id: cmd.id,
            name: cmd.name,
            description: cmd.description,
            aliases: cmd.aliases,
            binding: cmd.default_binding,
        })
        .collect()
}

pub fn load_graph(path: &Path) -> Result<Graph, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read diagram: {e}"))?;
    native::from_json(&bytes).map_err(|e| format!("failed to parse diagram: {e}"))
}

pub fn import_graph(path: &Path) -> Result<Graph, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read diagram: {e}"))?;
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("drawio") | Some("xml") => {
            let xml = std::str::from_utf8(&bytes)
                .map_err(|e| format!("diagram XML is not valid UTF-8: {e}"))?;
            import_drawio(xml).map_err(|e| e.to_string())
        }
        Some("diagram") => native::from_json(&bytes).map_err(|e| e.to_string()),
        Some(other) => Err(format!("unsupported diagram format: .{other}")),
        None => Err("unsupported diagram format: missing extension".into()),
    }
}

pub fn save_graph(path: &Path, state: &DiagramState) -> Result<(), String> {
    let json = native::to_json_pretty(state.graph()).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("failed to write diagram: {e}"))
}

pub fn command_actions(outcome: CommandOutcome) -> Vec<ItemAction> {
    match outcome {
        CommandOutcome::Handled | CommandOutcome::Ignored => Vec::new(),
        CommandOutcome::Message(message) => vec![ItemAction::SetMessage(message)],
        CommandOutcome::RequestSave => vec![ItemAction::SetMessage(
            "Use the host save command for diagrams".into(),
        )],
    }
}
