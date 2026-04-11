use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rhai::{Engine, Scope, AST};

use crate::editor::commands::EditorCommand;
use crate::editor::keymap::EditorMode;

/// Events that plugins can hook into.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hook {
    OnOpen,
    OnSave,
    AfterSave,
    OnChange,
    OnModeChange,
    OnStartup,
}

impl Hook {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Hook::OnOpen),
            "save" => Some(Hook::OnSave),
            "after_save" => Some(Hook::AfterSave),
            "change" => Some(Hook::OnChange),
            "mode_change" => Some(Hook::OnModeChange),
            "startup" => Some(Hook::OnStartup),
            _ => None,
        }
    }
}

/// A registered keybinding from a plugin.
#[derive(Clone, Debug)]
pub struct PluginBinding {
    pub mode: EditorMode,
    pub key: String,
    pub command_name: String,
}

/// Commands queued by Rhai scripts during execution.
/// Processed after script finishes.
type CommandQueue = Rc<RefCell<Vec<EditorCommand>>>;

/// Shared state accessible from Rhai functions.
struct SharedState {
    command_queue: CommandQueue,
    content: Rc<RefCell<String>>,
    cursor: Rc<RefCell<usize>>,
    selection: Rc<RefCell<(usize, usize)>>,
    hooks: Rc<RefCell<HashMap<String, Vec<AST>>>>,
    bindings: Rc<RefCell<Vec<PluginBinding>>>,
    custom_commands: Rc<RefCell<HashMap<String, AST>>>,
}

/// The plugin engine manages Rhai script execution and hook dispatch.
pub struct PluginEngine {
    engine: Engine,
    hooks: HashMap<Hook, Vec<AST>>,
    bindings: Vec<PluginBinding>,
    custom_commands: HashMap<String, AST>,
    command_queue: CommandQueue,
    /// Snapshot of editor content for Rhai to read
    content_snapshot: Rc<RefCell<String>>,
    cursor_snapshot: Rc<RefCell<usize>>,
    selection_snapshot: Rc<RefCell<(usize, usize)>>,
    /// Pending hooks/bindings from loading (before finalization)
    pending_hooks: Rc<RefCell<HashMap<String, Vec<AST>>>>,
    pending_bindings: Rc<RefCell<Vec<PluginBinding>>>,
    pending_commands: Rc<RefCell<HashMap<String, AST>>>,
}

impl PluginEngine {
    pub fn new() -> Self {
        let command_queue: CommandQueue = Rc::new(RefCell::new(Vec::new()));
        let content_snapshot = Rc::new(RefCell::new(String::new()));
        let cursor_snapshot = Rc::new(RefCell::new(0usize));
        let selection_snapshot = Rc::new(RefCell::new((0usize, 0usize)));
        let pending_hooks = Rc::new(RefCell::new(HashMap::new()));
        let pending_bindings = Rc::new(RefCell::new(Vec::new()));
        let pending_commands = Rc::new(RefCell::new(HashMap::new()));

        let mut engine = Engine::new();
        engine.set_max_operations(500_000);
        engine.set_max_string_size(100_000);

        // Register editor API functions
        Self::register_editor_api(
            &mut engine,
            command_queue.clone(),
            content_snapshot.clone(),
            cursor_snapshot.clone(),
            selection_snapshot.clone(),
        );

        // Register plugin setup functions (bind, on, register_command)
        Self::register_plugin_api(
            &mut engine,
            pending_hooks.clone(),
            pending_bindings.clone(),
            pending_commands.clone(),
        );

        Self {
            engine,
            hooks: HashMap::new(),
            bindings: Vec::new(),
            custom_commands: HashMap::new(),
            command_queue,
            content_snapshot,
            cursor_snapshot,
            selection_snapshot,
            pending_hooks,
            pending_bindings,
            pending_commands,
        }
    }

    fn register_editor_api(
        engine: &mut Engine,
        queue: CommandQueue,
        content: Rc<RefCell<String>>,
        cursor: Rc<RefCell<usize>>,
        selection: Rc<RefCell<(usize, usize)>>,
    ) {
        // insert_text(s) — queue an InsertText command
        let q = queue.clone();
        engine.register_fn("insert_text", move |text: String| {
            q.borrow_mut().push(EditorCommand::InsertText(text));
        });

        // delete_selection() — queue DeleteSelection
        let q = queue.clone();
        engine.register_fn("delete_selection", move || {
            q.borrow_mut().push(EditorCommand::DeleteSelection);
        });

        // get_content() — return current content snapshot
        let c = content.clone();
        engine.register_fn("get_content", move || -> String {
            c.borrow().clone()
        });

        // get_cursor() — return cursor position
        let cur = cursor.clone();
        engine.register_fn("get_cursor", move || -> i64 {
            *cur.borrow() as i64
        });

        // get_selection() — return (start, end) as a map
        let sel = selection.clone();
        engine.register_fn("get_selection_start", move || -> i64 {
            sel.borrow().0 as i64
        });
        let sel = selection.clone();
        engine.register_fn("get_selection_end", move || -> i64 {
            sel.borrow().1 as i64
        });

        // move_cursor_to(offset)
        let q = queue.clone();
        engine.register_fn("move_cursor_to", move |offset: i64| {
            q.borrow_mut()
                .push(EditorCommand::MoveToOffset(offset.max(0) as usize));
        });

        // select_range(start, end)
        let q = queue.clone();
        engine.register_fn("select_range", move |start: i64, end: i64| {
            let s = start.max(0) as usize;
            let e = end.max(0) as usize;
            q.borrow_mut()
                .push(EditorCommand::SelectToOffset(s));
            q.borrow_mut()
                .push(EditorCommand::MoveToOffset(s));
            q.borrow_mut()
                .push(EditorCommand::SelectToOffset(e));
        });

        // Movement commands
        let q = queue.clone();
        engine.register_fn("move_left", move || {
            q.borrow_mut().push(EditorCommand::MoveLeft);
        });
        let q = queue.clone();
        engine.register_fn("move_right", move || {
            q.borrow_mut().push(EditorCommand::MoveRight);
        });
        let q = queue.clone();
        engine.register_fn("move_up", move || {
            q.borrow_mut().push(EditorCommand::MoveUp);
        });
        let q = queue.clone();
        engine.register_fn("move_down", move || {
            q.borrow_mut().push(EditorCommand::MoveDown);
        });

        // Undo/Redo
        let q = queue.clone();
        engine.register_fn("undo", move || {
            q.borrow_mut().push(EditorCommand::Undo);
        });
        let q = queue.clone();
        engine.register_fn("redo", move || {
            q.borrow_mut().push(EditorCommand::Redo);
        });

        // Enter mode
        let q = queue.clone();
        engine.register_fn("enter_insert_mode", move || {
            q.borrow_mut()
                .push(EditorCommand::EnterMode(EditorMode::Insert));
        });
        let q = queue.clone();
        engine.register_fn("enter_normal_mode", move || {
            q.borrow_mut()
                .push(EditorCommand::EnterMode(EditorMode::Normal));
        });

        // Utility functions
        engine.register_fn("date_today", || -> String {
            chrono_free_today()
        });

        engine.register_fn("time_now", || -> String {
            chrono_free_now()
        });
    }

    fn register_plugin_api(
        engine: &mut Engine,
        hooks: Rc<RefCell<HashMap<String, Vec<AST>>>>,
        bindings: Rc<RefCell<Vec<PluginBinding>>>,
        commands: Rc<RefCell<HashMap<String, AST>>>,
    ) {
        // bind(mode, key, command_name)
        let b = bindings.clone();
        engine.register_fn("bind", move |mode: String, key: String, command: String| {
            let editor_mode = match mode.as_str() {
                "insert" => EditorMode::Insert,
                "normal" => EditorMode::Normal,
                "visual" => EditorMode::Visual,
                _ => EditorMode::Insert,
            };
            b.borrow_mut().push(PluginBinding {
                mode: editor_mode,
                key,
                command_name: command,
            });
        });

        // unbind(mode, key) — not yet functional, placeholder
        engine.register_fn("unbind", |_mode: String, _key: String| {
            // TODO: implement unbinding
        });

        // set(key, value) — configuration setter (placeholder)
        engine.register_fn("set", |_key: String, _value: bool| {
            // TODO: connect to config system
        });

        // get(key) — configuration getter (placeholder)
        engine.register_fn("get", |_key: String| -> bool {
            false
        });
    }

    /// Load all plugin files from a directory.
    pub fn load_plugins_from(&mut self, dir: &Path) {
        if !dir.exists() || !dir.is_dir() {
            return;
        }

        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "rhai"))
            .collect();
        entries.sort();

        for path in entries {
            if let Err(e) = self.load_plugin(&path) {
                eprintln!("plugin error ({}): {}", path.display(), e);
            }
        }
    }

    /// Load and execute a single plugin file.
    fn load_plugin(&mut self, path: &Path) -> Result<(), String> {
        let script = std::fs::read_to_string(path)
            .map_err(|e| format!("read error: {}", e))?;

        let ast = self
            .engine
            .compile(&script)
            .map_err(|e| format!("compile error: {}", e))?;

        let mut scope = Scope::new();
        self.engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| format!("runtime error: {}", e))?;

        // Collect any bindings registered during execution
        let new_bindings: Vec<PluginBinding> =
            self.pending_bindings.borrow_mut().drain(..).collect();
        self.bindings.extend(new_bindings);

        // Collect hooks - for now hooks need to be registered as named functions
        // in the script that we detect and store
        // TODO: support `on("save", || { ... })` syntax with closures

        Ok(())
    }

    /// Load plugins from global and vault directories.
    pub fn load_all_plugins(&mut self, vault_path: Option<&Path>) {
        // Global plugins: ~/.config/memex/plugins/
        if let Some(config_dir) = dirs::config_dir() {
            let global_plugins = config_dir.join("memex").join("plugins");
            self.load_plugins_from(&global_plugins);
        }

        // Vault plugins: {vault}/.memex/plugins/
        if let Some(vault) = vault_path {
            let vault_plugins = vault.join(".memex").join("plugins");
            self.load_plugins_from(&vault_plugins);
        }
    }

    /// Update snapshots from editor state before running hooks.
    pub fn update_snapshots(&self, content: &str, cursor: usize, selection: (usize, usize)) {
        *self.content_snapshot.borrow_mut() = content.to_string();
        *self.cursor_snapshot.borrow_mut() = cursor;
        *self.selection_snapshot.borrow_mut() = selection;
    }

    /// Fire a hook event and return queued commands.
    pub fn fire_hook(
        &mut self,
        hook: &Hook,
        content: &str,
        cursor: usize,
        selection: (usize, usize),
    ) -> Vec<EditorCommand> {
        self.update_snapshots(content, cursor, selection);
        self.command_queue.borrow_mut().clear();

        if let Some(asts) = self.hooks.get(hook) {
            for ast in asts.clone() {
                let mut scope = Scope::new();
                if let Err(e) = self.engine.run_ast_with_scope(&mut scope, &ast) {
                    eprintln!("hook error ({:?}): {}", hook, e);
                }
            }
        }

        self.command_queue.borrow_mut().drain(..).collect()
    }

    /// Run a custom command by name. Returns queued editor commands.
    pub fn run_command(
        &mut self,
        name: &str,
        content: &str,
        cursor: usize,
        selection: (usize, usize),
    ) -> Option<Vec<EditorCommand>> {
        let ast = self.custom_commands.get(name)?.clone();
        self.update_snapshots(content, cursor, selection);
        self.command_queue.borrow_mut().clear();

        let mut scope = Scope::new();
        if let Err(e) = self.engine.run_ast_with_scope(&mut scope, &ast) {
            eprintln!("command error ({}): {}", name, e);
        }

        Some(self.command_queue.borrow_mut().drain(..).collect())
    }

    /// Get all plugin-registered keybindings.
    pub fn bindings(&self) -> &[PluginBinding] {
        &self.bindings
    }

    /// Drain pending commands (from direct API calls in scripts).
    pub fn drain_commands(&self) -> Vec<EditorCommand> {
        self.command_queue.borrow_mut().drain(..).collect()
    }

    /// Register a hook with an AST to run.
    pub fn register_hook(&mut self, hook: Hook, ast: AST) {
        self.hooks.entry(hook).or_default().push(ast);
    }
}

/// Simple date without chrono dependency — uses system time.
fn chrono_free_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple date calculation
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn chrono_free_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn days_to_ymd(days_since_epoch: u64) -> (u64, u64, u64) {
    // Simplified Gregorian calendar calculation
    let mut y = 1970;
    let mut remaining = days_since_epoch as i64;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for md in &month_days {
        if remaining < *md {
            break;
        }
        remaining -= md;
        m += 1;
    }

    (y, m + 1, remaining as u64 + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_engine_creation() {
        let engine = PluginEngine::new();
        assert!(engine.bindings().is_empty());
    }

    #[test]
    fn test_rhai_insert_text() {
        let mut engine = PluginEngine::new();
        let ast = engine
            .engine
            .compile(r#"insert_text("hello world");"#)
            .unwrap();

        let mut scope = Scope::new();
        engine
            .engine
            .run_ast_with_scope(&mut scope, &ast)
            .unwrap();

        let cmds = engine.drain_commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            EditorCommand::InsertText(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected InsertText"),
        }
    }

    #[test]
    fn test_rhai_get_content() {
        let engine = PluginEngine::new();
        engine.update_snapshots("test content", 5, (0, 0));

        let result: String = engine
            .engine
            .eval(r#"get_content()"#)
            .unwrap();
        assert_eq!(result, "test content");
    }

    #[test]
    fn test_rhai_bind() {
        let mut engine = PluginEngine::new();
        let ast = engine
            .engine
            .compile(r#"bind("normal", "ctrl-d", "daily_note");"#)
            .unwrap();

        let mut scope = Scope::new();
        engine
            .engine
            .run_ast_with_scope(&mut scope, &ast)
            .unwrap();

        let bindings: Vec<PluginBinding> =
            engine.pending_bindings.borrow_mut().drain(..).collect();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].key, "ctrl-d");
        assert_eq!(bindings[0].command_name, "daily_note");
    }

    #[test]
    fn test_date_today() {
        let date = chrono_free_today();
        // Should be in YYYY-MM-DD format
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let mut engine = PluginEngine::new();
        engine.load_plugins_from(Path::new("/nonexistent/path"));
        // Should not panic
    }
}
