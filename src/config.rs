use std::{fs, path::Path};

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub prometheus: PrometheusConfig,
    #[serde(default)]
    pub display: DisplayConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PrometheusConfig {
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DisplayConfig {
    pub max_metrics: Option<usize>,
    pub initial_metric: Option<String>,
    pub refresh_secs: Option<u64>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        toml::from_str(&text).map_err(|err| format!("failed to parse {}: {err}", path.display()))
    }
}
