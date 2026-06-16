use crate::command::Command;
use crate::minibuffer::Candidate;

/// Global commands available in every item context.
pub(crate) fn global_commands() -> Vec<Command> {
    vec![
        Command {
            id: "write",
            name: "Save",
            description: "Save current note to disk",
            aliases: &["w", "save"],
            binding: Some(":w"),
        },
        Command {
            id: "quit",
            name: "Close Secondary or Quit",
            description: "Close the focused secondary slot, otherwise quit",
            aliases: &["q", "exit"],
            binding: Some(":q"),
        },
        Command {
            id: "wq",
            name: "Save and Quit",
            description: "Save current note and quit",
            aliases: &["x"],
            binding: Some(":wq"),
        },
        Command {
            id: "vault-switch",
            name: "Switch Vault",
            description: "Switch to a recent vault",
            aliases: &["vault", "vaults", "switch-vault"],
            binding: Some(":vault-switch"),
        },
        Command {
            id: "vault-open",
            name: "Open Vault",
            description: "Browse filesystem to open a vault",
            aliases: &["open-vault"],
            binding: Some(":vault-open"),
        },
        Command {
            id: "notes",
            name: "Find Note",
            description: "Search and open a note in current vault",
            aliases: &["find-note", "find", "note"],
            binding: Some("Ctrl+P"),
        },
        Command {
            id: "edit",
            name: "Edit File",
            description: "Open a file by path",
            aliases: &["e", "open"],
            binding: Some(":e <path>"),
        },
        Command {
            id: "set",
            name: "Set Option",
            description: "Set an editor option",
            aliases: &[],
            binding: Some(":set <option>"),
        },
        Command {
            id: "set-vim",
            name: "Enable Vim Mode",
            description: "Enable vim keybindings",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "set-novim",
            name: "Disable Vim Mode",
            description: "Disable vim keybindings",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "nohlsearch",
            name: "Clear Search Highlighting",
            description: "Remove search result highlighting",
            aliases: &["noh"],
            binding: Some(":noh"),
        },
        Command {
            id: "toggle-vim",
            name: "Toggle Vim Mode",
            description: "Toggle vim mode on/off",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "open-graph",
            name: "Open Graph",
            description: "Open the vault graph in the secondary slot",
            aliases: &["graph"],
            binding: None,
        },
        Command {
            id: "diagram-new",
            name: "New Diagram",
            description: "Create a new excalidraw diagram, link it, and open the editor",
            aliases: &["new-diagram", "diagram"],
            binding: None,
        },
        Command {
            id: "diagram-import",
            name: "Import Diagram",
            description: "Import a .drawio or .excalidraw file into diagrams/ and link it",
            aliases: &["import-diagram"],
            binding: Some(":diagram-import <path>"),
        },
        Command {
            id: "toggle-secondary-maximize",
            name: "Toggle Secondary Maximize",
            description: "Toggle the secondary slot between side-by-side and full width",
            aliases: &["secondary-maximize", "maximize-tool"],
            binding: None,
        },
        Command {
            id: "close-secondary",
            name: "Close Secondary",
            description: "Close the secondary slot and return focus to the editor",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "backlinks",
            name: "Backlinks",
            description: "Show notes that link to the current note",
            aliases: &["bl", "references"],
            binding: None,
        },
        Command {
            id: "today",
            name: "Today's Journal",
            description: "Open or create today's journal note",
            aliases: &["daily", "journal"],
            binding: None,
        },
        Command {
            id: "tags",
            name: "Tags",
            description: "List all tags in the vault",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "tag",
            name: "Tag Search",
            description: "Notes with a specific tag",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "orphans",
            name: "Orphan Notes",
            description: "Notes with no incoming or outgoing links",
            aliases: &[],
            binding: None,
        },
        Command {
            id: "search-content",
            name: "Search Content",
            description: "Full-text search across notes",
            aliases: &["search", "grep"],
            binding: Some("Ctrl+Shift+F"),
        },
        Command {
            id: "rename",
            name: "Rename Note",
            description: "Update the current note's title (no file rename - IDs stay stable)",
            aliases: &["rn"],
            binding: Some(":rename <title>"),
        },
        Command {
            id: "insert-links-by-tag",
            name: "Insert Links by Tag",
            description: "Insert wikilinks to all notes with a tag (MOC helper)",
            aliases: &["moc"],
            binding: Some(":moc <tag>"),
        },
        Command {
            id: "vault-forget",
            name: "Forget Vault",
            description: "Remove a vault from the recent-vaults list",
            aliases: &["forget-vault"],
            binding: Some(":vault-forget <path>"),
        },
        Command {
            id: "toggle-backlinks",
            name: "Toggle Backlinks",
            description: "Show or hide backlinks in the secondary slot",
            aliases: &["backlinks-panel"],
            binding: Some("Ctrl+Shift+B"),
        },
        Command {
            id: "attach",
            name: "Attach from Clipboard",
            description: "Save clipboard image to attachments/ and insert a link",
            aliases: &["paste-image"],
            binding: None,
        },
        Command {
            id: "theme",
            name: "Theme",
            description: "Select an application theme",
            aliases: &["themes", "color-scheme"],
            binding: Some(":theme"),
        },
    ]
}

pub(crate) fn command_to_candidate(cmd: &Command) -> Candidate {
    let detail = if let Some(binding) = cmd.binding {
        format!("{}  [{}]", cmd.description, binding)
    } else {
        cmd.description.to_string()
    };
    Candidate {
        label: cmd.name.to_string(),
        detail: Some(detail),
        is_action: false,
        data: cmd.id.to_string(),
    }
}
