use crate::platform::PlatformPaths;
use crate::rules::Rule;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CleanupConfig {
    #[serde(default)]
    pub scan_paths: Vec<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CleanupState {
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub dismissed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub config_file: PathBuf,
    pub state_file: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ConfigError {
    #[error("Failed to read config: {0}")]
    ReadError(String),
    #[error("Failed to parse config: {0}")]
    ParseError(String),
    #[error("Missing home directory")]
    MissingHome,
}

pub const DEFAULT_RULES: &str = include_str!("defaults.toml");

pub fn default_config_paths(platform_paths: &PlatformPaths) -> ConfigPaths {
    let base_dir = platform_paths.config_dir.join("mcdu");

    ConfigPaths {
        config_file: base_dir.join("cleanup.toml"),
        state_file: base_dir.join("cleanup-state.toml"),
    }
}

pub fn load_config(paths: &ConfigPaths) -> Result<CleanupConfig, ConfigError> {
    let mut config: CleanupConfig =
        toml::from_str(DEFAULT_RULES).map_err(|e| ConfigError::ParseError(e.to_string()))?;

    if paths.config_file.exists() {
        let contents = fs::read_to_string(&paths.config_file)
            .map_err(|e| ConfigError::ReadError(e.to_string()))?;
        let user_config: CleanupConfig =
            toml::from_str(&contents).map_err(|e| ConfigError::ParseError(e.to_string()))?;

        if !user_config.scan_paths.is_empty() {
            config.scan_paths = user_config.scan_paths;
        }

        if !user_config.rules.is_empty() {
            // Merge by name: user rules override defaults
            for user_rule in user_config.rules {
                if let Some(existing) = config.rules.iter_mut().find(|r| r.name == user_rule.name) {
                    *existing = user_rule;
                } else {
                    config.rules.push(user_rule);
                }
            }
        }
    }

    if config.scan_paths.is_empty() {
        config.scan_paths.push("${HOME}".to_string());
    }

    Ok(config)
}

pub fn load_state(paths: &ConfigPaths) -> Result<CleanupState, ConfigError> {
    if !paths.state_file.exists() {
        return Ok(CleanupState::default());
    }

    let contents =
        fs::read_to_string(&paths.state_file).map_err(|e| ConfigError::ReadError(e.to_string()))?;

    toml::from_str(&contents).map_err(|e| ConfigError::ParseError(e.to_string()))
}

pub fn save_state(paths: &ConfigPaths, state: &CleanupState) -> Result<(), ConfigError> {
    if let Some(dir) = paths.state_file.parent() {
        fs::create_dir_all(dir).map_err(|e| ConfigError::ReadError(e.to_string()))?;
    }

    let serialized =
        toml::to_string_pretty(state).map_err(|e| ConfigError::ParseError(e.to_string()))?;

    let tmp_path = paths.state_file.with_extension("tmp");
    {
        let mut file =
            fs::File::create(&tmp_path).map_err(|e| ConfigError::ReadError(e.to_string()))?;
        file.write_all(serialized.as_bytes())
            .map_err(|e| ConfigError::ReadError(e.to_string()))?;
        file.sync_all()
            .map_err(|e| ConfigError::ReadError(e.to_string()))?;
    }
    fs::rename(&tmp_path, &paths.state_file).map_err(|e| ConfigError::ReadError(e.to_string()))
}

#[allow(dead_code)]
pub fn derive_categories(config: &CleanupConfig) -> Vec<Category> {
    let mut categories: Vec<Category> = Vec::new();

    for rule in &config.rules {
        if let Some(cat) = categories.iter_mut().find(|c| c.name == rule.category) {
            cat.rules.push(rule.clone());
        } else {
            categories.push(Category {
                name: rule.category.clone(),
                rules: vec![rule.clone()],
            });
        }
    }

    categories
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn platform_paths(tmp: &tempfile::TempDir) -> PlatformPaths {
        PlatformPaths {
            home_dir: tmp.path().join("home"),
            cache_dir: tmp.path().join("cache"),
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
        }
    }

    #[test]
    fn loads_defaults_when_no_user_config() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let config_paths = default_config_paths(&paths);

        let config = load_config(&config_paths).unwrap();
        assert!(!config.rules.is_empty());
        assert!(!config.scan_paths.is_empty());
    }

    #[test]
    fn merges_user_rules_with_defaults() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let config_dir = paths.config_dir.join("mcdu");
        fs::create_dir_all(&config_dir).unwrap();
        let user_config = r#"
scan_paths = ["~/custom"]

[[rules]]
name = "custom"
category = "tests"
path = "~/custom"
pattern = "**/*"
enabled = true
risky = false
"#;
        fs::write(config_dir.join("cleanup.toml"), user_config).unwrap();

        let config_paths = default_config_paths(&paths);
        let config = load_config(&config_paths).unwrap();

        assert!(config
            .rules
            .iter()
            .any(|rule| rule.name == "custom" && rule.category == "tests"));
        assert_eq!(config.scan_paths, vec!["~/custom".to_string()]);
    }

    #[test]
    fn user_rule_overrides_default_by_name() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let config_dir = paths.config_dir.join("mcdu");
        fs::create_dir_all(&config_dir).unwrap();

        // Pick a known default rule name if present; otherwise skip meaningful assert
        let defaults: CleanupConfig = toml::from_str(DEFAULT_RULES).unwrap();
        let Some(default_name) = defaults.rules.first().map(|r| r.name.clone()) else {
            return;
        };

        let user_config = format!(
            r#"
[[rules]]
name = "{default_name}"
category = "overridden"
path = "~/nowhere"
pattern = "**/*"
enabled = false
risky = true
"#
        );
        fs::write(config_dir.join("cleanup.toml"), user_config).unwrap();

        let config_paths = default_config_paths(&paths);
        let config = load_config(&config_paths).unwrap();
        let matches: Vec<_> = config
            .rules
            .iter()
            .filter(|r| r.name == default_name)
            .collect();
        assert_eq!(matches.len(), 1);
        assert!(!matches[0].enabled);
        assert_eq!(matches[0].category, "overridden");
    }

    #[test]
    fn loads_state_if_present_else_default() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let config_dir = paths.config_dir.join("mcdu");
        fs::create_dir_all(&config_dir).unwrap();
        let state = CleanupState {
            selected: vec!["rule-a".into()],
            dismissed: vec!["rule-b".into()],
        };
        let serialized = toml::to_string(&state).unwrap();
        fs::write(config_dir.join("cleanup-state.toml"), serialized).unwrap();

        let config_paths = default_config_paths(&paths);
        let loaded = load_state(&config_paths).unwrap();

        assert_eq!(loaded.selected, state.selected);
        assert_eq!(loaded.dismissed, state.dismissed);
    }

    #[test]
    fn saves_state_to_file() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let config_paths = default_config_paths(&paths);
        let state = CleanupState {
            selected: vec!["rule-a".into(), "rule-b".into()],
            dismissed: vec!["rule-c".into()],
        };

        save_state(&config_paths, &state).unwrap();
        let loaded = load_state(&config_paths).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn derives_categories_from_rules_preserving_order() {
        let config = CleanupConfig {
            scan_paths: vec![],
            rules: vec![
                Rule {
                    name: "a".into(),
                    category: "cat1".into(),
                    pattern: "**/*".into(),
                    path: "~/tmp".into(),
                    signature: None,
                    min_age_hours: None,
                    min_size_bytes: None,
                    risky: false,
                    enabled: true,
                    ..Default::default()
                },
                Rule {
                    name: "b".into(),
                    category: "cat2".into(),
                    pattern: "**/*".into(),
                    path: "~/tmp".into(),
                    signature: None,
                    min_age_hours: None,
                    min_size_bytes: None,
                    risky: false,
                    enabled: true,
                    ..Default::default()
                },
                Rule {
                    name: "c".into(),
                    category: "cat1".into(),
                    pattern: "**/*".into(),
                    path: "~/tmp".into(),
                    signature: None,
                    min_age_hours: None,
                    min_size_bytes: None,
                    risky: false,
                    enabled: true,
                    ..Default::default()
                },
            ],
        };

        let cats = derive_categories(&config);
        assert_eq!(cats.len(), 2);
        assert_eq!(cats[0].name, "cat1");
        assert_eq!(cats[0].rules.len(), 2);
        assert_eq!(cats[1].name, "cat2");
        assert_eq!(cats[1].rules.len(), 1);
    }
}
