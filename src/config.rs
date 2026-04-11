use std::path::{Path, PathBuf};

use rhai::{Engine, Scope};

/// All configurable values for memex.
#[derive(Debug, Clone)]
pub struct MemexConfig {
    // Font sizes
    pub h1_size: f32,
    pub h2_size: f32,
    pub h3_size: f32,
    pub body_size: f32,

    // Colors as (r, g, b) tuples
    pub text_color: (u8, u8, u8),
    pub heading_color: (u8, u8, u8),
    pub marker_color: (u8, u8, u8),
    pub bg_color: (u8, u8, u8),
    pub editor_bg: (u8, u8, u8),

    // Editor behavior
    pub auto_save: bool,
    pub auto_save_delay_secs: f64,
    pub tab_size: i64,
    pub line_height: f64,
}

impl Default for MemexConfig {
    fn default() -> Self {
        Self {
            h1_size: 32.0,
            h2_size: 24.0,
            h3_size: 20.0,
            body_size: 16.0,
            text_color: (40, 40, 40),
            heading_color: (25, 25, 25),
            marker_color: (160, 160, 170),
            bg_color: (248, 248, 248),
            editor_bg: (255, 255, 255),
            auto_save: false,
            auto_save_delay_secs: 3.0,
            tab_size: 4,
            line_height: 1.2,
        }
    }
}

impl PartialEq for MemexConfig {
    fn eq(&self, other: &Self) -> bool {
        self.h1_size == other.h1_size
            && self.h2_size == other.h2_size
            && self.h3_size == other.h3_size
            && self.body_size == other.body_size
            && self.text_color == other.text_color
            && self.heading_color == other.heading_color
            && self.marker_color == other.marker_color
            && self.bg_color == other.bg_color
            && self.editor_bg == other.editor_bg
            && self.auto_save == other.auto_save
            && self.tab_size == other.tab_size
    }
}

/// Build a Rhai engine with the memex configuration API registered.
fn build_engine() -> Engine {
    let mut engine = Engine::new();
    // Limit script execution for safety
    engine.set_max_operations(100_000);
    engine.set_max_string_size(10_000);
    engine
}

/// Load and execute a config script, mutating the config.
fn run_config_script(
    engine: &Engine,
    script_path: &Path,
    config: &mut MemexConfig,
) -> Result<(), String> {
    let script = std::fs::read_to_string(script_path)
        .map_err(|e| format!("failed to read {}: {}", script_path.display(), e))?;

    let ast = engine
        .compile(&script)
        .map_err(|e| format!("parse error in {}: {}", script_path.display(), e))?;

    let mut scope = Scope::new();

    // Push config values into scope so the script can modify them
    scope.push("h1_size", config.h1_size as f64);
    scope.push("h2_size", config.h2_size as f64);
    scope.push("h3_size", config.h3_size as f64);
    scope.push("body_size", config.body_size as f64);

    scope.push("text_color_r", config.text_color.0 as i64);
    scope.push("text_color_g", config.text_color.1 as i64);
    scope.push("text_color_b", config.text_color.2 as i64);

    scope.push("heading_color_r", config.heading_color.0 as i64);
    scope.push("heading_color_g", config.heading_color.1 as i64);
    scope.push("heading_color_b", config.heading_color.2 as i64);

    scope.push("marker_color_r", config.marker_color.0 as i64);
    scope.push("marker_color_g", config.marker_color.1 as i64);
    scope.push("marker_color_b", config.marker_color.2 as i64);

    scope.push("bg_color_r", config.bg_color.0 as i64);
    scope.push("bg_color_g", config.bg_color.1 as i64);
    scope.push("bg_color_b", config.bg_color.2 as i64);

    scope.push("editor_bg_r", config.editor_bg.0 as i64);
    scope.push("editor_bg_g", config.editor_bg.1 as i64);
    scope.push("editor_bg_b", config.editor_bg.2 as i64);

    scope.push("auto_save", config.auto_save);
    scope.push("auto_save_delay_secs", config.auto_save_delay_secs);
    scope.push("tab_size", config.tab_size);
    scope.push("line_height", config.line_height);

    engine
        .run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| format!("runtime error in {}: {}", script_path.display(), e))?;

    // Read back modified values
    if let Some(v) = scope.get_value::<f64>("h1_size") {
        config.h1_size = v as f32;
    }
    if let Some(v) = scope.get_value::<f64>("h2_size") {
        config.h2_size = v as f32;
    }
    if let Some(v) = scope.get_value::<f64>("h3_size") {
        config.h3_size = v as f32;
    }
    if let Some(v) = scope.get_value::<f64>("body_size") {
        config.body_size = v as f32;
    }

    // Read back colors
    fn read_color(scope: &Scope, prefix: &str) -> Option<(u8, u8, u8)> {
        let r = scope.get_value::<i64>(&format!("{}_r", prefix))? as u8;
        let g = scope.get_value::<i64>(&format!("{}_g", prefix))? as u8;
        let b = scope.get_value::<i64>(&format!("{}_b", prefix))? as u8;
        Some((r, g, b))
    }

    if let Some(c) = read_color(&scope, "text_color") {
        config.text_color = c;
    }
    if let Some(c) = read_color(&scope, "heading_color") {
        config.heading_color = c;
    }
    if let Some(c) = read_color(&scope, "marker_color") {
        config.marker_color = c;
    }
    if let Some(c) = read_color(&scope, "bg_color") {
        config.bg_color = c;
    }
    if let Some(c) = read_color(&scope, "editor_bg") {
        config.editor_bg = c;
    }

    if let Some(v) = scope.get_value::<bool>("auto_save") {
        config.auto_save = v;
    }
    if let Some(v) = scope.get_value::<f64>("auto_save_delay_secs") {
        config.auto_save_delay_secs = v;
    }
    if let Some(v) = scope.get_value::<i64>("tab_size") {
        config.tab_size = v;
    }
    if let Some(v) = scope.get_value::<f64>("line_height") {
        config.line_height = v;
    }

    Ok(())
}

/// Load configuration: global config first, then vault-specific overlay.
pub fn load_config(vault_path: Option<&Path>) -> MemexConfig {
    let mut config = MemexConfig::default();
    let engine = build_engine();

    // 1. Global config: ~/.config/memex/config.rhai
    let global_path = global_config_path();
    if global_path.exists() {
        if let Err(e) = run_config_script(&engine, &global_path, &mut config) {
            eprintln!("global config error: {}", e);
        }
    }

    // 2. Vault config: {vault}/.memex/config.rhai
    if let Some(vault) = vault_path {
        let vault_config = vault.join(".memex").join("config.rhai");
        if vault_config.exists() {
            if let Err(e) = run_config_script(&engine, &vault_config, &mut config) {
                eprintln!("vault config error: {}", e);
            }
        }
    }

    config
}

/// Reload configuration (for the "Reload configuration" command).
pub fn reload_config(vault_path: Option<&Path>) -> MemexConfig {
    load_config(vault_path)
}

fn global_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("memex")
        .join("config.rhai")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_default_config() {
        let config = MemexConfig::default();
        assert_eq!(config.h1_size, 32.0);
        assert_eq!(config.body_size, 16.0);
        assert_eq!(config.text_color, (40, 40, 40));
        assert!(!config.auto_save);
    }

    #[test]
    fn test_rhai_config_script() {
        let dir = std::env::temp_dir().join("memex-test-config");
        fs::create_dir_all(&dir).unwrap();
        let script_path = dir.join("config.rhai");

        fs::write(
            &script_path,
            r#"
            h1_size = 48.0;
            body_size = 18.0;
            auto_save = true;
            tab_size = 2;
            bg_color_r = 10;
            bg_color_g = 20;
            bg_color_b = 30;
            "#,
        )
        .unwrap();

        let mut config = MemexConfig::default();
        let engine = build_engine();
        run_config_script(&engine, &script_path, &mut config).unwrap();

        assert_eq!(config.h1_size, 48.0);
        assert_eq!(config.body_size, 18.0);
        assert!(config.auto_save);
        assert_eq!(config.tab_size, 2);
        assert_eq!(config.bg_color, (10, 20, 30));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_invalid_script_returns_error() {
        let dir = std::env::temp_dir().join("memex-test-config-err");
        fs::create_dir_all(&dir).unwrap();
        let script_path = dir.join("bad.rhai");
        fs::write(&script_path, "this is not valid rhai {{{}}}").unwrap();

        let mut config = MemexConfig::default();
        let engine = build_engine();
        let result = run_config_script(&engine, &script_path, &mut config);
        assert!(result.is_err());

        fs::remove_dir_all(&dir).ok();
    }
}
