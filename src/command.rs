use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::minibuffer::Candidate;

const MAX_RESULTS: usize = 15;

/// A command that can be discovered and executed from the command palette.
#[derive(Clone, Debug)]
pub struct Command {
    /// Unique identifier used for execution dispatch.
    pub id: &'static str,
    /// Human-readable name shown in the palette.
    pub name: &'static str,
    /// Short description of what the command does.
    pub description: &'static str,
    /// Alternative names that also match (e.g. "w" for "write").
    pub aliases: &'static [&'static str],
    /// Optional keybinding hint shown in the palette.
    pub binding: Option<&'static str>,
}

/// Global registry of all available commands.
///
/// Commands are registered at startup and can be queried by the command palette
/// for fuzzy-filtered completion. The registry is the single source of truth for
/// "what commands exist" — mirroring Zed's global action registry pattern.
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: Vec::new(),
        };
        registry.register_builtins();
        registry
    }

    /// Register a new command.
    pub fn register(&mut self, command: Command) {
        self.commands.push(command);
    }

    /// Get all registered commands.
    pub fn all(&self) -> &[Command] {
        &self.commands
    }

    /// Look up a command by id or any of its aliases.
    pub fn lookup(&self, name: &str) -> Option<&Command> {
        self.commands.iter().find(|c| {
            c.id == name || c.aliases.contains(&name)
        })
    }

    /// Fuzzy-filter commands against a query string, returning candidates
    /// suitable for the minibuffer completion list.
    pub fn fuzzy_filter(&self, query: &str) -> Vec<Candidate> {
        if query.is_empty() {
            return self
                .commands
                .iter()
                .take(MAX_RESULTS)
                .map(|c| command_to_candidate(c))
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &Command)> = self
            .commands
            .iter()
            .filter_map(|c| {
                // Match against name, description, id, and aliases
                let scores = [
                    matcher.fuzzy_match(c.name, query),
                    matcher.fuzzy_match(c.description, query),
                    matcher.fuzzy_match(c.id, query),
                ];
                let alias_score = c
                    .aliases
                    .iter()
                    .filter_map(|a| matcher.fuzzy_match(a, query))
                    .max();

                let best = scores.into_iter().flatten().chain(alias_score).max();
                best.map(|score| (score, c))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, c)| command_to_candidate(c))
            .collect()
    }

    /// Register all built-in commands.
    fn register_builtins(&mut self) {
        self.register(Command {
            id: "write",
            name: "Save",
            description: "Save current note to disk",
            aliases: &["w", "save"],
            binding: Some(":w"),
        });
        self.register(Command {
            id: "quit",
            name: "Quit",
            description: "Quit memex",
            aliases: &["q", "exit"],
            binding: Some(":q"),
        });
        self.register(Command {
            id: "wq",
            name: "Save and Quit",
            description: "Save current note and quit",
            aliases: &["x"],
            binding: Some(":wq"),
        });
        self.register(Command {
            id: "vault",
            name: "Switch Vault",
            description: "Open or switch to a vault",
            aliases: &["vaults", "open-vault", "switch-vault"],
            binding: Some(":vault"),
        });
        self.register(Command {
            id: "notes",
            name: "Find Note",
            description: "Search and open a note in current vault",
            aliases: &["find-note", "find", "note"],
            binding: Some("Ctrl+P"),
        });
        self.register(Command {
            id: "edit",
            name: "Edit File",
            description: "Open a file by path",
            aliases: &["e", "open"],
            binding: Some(":e <path>"),
        });
        self.register(Command {
            id: "set",
            name: "Set Option",
            description: "Set an editor option",
            aliases: &[],
            binding: Some(":set <option>"),
        });
        self.register(Command {
            id: "set-vim",
            name: "Enable Vim Mode",
            description: "Enable vim keybindings",
            aliases: &[],
            binding: None,
        });
        self.register(Command {
            id: "set-novim",
            name: "Disable Vim Mode",
            description: "Disable vim keybindings",
            aliases: &[],
            binding: None,
        });
        self.register(Command {
            id: "nohlsearch",
            name: "Clear Search Highlighting",
            description: "Remove search result highlighting",
            aliases: &["noh"],
            binding: Some(":noh"),
        });
        self.register(Command {
            id: "toggle-vim",
            name: "Toggle Vim Mode",
            description: "Toggle vim mode on/off",
            aliases: &[],
            binding: None,
        });
        // Outline commands
        self.register(Command {
            id: "outline-cycle-fold",
            name: "Outline: Toggle Fold",
            description: "Cycle fold state on current heading",
            aliases: &["fold", "toggle-fold"],
            binding: Some("Tab"),
        });
        self.register(Command {
            id: "outline-global-cycle",
            name: "Outline: Global Cycle",
            description: "Cycle all headings: overview → children → show all",
            aliases: &["fold-all", "unfold-all"],
            binding: Some("S-Tab"),
        });
        self.register(Command {
            id: "outline-promote",
            name: "Outline: Promote Heading",
            description: "Decrease heading level (## → #)",
            aliases: &["promote"],
            binding: Some("M-left"),
        });
        self.register(Command {
            id: "outline-demote",
            name: "Outline: Demote Heading",
            description: "Increase heading level (# → ##)",
            aliases: &["demote"],
            binding: Some("M-right"),
        });
        self.register(Command {
            id: "outline-move-up",
            name: "Outline: Move Subtree Up",
            description: "Swap heading subtree with previous sibling",
            aliases: &[],
            binding: Some("M-up"),
        });
        self.register(Command {
            id: "outline-move-down",
            name: "Outline: Move Subtree Down",
            description: "Swap heading subtree with next sibling",
            aliases: &[],
            binding: Some("M-down"),
        });
        self.register(Command {
            id: "outline-next-heading",
            name: "Outline: Next Heading",
            description: "Jump to next heading",
            aliases: &[],
            binding: Some("M-n"),
        });
        self.register(Command {
            id: "outline-prev-heading",
            name: "Outline: Previous Heading",
            description: "Jump to previous heading",
            aliases: &[],
            binding: Some("M-p"),
        });
    }
}

fn command_to_candidate(cmd: &Command) -> Candidate {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_builtins() {
        let reg = CommandRegistry::new();
        assert!(reg.all().len() >= 10);
    }

    #[test]
    fn test_lookup_by_id() {
        let reg = CommandRegistry::new();
        let cmd = reg.lookup("write").unwrap();
        assert_eq!(cmd.name, "Save");
    }

    #[test]
    fn test_lookup_by_alias() {
        let reg = CommandRegistry::new();
        let cmd = reg.lookup("w").unwrap();
        assert_eq!(cmd.id, "write");

        let cmd = reg.lookup("q").unwrap();
        assert_eq!(cmd.id, "quit");
    }

    #[test]
    fn test_lookup_missing() {
        let reg = CommandRegistry::new();
        assert!(reg.lookup("nonexistent").is_none());
    }

    #[test]
    fn test_fuzzy_filter_empty_query() {
        let reg = CommandRegistry::new();
        let results = reg.fuzzy_filter("");
        assert!(!results.is_empty());
        // Should return all commands (up to MAX_RESULTS)
        assert!(results.len() <= MAX_RESULTS);
    }

    #[test]
    fn test_fuzzy_filter_matches() {
        let reg = CommandRegistry::new();

        let results = reg.fuzzy_filter("save");
        assert!(!results.is_empty());
        assert_eq!(results[0].data, "write"); // "Save" matches

        let results = reg.fuzzy_filter("quit");
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.data == "quit"));
    }

    #[test]
    fn test_fuzzy_filter_alias() {
        let reg = CommandRegistry::new();
        let results = reg.fuzzy_filter("wq");
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.data == "wq"));
    }

    #[test]
    fn test_register_custom_command() {
        let mut reg = CommandRegistry::new();
        let initial_count = reg.all().len();

        reg.register(Command {
            id: "my-plugin-cmd",
            name: "My Plugin",
            description: "Does something cool",
            aliases: &[],
            binding: None,
        });

        assert_eq!(reg.all().len(), initial_count + 1);
        assert!(reg.lookup("my-plugin-cmd").is_some());
    }
}
