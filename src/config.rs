use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{ImDocument, value};

/// User configuration loaded from the global TOML file.
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

/// Every field is optional because the file overrides native defaults.
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

/// Load native defaults, then the global config.
pub fn load_config() -> MemexConfig {
    load_config_from_path(Some(global_config_path()).filter(|path| path.exists()))
}

/// Persist a theme selection to the global configuration file.
pub fn save_theme(theme: &str) -> Result<PathBuf, String> {
    if crate::theme::by_id(theme).is_none() {
        return Err(format!("unknown theme '{theme}'"));
    }

    let path = global_config_path();
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid config path: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;

    save_theme_to_path(&path, theme)?;
    Ok(path)
}

fn save_theme_to_path(path: &Path, theme: &str) -> Result<(), String> {
    let source = if path.exists() {
        std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
    } else {
        String::new()
    };
    let document = ImDocument::parse(source.clone())
        .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;

    let output = if let Some(item) = document.get("theme") {
        item
            .as_value()
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("theme in {} must be a string", path.display()))?;
        let span = item
            .span()
            .ok_or_else(|| format!("could not locate theme in {}", path.display()))?;
        let representation = &source[span.clone()];
        let replacement = if representation.starts_with("'''") {
            format!("'''{theme}'''")
        } else if representation.starts_with('\'') {
            format!("'{theme}'")
        } else if representation.starts_with("\"\"\"") {
            format!("\"\"\"{theme}\"\"\"")
        } else {
            toml::Value::String(theme.to_string()).to_string()
        };

        let mut output = source;
        output.replace_range(span, &replacement);
        output
    } else {
        let mut document = document.into_mut();
        document["theme"] = value(theme);
        document.to_string()
    };

    std::fs::write(path, output)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    Ok(())
}

fn load_config_from_path(path: Option<PathBuf>) -> MemexConfig {
    let mut config = MemexConfig::default();
    if let Some(path) = path {
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
        let config = load_config_from_path(Some(path.clone()));
        assert_eq!(config.theme, "nord");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn rejects_unknown_keys() {
        let path = temp_config("unknown", "magic = true\n");
        let mut config = MemexConfig::default();
        assert!(apply_config_file(&path, &mut config).is_err());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn refuses_to_save_unknown_theme() {
        assert!(save_theme("missing").is_err());
    }

    #[test]
    fn updating_theme_preserves_comments_order_and_style() {
        let source = concat!(
            "# Memex settings\n",
            "future_option = 42\n",
            "\n",
            "theme=\"nord\" # keep this comment\n",
            "another_option = true\n",
        );
        let path = temp_config("preserve", source);

        save_theme_to_path(&path, "gruvbox-dark").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            source.replace("\"nord\"", "\"gruvbox-dark\"")
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn adding_theme_preserves_existing_document() {
        let source = "# Memex settings\nfuture_option = 42\n";
        let path = temp_config("append", source);

        save_theme_to_path(&path, "nord").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{source}theme = \"nord\"\n")
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn updating_theme_preserves_quote_style() {
        let source = "theme = 'nord' # literal string\n";
        let path = temp_config("quotes", source);

        save_theme_to_path(&path, "solarized-dark").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "theme = 'solarized-dark' # literal string\n"
        );
        std::fs::remove_file(path).ok();
    }
}
