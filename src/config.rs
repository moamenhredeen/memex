use std::path::{Path, PathBuf};

use serde::Deserialize;

/// User configuration after global and vault overlays have been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemexConfig {
    pub theme: String,
}

impl Default for MemexConfig {
    fn default() -> Self {
        Self {
            theme: crate::theme::SOLARIZED_LIGHT.id.to_string(),
        }
    }
}

/// Every field is optional because each file is an overlay on native defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    theme: Option<String>,
}

impl ConfigFile {
    fn apply(self, config: &mut MemexConfig) -> Result<(), String> {
        if let Some(theme) = self.theme {
            if crate::theme::by_id(&theme).is_none() {
                return Err(format!(
                    "unknown theme '{theme}'; available themes: {}",
                    crate::theme::THEMES
                        .iter()
                        .map(|theme| theme.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            config.theme = theme;
        }
        Ok(())
    }
}

fn apply_config_file(path: &Path, config: &mut MemexConfig) -> Result<(), String> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let file: ConfigFile = toml::from_str(&source)
        .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;
    file.apply(config)
        .map_err(|error| format!("invalid config in {}: {error}", path.display()))
}

/// Load native defaults, then the global config, then the vault overlay.
pub fn load_config(vault_path: Option<&Path>) -> MemexConfig {
    load_config_from_paths(
        Some(global_config_path()).filter(|path| path.exists()),
        vault_path
            .map(|vault| vault.join(".memex").join("config.toml"))
            .filter(|path| path.exists()),
    )
}

fn load_config_from_paths(global: Option<PathBuf>, vault: Option<PathBuf>) -> MemexConfig {
    let mut config = MemexConfig::default();
    for path in [global, vault].into_iter().flatten() {
        if let Err(error) = apply_config_file(&path, &mut config) {
            eprintln!("config error: {error}");
        }
    }
    config
}

fn global_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("memex")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str, source: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "memex-config-{name}-{}-{}.toml",
            std::process::id(),
            fastrand::u64(..)
        ));
        std::fs::write(&path, source).unwrap();
        path
    }

    #[test]
    fn defaults_are_powerful_without_a_config_file() {
        assert_eq!(MemexConfig::default().theme, "solarized-light");
    }

    #[test]
    fn loads_toml_config() {
        let path = temp_config("theme", "theme = \"nord\"\n");
        let config = load_config_from_paths(Some(path.clone()), None);
        assert_eq!(config.theme, "nord");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn vault_config_overlays_global_config() {
        let global = temp_config("global", "theme = \"nord\"\n");
        let vault = temp_config("vault", "theme = \"gruvbox-dark\"\n");
        let config = load_config_from_paths(Some(global.clone()), Some(vault.clone()));
        assert_eq!(config.theme, "gruvbox-dark");
        std::fs::remove_file(global).ok();
        std::fs::remove_file(vault).ok();
    }

    #[test]
    fn invalid_overlay_keeps_the_previous_value() {
        let global = temp_config("valid", "theme = \"nord\"\n");
        let vault = temp_config("invalid", "theme = \"missing\"\n");
        let config = load_config_from_paths(Some(global.clone()), Some(vault.clone()));
        assert_eq!(config.theme, "nord");
        std::fs::remove_file(global).ok();
        std::fs::remove_file(vault).ok();
    }

    #[test]
    fn rejects_unknown_keys() {
        let path = temp_config("unknown", "magic = true\n");
        let mut config = MemexConfig::default();
        assert!(apply_config_file(&path, &mut config).is_err());
        std::fs::remove_file(path).ok();
    }
}
