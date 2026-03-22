use std::{fs, path::Path};

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub prometheus: PrometheusConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PrometheusConfig {
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DisplayConfig {
    pub max_metrics: Option<usize>,
    pub initial_metric: Option<String>,
    pub refresh_secs: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_path")]
    pub path: String,
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            path: default_log_path(),
            level: default_log_level(),
        }
    }
}

fn default_log_path() -> String {
    String::from("cranberry.log")
}

fn default_log_level() -> String {
    String::from("info")
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        toml::from_str(&text).map_err(|err| format!("failed to parse {}: {err}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn applies_logging_defaults_when_section_is_missing() {
        let config: Config = toml::from_str(
            r#"
            [display]
            refresh_secs = 10
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.logging.path, "cranberry.log");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn parses_logging_configuration() {
        let config: Config = toml::from_str(
            r#"
            [logging]
            path = "logs/cranberry-dev.log"
            level = "debug"
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.logging.path, "logs/cranberry-dev.log");
        assert_eq!(config.logging.level, "debug");
    }
}
