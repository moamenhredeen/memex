use std::collections::HashMap;

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::command::Command;
use crate::minibuffer::Candidate;

const MAX_RESULTS: usize = 15;

// ─── MajorMode ───────────────────────────────────────────────────────────────

/// A major mode bundles the three pillars of a view context:
///   1. **Keymap layers** — which layers activate/deactivate when entering this mode
///   2. **Commands** — operations discoverable in the command palette
///   3. **Identity** — id + display name for mode-line
///
/// Following Emacs conventions: a major mode is a coherent package, not
/// scattered `if view_mode == X` checks. Switching modes is a single
/// operation that swaps keymaps and commands together.
///
/// Vim sub-modes (normal/insert/visual) are NOT major modes — they are
/// layer activations within the editor major mode, conceptually like
/// Emacs minor modes.
pub struct MajorMode {
    /// Unique identifier (e.g., "editor", "pdf").
    pub id: &'static str,
    /// Display name for the mode-line (e.g., "Markdown", "PDF").
    pub name: &'static str,
    /// Keymap layer IDs to activate when entering this mode.
    pub layers: Vec<&'static str>,
    /// Keymap layer IDs to deactivate when entering this mode.
    pub deactivate_layers: Vec<&'static str>,
    /// Commands specific to this mode (shown in palette only when active).
    pub commands: Vec<Command>,
}

// ─── ModeRegistry ────────────────────────────────────────────────────────────

/// Registry of all major modes and global commands.
///
/// Follows the Emacs model:
/// - **Global commands** (like `global-map` commands) are always available
/// - **Mode commands** are active only when that mode is current
/// - Palette lookup: mode commands first, then global (mode can shadow global)
pub struct ModeRegistry {
    /// Commands available in every mode (save, quit, vault-switch, etc.).
    pub global_commands: Vec<Command>,
    /// Registered major modes, keyed by mode id.
    pub modes: HashMap<&'static str, MajorMode>,
    /// Currently active mode id.
    pub active_mode: &'static str,
}

impl ModeRegistry {
    /// Create a new registry with all built-in modes and global commands.
    pub fn new() -> Self {
        let mut registry = Self {
            global_commands: Vec::new(),
            modes: HashMap::new(),
            active_mode: "editor",
        };
        registry.register_builtins();
        registry
    }

    /// Get the currently active mode.
    pub fn active(&self) -> Option<&MajorMode> {
        self.modes.get(self.active_mode)
    }

    /// Get a mode by id.
    pub fn get(&self, mode_id: &str) -> Option<&MajorMode> {
        self.modes.get(mode_id)
    }

    /// Switch to a new mode. Returns the layers to activate and deactivate.
    ///
    /// The caller is responsible for actually calling `stack.activate_layer()`
    /// and `stack.deactivate_layer()` on the keymap system — the mode registry
    /// is pure data and doesn't own the layer stack.
    pub fn switch_mode(&mut self, new_mode: &'static str) -> ModeSwitchPlan {
        let mut plan = ModeSwitchPlan {
            deactivate: Vec::new(),
            activate: Vec::new(),
        };

        // Deactivate old mode's layers
        if let Some(old_mode) = self.modes.get(self.active_mode) {
            for layer in &old_mode.layers {
                plan.deactivate.push(layer);
            }
        }

        // Apply new mode's plan
        if let Some(new_mode_def) = self.modes.get(new_mode) {
            for layer in &new_mode_def.deactivate_layers {
                plan.deactivate.push(layer);
            }
            for layer in &new_mode_def.layers {
                plan.activate.push(layer);
            }
        }

        self.active_mode = new_mode;
        plan
    }

    /// Look up a command by id or alias, searching mode commands first, then global.
    pub fn lookup(&self, name: &str) -> Option<&Command> {
        // Mode-local commands shadow global (like Emacs mode-map shadows global-map)
        if let Some(mode) = self.modes.get(self.active_mode) {
            if let Some(cmd) = mode.commands.iter().find(|c| {
                c.id == name || c.aliases.contains(&name)
            }) {
                return Some(cmd);
            }
        }
        self.global_commands.iter().find(|c| {
            c.id == name || c.aliases.contains(&name)
        })
    }

    /// Fuzzy-filter commands for the command palette, scoped to the active mode.
    ///
    /// Returns mode-specific commands + global commands, ranked by match score.
    /// Mode commands appear before global commands at equal scores.
    pub fn fuzzy_filter(&self, query: &str) -> Vec<Candidate> {
        let mode_cmds = self.modes.get(self.active_mode)
            .map(|m| m.commands.as_slice())
            .unwrap_or(&[]);
        let global_cmds = &self.global_commands;

        if query.is_empty() {
            return mode_cmds.iter().chain(global_cmds.iter())
                .take(MAX_RESULTS)
                .map(|c| command_to_candidate(c))
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &Command)> = mode_cmds.iter()
            .chain(global_cmds.iter())
            .filter_map(|c| {
                let scores = [
                    matcher.fuzzy_match(c.name, query),
                    matcher.fuzzy_match(c.description, query),
                    matcher.fuzzy_match(c.id, query),
                ];
                let alias_score = c.aliases.iter()
                    .filter_map(|a| matcher.fuzzy_match(a, query))
                    .max();
                let best = scores.into_iter().flatten().chain(alias_score).max();
                best.map(|score| (score, c))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter()
            .take(MAX_RESULTS)
            .map(|(_, c)| command_to_candidate(c))
            .collect()
    }

    /// Register all built-in modes and global commands.
    fn register_builtins(&mut self) {
        self.register_global_commands();
        self.register_editor_mode();
        self.register_pdf_mode();
    }

    fn register_global_commands(&mut self) {
        self.global_commands = vec![
            Command {
                id: "write",
                name: "Save",
                description: "Save current note to disk",
                aliases: &["w", "save"],
                binding: Some(":w"),
            },
            Command {
                id: "quit",
                name: "Quit",
                description: "Quit memex",
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
        ];
    }

    fn register_editor_mode(&mut self) {
        let mode = MajorMode {
            id: "editor",
            name: "Markdown",
            layers: vec![
                "vim:normal",
                "vim:motion",
                "leader",
                "markdown",
            ],
            deactivate_layers: vec![
                "pdf",
            ],
            commands: vec![
                Command {
                    id: "outline-cycle-fold",
                    name: "Outline: Toggle Fold",
                    description: "Cycle fold state on current heading",
                    aliases: &["fold", "toggle-fold"],
                    binding: Some("Tab"),
                },
                Command {
                    id: "outline-global-cycle",
                    name: "Outline: Global Cycle",
                    description: "Cycle all headings: overview → children → show all",
                    aliases: &["fold-all", "unfold-all"],
                    binding: Some("S-Tab"),
                },
                Command {
                    id: "outline-promote",
                    name: "Outline: Promote Heading",
                    description: "Decrease heading level (## → #)",
                    aliases: &["promote"],
                    binding: Some("M-left"),
                },
                Command {
                    id: "outline-demote",
                    name: "Outline: Demote Heading",
                    description: "Increase heading level (# → ##)",
                    aliases: &["demote"],
                    binding: Some("M-right"),
                },
                Command {
                    id: "outline-move-up",
                    name: "Outline: Move Subtree Up",
                    description: "Swap heading subtree with previous sibling",
                    aliases: &[],
                    binding: Some("M-up"),
                },
                Command {
                    id: "outline-move-down",
                    name: "Outline: Move Subtree Down",
                    description: "Swap heading subtree with next sibling",
                    aliases: &[],
                    binding: Some("M-down"),
                },
                Command {
                    id: "outline-next-heading",
                    name: "Outline: Next Heading",
                    description: "Jump to next heading",
                    aliases: &[],
                    binding: Some("M-n"),
                },
                Command {
                    id: "outline-prev-heading",
                    name: "Outline: Previous Heading",
                    description: "Jump to previous heading",
                    aliases: &[],
                    binding: Some("M-p"),
                },
            ],
        };
        self.modes.insert("editor", mode);
    }

    fn register_pdf_mode(&mut self) {
        let mode = MajorMode {
            id: "pdf",
            name: "PDF",
            layers: vec![
                "pdf",
            ],
            deactivate_layers: vec![
                "vim:normal",
                "vim:motion",
                "vim:insert",
                "vim:visual",
                "vim:operator-pending",
                "leader",
                "markdown",
            ],
            commands: vec![
                Command {
                    id: "pdf-toc",
                    name: "PDF: Table of Contents",
                    description: "Browse and jump to table of contents entries",
                    aliases: &["toc", "outline", "contents"],
                    binding: Some("o"),
                },
                Command {
                    id: "pdf-goto-page",
                    name: "PDF: Go to Page",
                    description: "Jump to a specific page number",
                    aliases: &["goto-page", "page"],
                    binding: Some("P"),
                },
                Command {
                    id: "pdf-bookmarks",
                    name: "PDF: Bookmarks",
                    description: "Browse PDF bookmarks (outline entries)",
                    aliases: &["bookmarks"],
                    binding: None,
                },
                Command {
                    id: "pdf-fit-width",
                    name: "PDF: Fit Width",
                    description: "Zoom to fit page width to viewport",
                    aliases: &["fit-width"],
                    binding: Some("w"),
                },
                Command {
                    id: "pdf-fit-page",
                    name: "PDF: Fit Page",
                    description: "Zoom to fit entire page in viewport",
                    aliases: &["fit-page"],
                    binding: Some("W"),
                },
                Command {
                    id: "pdf-rotate-cw",
                    name: "PDF: Rotate Clockwise",
                    description: "Rotate current page 90° clockwise",
                    aliases: &["rotate-cw", "rotate"],
                    binding: Some("r"),
                },
                Command {
                    id: "pdf-rotate-ccw",
                    name: "PDF: Rotate Counter-Clockwise",
                    description: "Rotate current page 90° counter-clockwise",
                    aliases: &["rotate-ccw"],
                    binding: Some("R"),
                },
                Command {
                    id: "pdf-dark-mode",
                    name: "PDF: Toggle Dark Mode",
                    description: "Invert colors for night reading",
                    aliases: &["dark-mode", "invert"],
                    binding: None,
                },
                Command {
                    id: "pdf-two-page",
                    name: "PDF: Two-Page Spread",
                    description: "Toggle side-by-side two-page view",
                    aliases: &["spread", "two-page"],
                    binding: None,
                },
                Command {
                    id: "pdf-copy-link",
                    name: "PDF: Copy Page Link",
                    description: "Copy [[file.pdf#page=N]] link to clipboard",
                    aliases: &["copy-link", "yank-link"],
                    binding: Some("y"),
                },
                Command {
                    id: "pdf-extract-text",
                    name: "PDF: Extract Page Text",
                    description: "Copy text from current page to clipboard",
                    aliases: &["extract-text"],
                    binding: Some("Y"),
                },
                Command {
                    id: "pdf-scroll-down",
                    name: "PDF: Scroll Down",
                    description: "Scroll PDF down one step",
                    aliases: &[],
                    binding: Some("j"),
                },
                Command {
                    id: "pdf-scroll-up",
                    name: "PDF: Scroll Up",
                    description: "Scroll PDF up one step",
                    aliases: &[],
                    binding: Some("k"),
                },
                Command {
                    id: "pdf-half-page-down",
                    name: "PDF: Half Page Down",
                    description: "Scroll PDF down half a page",
                    aliases: &[],
                    binding: Some("Ctrl-d"),
                },
                Command {
                    id: "pdf-half-page-up",
                    name: "PDF: Half Page Up",
                    description: "Scroll PDF up half a page",
                    aliases: &[],
                    binding: Some("Ctrl-u"),
                },
                Command {
                    id: "pdf-zoom-in",
                    name: "PDF: Zoom In",
                    description: "Increase PDF zoom level",
                    aliases: &[],
                    binding: Some("+"),
                },
                Command {
                    id: "pdf-zoom-out",
                    name: "PDF: Zoom Out",
                    description: "Decrease PDF zoom level",
                    aliases: &[],
                    binding: Some("-"),
                },
                Command {
                    id: "pdf-goto-first",
                    name: "PDF: Go to First Page",
                    description: "Jump to the first page",
                    aliases: &[],
                    binding: Some("g"),
                },
                Command {
                    id: "pdf-goto-last",
                    name: "PDF: Go to Last Page",
                    description: "Jump to the last page",
                    aliases: &[],
                    binding: Some("G"),
                },
                Command {
                    id: "pdf-search",
                    name: "PDF: Search Text",
                    description: "Search for text across all pages",
                    aliases: &["search", "find"],
                    binding: Some("/"),
                },
                Command {
                    id: "pdf-search-next",
                    name: "PDF: Next Match",
                    description: "Jump to the next search match",
                    aliases: &["next-match"],
                    binding: Some("n"),
                },
                Command {
                    id: "pdf-search-prev",
                    name: "PDF: Previous Match",
                    description: "Jump to the previous search match",
                    aliases: &["prev-match"],
                    binding: Some("N"),
                },
            ],
        };
        self.modes.insert("pdf", mode);
    }
}

/// Plan for switching between modes — lists layers to activate and deactivate.
pub struct ModeSwitchPlan {
    pub deactivate: Vec<&'static str>,
    pub activate: Vec<&'static str>,
}

impl ModeSwitchPlan {
    /// Apply this plan to a keymap layer stack.
    pub fn apply(&self, stack: &mut crate::keymap::LayerStack) {
        for layer in &self.deactivate {
            stack.deactivate_layer(layer);
        }
        for layer in &self.activate {
            stack.activate_layer(layer);
        }
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_global_commands() {
        let reg = ModeRegistry::new();
        assert!(reg.global_commands.len() >= 10);
    }

    #[test]
    fn test_registry_has_editor_mode() {
        let reg = ModeRegistry::new();
        let mode = reg.get("editor").unwrap();
        assert_eq!(mode.name, "Markdown");
        assert!(!mode.commands.is_empty());
        assert!(mode.commands.iter().any(|c| c.id == "outline-cycle-fold"));
    }

    #[test]
    fn test_registry_has_pdf_mode() {
        let reg = ModeRegistry::new();
        let mode = reg.get("pdf").unwrap();
        assert_eq!(mode.name, "PDF");
        assert!(mode.commands.iter().any(|c| c.id == "pdf-toc"));
        assert!(mode.commands.iter().any(|c| c.id == "pdf-scroll-down"));
    }

    #[test]
    fn test_default_active_mode_is_editor() {
        let reg = ModeRegistry::new();
        assert_eq!(reg.active_mode, "editor");
        let active = reg.active().unwrap();
        assert_eq!(active.id, "editor");
    }

    #[test]
    fn test_lookup_global_command() {
        let reg = ModeRegistry::new();
        let cmd = reg.lookup("write").unwrap();
        assert_eq!(cmd.name, "Save");
    }

    #[test]
    fn test_lookup_by_alias() {
        let reg = ModeRegistry::new();
        let cmd = reg.lookup("w").unwrap();
        assert_eq!(cmd.id, "write");

        let cmd = reg.lookup("q").unwrap();
        assert_eq!(cmd.id, "quit");
    }

    #[test]
    fn test_lookup_mode_command_in_editor() {
        let reg = ModeRegistry::new();
        // Editor mode is active by default
        let cmd = reg.lookup("outline-cycle-fold").unwrap();
        assert_eq!(cmd.name, "Outline: Toggle Fold");
    }

    #[test]
    fn test_lookup_mode_command_not_in_wrong_mode() {
        let reg = ModeRegistry::new();
        // Editor mode active — PDF commands should not be found via mode lookup
        // but they ARE in the PDF mode's commands, not in global
        // Since lookup checks active mode then global, pdf-toc won't be found
        // when in editor mode (it's not global and not in editor's commands)
        assert!(reg.lookup("pdf-toc").is_none());
    }

    #[test]
    fn test_lookup_after_mode_switch() {
        let mut reg = ModeRegistry::new();
        reg.switch_mode("pdf");
        // Now PDF commands should be found
        let cmd = reg.lookup("pdf-toc").unwrap();
        assert_eq!(cmd.name, "PDF: Table of Contents");
        // Editor commands should NOT be found
        assert!(reg.lookup("outline-cycle-fold").is_none());
        // Global commands still available
        assert!(reg.lookup("write").is_some());
    }

    #[test]
    fn test_switch_mode_plan() {
        let mut reg = ModeRegistry::new();
        let plan = reg.switch_mode("pdf");

        // Should deactivate editor layers
        assert!(plan.deactivate.contains(&"vim:normal"));
        assert!(plan.deactivate.contains(&"vim:motion"));
        // Should activate PDF layer
        assert!(plan.activate.contains(&"pdf"));
    }

    #[test]
    fn test_switch_back_to_editor() {
        let mut reg = ModeRegistry::new();
        reg.switch_mode("pdf");
        let plan = reg.switch_mode("editor");

        // Should deactivate PDF layer
        assert!(plan.deactivate.contains(&"pdf"));
        // Should activate editor layers
        assert!(plan.activate.contains(&"vim:normal"));
        assert!(plan.activate.contains(&"vim:motion"));
    }

    #[test]
    fn test_fuzzy_filter_in_editor_mode() {
        let reg = ModeRegistry::new();
        let results = reg.fuzzy_filter("");
        // Should contain editor commands and global commands
        assert!(results.iter().any(|c| c.data == "write"));
        assert!(results.iter().any(|c| c.data == "outline-cycle-fold"));
        // Should NOT contain PDF commands
        assert!(!results.iter().any(|c| c.data == "pdf-toc"));
    }

    #[test]
    fn test_fuzzy_filter_in_pdf_mode() {
        let mut reg = ModeRegistry::new();
        reg.switch_mode("pdf");
        // With a specific query, should find PDF commands
        let results = reg.fuzzy_filter("table of contents");
        assert!(results.iter().any(|c| c.data == "pdf-toc"));
        // Global commands should also be findable
        let results = reg.fuzzy_filter("save");
        assert!(results.iter().any(|c| c.data == "write"));
        // Editor commands should NOT be findable
        let results = reg.fuzzy_filter("outline fold");
        assert!(!results.iter().any(|c| c.data == "outline-cycle-fold"));
    }

    #[test]
    fn test_fuzzy_filter_matches() {
        let reg = ModeRegistry::new();
        let results = reg.fuzzy_filter("save");
        assert!(!results.is_empty());
        assert_eq!(results[0].data, "write");
    }

    #[test]
    fn test_fuzzy_filter_alias() {
        let reg = ModeRegistry::new();
        let results = reg.fuzzy_filter("wq");
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.data == "wq"));
    }

    #[test]
    fn test_lookup_missing() {
        let reg = ModeRegistry::new();
        assert!(reg.lookup("nonexistent").is_none());
    }
}
