use crate::{
    config::DisplayConfig,
    prometheus::{parse_metrics, MetricSample},
};

pub struct App {
    pub metrics: Vec<MetricSample>,
    pub selected: usize,
    pub status: String,
    pub source_label: String,
    source: DataSource,
    display: DisplayConfig,
}

#[derive(Clone, Debug)]
enum DataSource {
    Sample,
    Http { url: String },
}

impl App {
    pub fn new(source_url: Option<String>, display: DisplayConfig) -> Self {
        let source = match source_url {
            Some(url) => DataSource::Http { url },
            None => DataSource::Sample,
        };

        let mut app = Self {
            metrics: Vec::new(),
            selected: 0,
            status: String::from("initializing"),
            source_label: String::new(),
            source,
            display,
        };
        app.reload();
        app
    }

    pub fn next(&mut self) {
        if self.metrics.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.metrics.len();
    }

    pub fn previous(&mut self) {
        if self.metrics.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.metrics.len() - 1
        } else {
            self.selected - 1
        };
    }

    pub fn selected_metric(&self) -> Option<&MetricSample> {
        self.metrics.get(self.selected)
    }

    pub fn reload(&mut self) {
        match &self.source {
            DataSource::Sample => {
                self.source_label = String::from("sample");
                self.reload_from_str(SAMPLE_PROMETHEUS);
            }
            DataSource::Http { url } => {
                self.source_label = url.clone();
                match fetch_metrics(url) {
                    Ok(body) => self.reload_from_str(&body),
                    Err(err) => {
                        self.metrics.clear();
                        self.selected = 0;
                        self.status = format!("fetch error: {err}");
                    }
                }
            }
        }
    }

    pub fn reload_from_str(&mut self, input: &str) {
        match parse_metrics(input) {
            Ok(metrics) => {
                self.metrics = metrics;
                self.apply_display_config();
                self.status = format!("loaded {} metrics", self.metrics.len());
            }
            Err(err) => {
                self.metrics.clear();
                self.selected = 0;
                self.status = format!("parse error: {err}");
            }
        }
    }

    fn apply_display_config(&mut self) {
        if let Some(max_metrics) = self.display.max_metrics {
            self.metrics.truncate(max_metrics);
        }

        self.selected = 0;
        if let Some(initial_metric) = &self.display.initial_metric {
            if let Some(index) = self
                .metrics
                .iter()
                .position(|metric| metric.name == *initial_metric)
            {
                self.selected = index;
            }
        }
    }
}

const SAMPLE_PROMETHEUS: &str = r#"
# HELP up Was the last scrape of the target successful.
# TYPE up gauge
up{job="node",instance="localhost:9100"} 1
process_cpu_seconds_total{instance="localhost:9100"} 12.4
process_resident_memory_bytes{instance="localhost:9100"} 4.194304e+07
http_requests_total{method="GET",code="200"} 128
http_requests_total{method="GET",code="500"} 3
"#;

fn fetch_metrics(url: &str) -> Result<String, String> {
    let response = reqwest::blocking::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .map_err(|err| err.to_string())?;

    let response = response.error_for_status().map_err(|err| err.to_string())?;
    response.text().map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::App;
    use crate::config::DisplayConfig;

    #[test]
    fn applies_display_config_after_reload() {
        let mut app = App::new(
            None,
            DisplayConfig {
                max_metrics: Some(2),
                initial_metric: Some(String::from("http_requests_total")),
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"node\"} 1\nhttp_requests_total{code=\"200\"} 2\nprocess_cpu_seconds_total 3\n",
        );

        assert_eq!(app.metrics.len(), 2);
        assert_eq!(app.selected, 1);
        assert_eq!(app.metrics[app.selected].name, "http_requests_total");
    }
}
