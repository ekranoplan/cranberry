use std::{fs, path::Path};

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub prometheus: PrometheusConfig,
    #[serde(default)]
    pub loki: LokiConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PrometheusConfig {
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LokiConfig {
    #[serde(default = "default_loki_base_url")]
    pub base_url: Option<String>,
    #[serde(default = "default_loki_host_label")]
    pub host_label: String,
    #[serde(default = "default_loki_log_label")]
    pub log_label: String,
    #[serde(default = "default_loki_poll_secs")]
    pub poll_secs: u64,
    #[serde(default = "default_loki_lookback_secs")]
    pub lookback_secs: u64,
}

impl Default for LokiConfig {
    fn default() -> Self {
        Self {
            base_url: default_loki_base_url(),
            host_label: default_loki_host_label(),
            log_label: default_loki_log_label(),
            poll_secs: default_loki_poll_secs(),
            lookback_secs: default_loki_lookback_secs(),
        }
    }
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

fn default_loki_host_label() -> String {
    String::from("host")
}

fn default_loki_base_url() -> Option<String> {
    Some(String::from("http://127.0.0.1:3100"))
}

fn default_loki_log_label() -> String {
    String::from("job")
}

fn default_loki_poll_secs() -> u64 {
    1
}

fn default_loki_lookback_secs() -> u64 {
    300
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
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

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
    fn applies_loki_defaults_when_section_is_missing() {
        let config: Config = toml::from_str(
            r#"
            [display]
            refresh_secs = 10
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            config.loki.base_url.as_deref(),
            Some("http://127.0.0.1:3100")
        );
        assert_eq!(config.loki.host_label, "host");
        assert_eq!(config.loki.log_label, "job");
        assert_eq!(config.loki.poll_secs, 1);
        assert_eq!(config.loki.lookback_secs, 300);
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

    #[test]
    fn parses_loki_configuration() {
        let config: Config = toml::from_str(
            r#"
            [loki]
            base_url = "http://127.0.0.1:3100"
            host_label = "instance"
            log_label = "container"
            poll_secs = 2
            lookback_secs = 120
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            config.loki.base_url.as_deref(),
            Some("http://127.0.0.1:3100")
        );
        assert_eq!(config.loki.host_label, "instance");
        assert_eq!(config.loki.log_label, "container");
        assert_eq!(config.loki.poll_secs, 2);
        assert_eq!(config.loki.lookback_secs, 120);
    }

    #[test]
    fn load_reports_parse_errors_with_file_path() {
        let path = unique_test_path("invalid-config.toml");
        fs::write(&path, "[logging\nlevel = \"info\"\n").expect("config file should be written");

        let err = Config::load(&path).expect_err("config should fail to parse");

        assert!(err.contains("failed to parse"));
        assert!(err.contains(path.to_string_lossy().as_ref()));
        fs::remove_file(path).expect("temporary config file should be removed");
    }

    fn unique_test_path(file_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cranberry-config-test-{}-{nanos}-{file_name}",
            std::process::id()
        ))
    }
}
