use crate::{
    config::{DisplayConfig, PrometheusConfig},
    prometheus::{parse_metrics, MetricSample},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use tracing::{error, info, warn};

const HISTORY_LIMIT: usize = 60;

pub struct App {
    pub metrics: Vec<MetricSample>,
    pub cursor: usize,
    pub status: String,
    pub source_label: String,
    pub filter_query: String,
    pub filter_input_open: bool,
    pub target_options: Vec<TargetFilter>,
    pub target_selected: usize,
    pub target_picker_open: bool,
    pub target_cursor: usize,
    pub history_view: HistoryView,
    source: DataSource,
    display: DisplayConfig,
    all_metrics: Vec<MetricSample>,
    metric_history: BTreeMap<MetricKey, VecDeque<f64>>,
    selected_metrics: BTreeSet<MetricKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryView {
    Graph,
    Table,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetFilter {
    pub job: String,
    pub target: String,
}

#[derive(Clone, Debug)]
enum DataSource {
    Sample,
    PrometheusApi {
        base_url: String,
        client: reqwest::blocking::Client,
    },
}

impl App {
    pub fn new(prometheus: PrometheusConfig, display: DisplayConfig) -> Self {
        let source = build_source(prometheus);
        let refresh_secs = display.refresh_secs.unwrap_or(15);

        let mut app = Self {
            metrics: Vec::new(),
            cursor: 0,
            status: String::from("initializing"),
            source_label: String::new(),
            filter_query: String::new(),
            filter_input_open: false,
            target_options: vec![TargetFilter::wildcard()],
            target_selected: 0,
            target_picker_open: false,
            target_cursor: 0,
            history_view: HistoryView::Graph,
            source,
            display,
            all_metrics: Vec::new(),
            metric_history: BTreeMap::new(),
            selected_metrics: BTreeSet::new(),
        };
        info!(refresh_secs, "app initialized");
        app.reload();
        app
    }

    pub fn next(&mut self) {
        if let Some(next) = wrapping_next(self.cursor, self.metrics.len()) {
            self.cursor = next;
        }
    }

    pub fn previous(&mut self) {
        if let Some(prev) = wrapping_prev(self.cursor, self.metrics.len()) {
            self.cursor = prev;
        }
    }

    pub fn selected_metric(&self) -> Option<&MetricSample> {
        self.metrics.get(self.cursor)
    }

    pub fn selected_metric_history(&self) -> Vec<f64> {
        self.selected_metric()
            .and_then(|metric| self.metric_history.get(&metric_key(metric)))
            .map(|values| values.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn toggle_metric_selection(&mut self) {
        let Some(metric) = self.selected_metric() else {
            return;
        };
        let key = metric_key(metric);
        let name = metric.name.clone();
        if self.selected_metrics.remove(&key) {
            self.status = format!("unselected {name}");
        } else {
            self.selected_metrics.insert(key);
            self.status = format!("selected {name}");
        }
    }

    pub fn clear_metric_selection(&mut self) {
        self.selected_metrics.clear();
        self.status = String::from("selection cleared");
    }

    pub fn is_metric_selected(&self, metric: &MetricSample) -> bool {
        self.selected_metrics.contains(&metric_key(metric))
    }

    pub fn selected_metrics_len(&self) -> usize {
        self.selected_metrics.len()
    }

    pub fn next_target(&mut self) {
        if let Some(next) = wrapping_next(self.target_selected, self.target_options.len()) {
            self.target_selected = next;
            self.target_cursor = self.target_selected;
            info!(target = %self.selected_target(), "selected next target");
            self.apply_target_selection();
        }
    }

    pub fn previous_target(&mut self) {
        if let Some(prev) = wrapping_prev(self.target_selected, self.target_options.len()) {
            self.target_selected = prev;
            self.target_cursor = self.target_selected;
            info!(target = %self.selected_target(), "selected previous target");
            self.apply_target_selection();
        }
    }

    pub fn selected_target(&self) -> &TargetFilter {
        &self.target_options[self.target_selected]
    }

    pub fn refresh_secs(&self) -> u64 {
        self.display.refresh_secs.unwrap_or(15)
    }

    pub fn toggle_history_view(&mut self) {
        self.history_view = match self.history_view {
            HistoryView::Graph => HistoryView::Table,
            HistoryView::Table => HistoryView::Graph,
        };
        self.status = format!("history view: {}", self.history_view.label());
        info!(
            history_view = self.history_view.label(),
            "toggled history view"
        );
    }

    pub fn open_filter_input(&mut self) {
        self.filter_input_open = true;
        self.status = format!("filter metrics: {}", self.filter_query);
        info!(filter = %self.filter_query, "opened filter input");
    }

    pub fn close_filter_input(&mut self) {
        self.filter_input_open = false;
        self.update_loaded_status();
        info!(filter = %self.filter_query, metrics = self.metrics.len(), "closed filter input");
    }

    pub fn push_filter_char(&mut self, ch: char) {
        let previous_selection = self.selected_metric_key();
        self.filter_query.push(ch);
        self.rebuild_metrics_view(previous_selection.as_ref());
        self.status = format!("filter metrics: {}", self.filter_query);
        info!(filter = %self.filter_query, added = %ch, "updated filter query");
    }

    pub fn pop_filter_char(&mut self) {
        let previous_selection = self.selected_metric_key();
        self.filter_query.pop();
        self.rebuild_metrics_view(previous_selection.as_ref());
        self.status = format!("filter metrics: {}", self.filter_query);
        info!(filter = %self.filter_query, "deleted filter character");
    }

    pub fn clear_filter(&mut self) {
        let previous_selection = self.selected_metric_key();
        self.filter_query.clear();
        self.rebuild_metrics_view(previous_selection.as_ref());
        self.status = String::from("filter cleared");
        info!("cleared filter query");
    }

    pub fn open_target_picker(&mut self) {
        self.target_picker_open = true;
        self.target_cursor = self.target_selected;
        self.status = String::from("select target and press Enter");
        info!(current_target = %self.selected_target(), "opened target picker");
    }

    pub fn close_target_picker(&mut self) {
        self.target_picker_open = false;
        self.update_loaded_status();
        info!(target = %self.selected_target(), "closed target picker");
    }

    pub fn picker_next(&mut self) {
        if let Some(next) = wrapping_next(self.target_cursor, self.target_options.len()) {
            self.target_cursor = next;
        }
    }

    pub fn picker_previous(&mut self) {
        if let Some(prev) = wrapping_prev(self.target_cursor, self.target_options.len()) {
            self.target_cursor = prev;
        }
    }

    pub fn picker_apply(&mut self) {
        if self.target_options.is_empty() {
            return;
        }
        self.target_selected = self.target_cursor;
        self.target_picker_open = false;
        info!(target = %self.selected_target(), "applied target picker selection");
        self.apply_target_selection();
    }

    pub fn reload(&mut self) {
        info!(source = %self.data_source_label(), "starting reload");
        match &self.source {
            DataSource::Sample => {
                self.source_label = String::from("sample");
                self.reload_from_str(SAMPLE_PROMETHEUS);
            }
            DataSource::PrometheusApi { base_url, client } => {
                let base_url = base_url.clone();
                let client = client.clone();
                self.source_label = base_url.clone();
                self.run_prometheus_reload(&base_url, &client, |app, url, http| {
                    app.reload_prometheus(url, http)
                });
            }
        }
    }

    pub fn reload_from_str(&mut self, input: &str) {
        info!(bytes = input.len(), "reloading metrics from text input");
        match parse_metrics(input) {
            Ok(metrics) => {
                info!(metrics = metrics.len(), "parsed metrics successfully");
                self.set_metrics(metrics)
            }
            Err(err) => {
                error!(error = %err, "failed to parse metrics");
                self.reset_failed_state(format!("parse error: {err}"))
            }
        }
    }

    fn reload_prometheus(
        &mut self,
        base_url: &str,
        client: &reqwest::blocking::Client,
    ) -> Result<(), String> {
        let previous = self.target_options.get(self.target_selected).cloned();
        info!(%base_url, "fetching targets from prometheus");
        let targets = fetch_targets(base_url, client)?;
        info!(%base_url, targets = targets.len(), "fetched targets from prometheus");
        self.target_options = targets;
        self.restore_target_selection(previous);
        self.target_picker_open = false;
        self.load_selected_target_metrics(base_url, client)
    }

    fn restore_target_selection(&mut self, previous: Option<TargetFilter>) {
        self.target_selected = previous
            .and_then(|current| self.target_options.iter().position(|item| item == &current))
            .unwrap_or(0);
        self.target_cursor = self.target_selected;
    }

    fn set_metrics(&mut self, metrics: Vec<MetricSample>) {
        let previous_selection = self.selected_metric_key();
        self.all_metrics = metrics;
        self.record_metric_history();
        self.refresh_target_options();
        self.rebuild_metrics_view(previous_selection.as_ref());
        info!(
            loaded_metrics = self.all_metrics.len(),
            visible_metrics = self.metrics.len(),
            target = %self.selected_target(),
            "updated metrics state"
        );
    }

    fn reset_failed_state(&mut self, status: String) {
        warn!(status = %status, "resetting application state after failure");
        self.all_metrics.clear();
        self.metrics.clear();
        self.metric_history.clear();
        self.target_options = vec![TargetFilter::wildcard()];
        self.target_selected = 0;
        self.target_cursor = 0;
        self.target_picker_open = false;
        self.cursor = 0;
        self.selected_metrics.clear();
        self.status = status;
    }

    fn apply_target_selection(&mut self) {
        info!(target = %self.selected_target(), "applying target selection");
        match &self.source {
            DataSource::Sample => {
                let previous_selection = self.selected_metric_key();
                self.rebuild_metrics_view(previous_selection.as_ref());
            }
            DataSource::PrometheusApi { base_url, client } => {
                let base_url = base_url.clone();
                let client = client.clone();
                self.run_prometheus_reload(&base_url, &client, |app, url, http| {
                    app.load_selected_target_metrics(url, http)
                });
            }
        }
    }

    fn load_selected_target_metrics(
        &mut self,
        base_url: &str,
        client: &reqwest::blocking::Client,
    ) -> Result<(), String> {
        let previous_selection = self.selected_metric_key();
        info!(%base_url, target = %self.selected_target(), "fetching target metrics");
        let metrics = fetch_target_metrics(base_url, self.selected_target(), client)?;
        self.all_metrics = metrics;
        self.record_metric_history();
        self.rebuild_metrics_view(previous_selection.as_ref());
        info!(
            %base_url,
            target = %self.selected_target(),
            metrics = self.all_metrics.len(),
            "loaded target metrics"
        );
        Ok(())
    }

    fn refresh_target_options(&mut self) {
        let previous = self.target_options.get(self.target_selected).cloned();
        let filters = self.all_metrics.iter().filter_map(|metric| {
            let job = label_value(metric, "job")?.to_owned();
            let target = label_value(metric, "instance")
                .or_else(|| label_value(metric, "target"))
                .map(str::to_owned)
                .unwrap_or_else(|| String::from("-"));
            Some(TargetFilter { job, target })
        });
        self.target_options = build_target_options(filters);
        self.restore_target_selection(previous);
    }

    fn rebuild_metrics_view(&mut self, previous_selection: Option<&MetricKey>) {
        let needle = self.filter_query.to_lowercase();
        self.metrics = self
            .all_metrics
            .iter()
            .filter(|metric| self.matches_selected_target(metric))
            .filter(|metric| self.matches_filter_with(&needle, metric))
            .cloned()
            .collect();
        if let Some(max_metrics) = self.display.max_metrics {
            self.metrics.truncate(max_metrics);
        }

        self.restore_selection(previous_selection);
        self.retain_visible_selection();

        self.update_loaded_status();
        info!(
            visible_metrics = self.metrics.len(),
            target = %self.selected_target(),
            filter = %self.filter_query,
            "rebuilt metrics view"
        );
    }

    fn record_metric_history(&mut self) {
        let current_keys: BTreeSet<MetricKey> = self.all_metrics.iter().map(metric_key).collect();

        self.metric_history
            .retain(|metric_key, _| current_keys.contains(metric_key));

        for metric in &self.all_metrics {
            let history = self.metric_history.entry(metric_key(metric)).or_default();
            history.push_back(metric.value);
            if history.len() > HISTORY_LIMIT {
                history.pop_front();
            }
        }
    }

    fn restore_selection(&mut self, previous_selection: Option<&MetricKey>) {
        self.cursor = 0;

        if let Some(previous_selection) = previous_selection {
            if let Some(index) = self
                .metrics
                .iter()
                .position(|metric| metric_key(metric) == *previous_selection)
            {
                self.cursor = index;
                return;
            }
        }

        if let Some(initial_metric) = &self.display.initial_metric {
            if let Some(index) = self
                .metrics
                .iter()
                .position(|metric| metric.name == *initial_metric)
            {
                self.cursor = index;
            }
        }
    }

    fn retain_visible_selection(&mut self) {
        let visible_keys: BTreeSet<MetricKey> = self.metrics.iter().map(metric_key).collect();
        self.selected_metrics
            .retain(|metric_key| visible_keys.contains(metric_key));
    }

    fn selected_metric_key(&self) -> Option<MetricKey> {
        self.selected_metric().map(metric_key)
    }

    fn matches_selected_target(&self, metric: &MetricSample) -> bool {
        let selected = self.selected_target();
        if selected.job == "*" {
            return true;
        }

        let metric_job = label_value(metric, "job");
        let metric_target =
            label_value(metric, "instance").or_else(|| label_value(metric, "target"));
        metric_job == Some(selected.job.as_str()) && metric_target == Some(selected.target.as_str())
    }

    fn matches_filter_with(&self, needle: &str, metric: &MetricSample) -> bool {
        if needle.is_empty() {
            return true;
        }

        if metric.name.to_lowercase().contains(needle) {
            return true;
        }

        metric.labels.iter().any(|(key, value)| {
            key.to_lowercase().contains(needle) || value.to_lowercase().contains(needle)
        })
    }

    fn update_loaded_status(&mut self) {
        self.status = format!(
            "loaded {} metrics for {}",
            self.metrics.len(),
            self.selected_target()
        );
    }

    fn run_prometheus_reload<F>(
        &mut self,
        base_url: &str,
        client: &reqwest::blocking::Client,
        reload: F,
    ) where
        F: FnOnce(&mut Self, &str, &reqwest::blocking::Client) -> Result<(), String>,
    {
        if let Err(err) = reload(self, base_url, client) {
            error!(%base_url, error = %err, "reload from prometheus failed");
            self.reset_failed_state(format!("fetch error: {err}"));
        }
    }
}

type MetricKey = (String, Vec<(String, String)>);

impl TargetFilter {
    fn wildcard() -> Self {
        TargetFilter {
            job: String::from("*"),
            target: String::from("*"),
        }
    }
}

impl fmt::Display for TargetFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.job == "*" {
            f.write_str("all targets")
        } else {
            write!(f, "{}/{}", self.job, self.target)
        }
    }
}

impl HistoryView {
    pub fn label(self) -> &'static str {
        match self {
            HistoryView::Graph => "graph",
            HistoryView::Table => "table",
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

fn wrapping_next(index: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some((index + 1) % len)
    }
}

fn wrapping_prev(index: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else if index == 0 {
        Some(len - 1)
    } else {
        Some(index - 1)
    }
}

fn build_source(prometheus: PrometheusConfig) -> DataSource {
    match prometheus.base_url {
        Some(base_url) => DataSource::PrometheusApi {
            base_url,
            client: reqwest::blocking::Client::new(),
        },
        None => DataSource::Sample,
    }
}

impl App {
    fn data_source_label(&self) -> &str {
        match &self.source {
            DataSource::Sample => "sample",
            DataSource::PrometheusApi { base_url, .. } => base_url.as_str(),
        }
    }
}

fn fetch_targets(
    base_url: &str,
    client: &reqwest::blocking::Client,
) -> Result<Vec<TargetFilter>, String> {
    let response = client
        .get(format!("{}/api/v1/series", base_url.trim_end_matches('/')))
        .query(&[("match[]", "up")])
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .map_err(|err| {
            error!(%base_url, error = %err, "prometheus target request failed");
            err.to_string()
        })?;
    let response = response.error_for_status().map_err(|err| {
        error!(%base_url, error = %err, "prometheus target response returned error status");
        err.to_string()
    })?;
    let payload: SeriesResponse = response.json().map_err(|err| {
        error!(%base_url, error = %err, "failed to decode prometheus target response");
        err.to_string()
    })?;

    if payload.status != "success" {
        error!(%base_url, status = %payload.status, "prometheus target query returned non-success status");
        return Err(format!("series status {}", payload.status));
    }

    let filters = payload.data.into_iter().filter_map(|series| {
        let job = series.get("job")?.clone();
        let target = series.get("instance")?.clone();
        Some(TargetFilter { job, target })
    });
    Ok(build_target_options(filters))
}

fn fetch_target_metrics(
    base_url: &str,
    target: &TargetFilter,
    client: &reqwest::blocking::Client,
) -> Result<Vec<MetricSample>, String> {
    let expr = if target.job == "*" {
        String::from("{job!=\"\"}")
    } else {
        format!(
            "{{job=\"{}\",instance=\"{}\"}}",
            escape_matcher(&target.job),
            escape_matcher(&target.target)
        )
    };

    let response = client
        .get(format!("{}/api/v1/query", base_url.trim_end_matches('/')))
        .query(&[("query", expr.as_str())])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .map_err(|err| {
            error!(
                %base_url,
                target = %target,
                query = %expr,
                error = %err,
                "prometheus metric request failed"
            );
            err.to_string()
        })?;
    let response = response.error_for_status().map_err(|err| {
        error!(
            %base_url,
            target = %target,
            query = %expr,
            error = %err,
            "prometheus metric response returned error status"
        );
        err.to_string()
    })?;
    let payload: QueryResponse = response.json().map_err(|err| {
        error!(
            %base_url,
            target = %target,
            error = %err,
            "failed to decode prometheus metric response"
        );
        err.to_string()
    })?;

    if payload.status != "success" {
        error!(
            %base_url,
            target = %target,
            status = %payload.status,
            "prometheus metric query returned non-success status"
        );
        return Err(format!("query status {}", payload.status));
    }

    let data = payload.data.ok_or_else(|| {
        error!(%base_url, target = %target, "prometheus metric response missing data");
        String::from("missing response data")
    })?;
    if data.result_type != "vector" {
        error!(
            %base_url,
            target = %target,
            result_type = %data.result_type,
            "unsupported prometheus result type"
        );
        return Err(format!("unsupported result type {}", data.result_type));
    }

    data.result
        .into_iter()
        .map(QuerySample::into_metric_sample)
        .collect()
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    status: String,
    data: Option<QueryData>,
}

#[derive(Debug, Deserialize)]
struct QueryData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<QuerySample>,
}

#[derive(Debug, Deserialize)]
struct QuerySample {
    metric: BTreeMap<String, String>,
    value: (f64, String),
}

impl QuerySample {
    fn into_metric_sample(self) -> Result<MetricSample, String> {
        let value = self
            .value
            .1
            .parse::<f64>()
            .map_err(|_| String::from("invalid sample value"))?;

        let name = self
            .metric
            .get("__name__")
            .cloned()
            .unwrap_or_else(|| String::from("unknown"));

        let labels: Vec<(String, String)> = self
            .metric
            .into_iter()
            .filter(|(key, _)| key != "__name__")
            .collect();

        Ok(MetricSample {
            name,
            labels,
            value,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SeriesResponse {
    status: String,
    data: Vec<BTreeMap<String, String>>,
}

fn build_target_options(targets: impl Iterator<Item = TargetFilter>) -> Vec<TargetFilter> {
    let mut seen = BTreeSet::new();
    let mut options = vec![TargetFilter::wildcard()];
    for filter in targets {
        if seen.insert((filter.job.clone(), filter.target.clone())) {
            options.push(filter);
        }
    }
    options[1..].sort_by(|left, right| {
        left.job
            .cmp(&right.job)
            .then_with(|| left.target.cmp(&right.target))
    });
    options
}

fn escape_matcher(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn label_value<'a>(metric: &'a MetricSample, key: &str) -> Option<&'a str> {
    metric
        .labels
        .iter()
        .find(|(label_key, _)| label_key == key)
        .map(|(_, value)| value.as_str())
}

fn metric_key(metric: &MetricSample) -> MetricKey {
    (metric.name.clone(), metric.labels.clone())
}

#[cfg(test)]
mod tests {
    use super::{App, HistoryView, TargetFilter, HISTORY_LIMIT};
    use crate::config::{DisplayConfig, PrometheusConfig};

    #[test]
    fn applies_display_config_after_reload() {
        let mut app = App::new(
            PrometheusConfig::default(),
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
        assert_eq!(app.cursor, 1);
        assert_eq!(app.metrics[app.cursor].name, "http_requests_total");
    }

    #[test]
    fn filters_metrics_by_selected_target() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\nup{job=\"api\",instance=\"b:9090\"} 1\n",
        );

        assert_eq!(app.target_options.len(), 3);
        app.next_target();
        assert_eq!(
            app.selected_target(),
            &TargetFilter {
                job: String::from("api"),
                target: String::from("a:9090"),
            }
        );
        assert_eq!(app.metrics.len(), 2);

        app.next_target();
        assert_eq!(app.metrics.len(), 1);
        assert_eq!(app.selected_target().target, "b:9090");
    }

    #[test]
    fn applies_target_picker_selection() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\nup{job=\"api\",instance=\"b:9090\"} 1\n",
        );
        app.open_target_picker();
        app.picker_next();
        app.picker_next();
        app.picker_apply();

        assert!(!app.target_picker_open);
        assert_eq!(app.selected_target().target, "b:9090");
        assert_eq!(app.metrics.len(), 1);
    }

    #[test]
    fn filters_metrics_by_text_query() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 70\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\n",
        );
        app.push_filter_char('g');
        app.push_filter_char('p');
        app.push_filter_char('u');

        assert_eq!(app.metrics.len(), 1);
        assert_eq!(app.metrics[0].name, "gpu_temperature_celsius");

        app.clear_filter();
        assert_eq!(app.metrics.len(), 3);
    }

    #[test]
    fn keeps_selected_metric_on_reload() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: Some(String::from("up")),
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 70\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\n",
        );
        app.next();
        assert_eq!(app.metrics[app.cursor].name, "gpu_temperature_celsius");

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 71\nrequests_total{job=\"api\",instance=\"a:9090\"} 3\n",
        );

        assert_eq!(app.metrics[app.cursor].name, "gpu_temperature_celsius");
    }

    #[test]
    fn keeps_text_filter_applied_after_reload() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 70\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\n",
        );
        app.push_filter_char('g');
        app.push_filter_char('p');
        app.push_filter_char('u');
        assert_eq!(app.metrics.len(), 1);

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 71\nrequests_total{job=\"api\",instance=\"a:9090\"} 3\n",
        );

        assert_eq!(app.metrics.len(), 1);
        assert_eq!(app.metrics[0].name, "gpu_temperature_celsius");
    }

    #[test]
    fn accumulates_selected_metric_history_across_reloads() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: Some(String::from("gpu_temperature_celsius")),
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 70\n",
        );
        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 71\n",
        );
        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 69\n",
        );

        assert_eq!(
            app.selected_metric().map(|metric| metric.name.as_str()),
            Some("gpu_temperature_celsius")
        );
        assert_eq!(app.selected_metric_history(), vec![70.0, 71.0, 69.0]);
    }

    #[test]
    fn caps_metric_history_to_history_limit() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: Some(String::from("up")),
                refresh_secs: None,
            },
        );

        for value in 0..(HISTORY_LIMIT + 5) {
            app.reload_from_str(&format!("up{{job=\"api\",instance=\"a:9090\"}} {value}\n"));
        }

        let expected: Vec<f64> = (5..(HISTORY_LIMIT + 5)).map(|value| value as f64).collect();
        assert_eq!(app.selected_metric_history(), expected);
    }

    #[test]
    fn toggles_multiple_metric_selection() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 70\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\n",
        );

        app.toggle_metric_selection();
        app.next();
        app.toggle_metric_selection();

        assert_eq!(app.selected_metrics_len(), 2);
        assert!(app.is_metric_selected(&app.metrics[0]));
        assert!(app.is_metric_selected(&app.metrics[1]));

        app.previous();
        app.toggle_metric_selection();

        assert_eq!(app.selected_metrics_len(), 1);
        assert!(!app.is_metric_selected(&app.metrics[0]));
        assert!(app.is_metric_selected(&app.metrics[1]));
    }

    #[test]
    fn retains_only_visible_selected_metrics_after_filtering() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\ngpu_temperature_celsius{job=\"api\",instance=\"a:9090\",gpu=\"0\"} 70\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\n",
        );

        app.toggle_metric_selection();
        app.next();
        app.toggle_metric_selection();
        assert_eq!(app.selected_metrics_len(), 2);

        app.push_filter_char('g');
        app.push_filter_char('p');
        app.push_filter_char('u');

        assert_eq!(app.metrics.len(), 1);
        assert_eq!(app.selected_metrics_len(), 1);
        assert_eq!(app.metrics[0].name, "gpu_temperature_celsius");
        assert!(app.is_metric_selected(&app.metrics[0]));
    }

    #[test]
    fn filters_metrics_by_target_label_when_instance_is_missing() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",target=\"alpha\"} 1\nrequests_total{job=\"api\",target=\"alpha\"} 2\nup{job=\"api\",target=\"beta\"} 1\n",
        );

        assert_eq!(app.target_options.len(), 3);
        app.next_target();
        assert_eq!(app.selected_target().target, "alpha");
        assert_eq!(app.metrics.len(), 2);

        app.next_target();
        assert_eq!(app.selected_target().target, "beta");
        assert_eq!(app.metrics.len(), 1);
    }

    #[test]
    fn resets_loaded_state_after_parse_error() {
        let mut app = App::new(
            PrometheusConfig::default(),
            DisplayConfig {
                max_metrics: None,
                initial_metric: None,
                refresh_secs: None,
            },
        );

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 1\nrequests_total{job=\"api\",instance=\"a:9090\"} 2\n",
        );
        app.toggle_metric_selection();
        app.open_target_picker();

        app.reload_from_str("up{job=\"api\" 1\n");

        assert!(app.metrics.is_empty());
        assert!(app.selected_metric().is_none());
        assert_eq!(app.selected_metrics_len(), 0);
        assert_eq!(app.target_options, vec![TargetFilter::wildcard()]);
        assert!(!app.target_picker_open);
        assert!(app.status.starts_with("parse error:"));
    }

    #[test]
    fn toggles_history_view_mode() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());

        assert_eq!(app.history_view, HistoryView::Graph);

        app.toggle_history_view();
        assert_eq!(app.history_view, HistoryView::Table);
        assert_eq!(app.status, "history view: table");

        app.toggle_history_view();
        assert_eq!(app.history_view, HistoryView::Graph);
        assert_eq!(app.status, "history view: graph");
    }
}
