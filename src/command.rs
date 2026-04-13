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

