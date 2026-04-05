use crate::{
    config::{DisplayConfig, LokiConfig, PrometheusConfig},
    loki::{LogEntry, LokiClient},
    prometheus::{parse_metrics, MetricSample},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use tracing::{error, info, warn};

const HISTORY_LIMIT: usize = 60;
const LOG_ENTRY_LIMIT: usize = 3000;

type LogStreamKey = (String, String);

#[derive(Clone, Debug, Default)]
struct LogStreamState {
    entries: Vec<LogEntry>,
    last_timestamp_ns: Option<i64>,
    tail_offset: usize,
}

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
    pub screen: ScreenMode,
    pub log_focus: LogFocus,
    pub log_hosts: Vec<String>,
    pub log_host_selected: usize,
    pub log_names: Vec<String>,
    pub log_name_selected: usize,
    pub log_entries: Vec<LogEntry>,
    pub log_filter_query: String,
    pub log_filter_input_open: bool,
    source: DataSource,
    loki: Option<LokiClient>,
    loki_poll_secs: u64,
    log_streams: BTreeMap<LogStreamKey, LogStreamState>,
    log_tail_offset: usize,
    last_log_timestamp_ns: Option<i64>,
    display: DisplayConfig,
    all_metrics: Vec<MetricSample>,
    metric_history: BTreeMap<MetricKey, VecDeque<f64>>,
    selected_metrics: BTreeSet<MetricKey>,
    target_notice: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryView {
    Graph,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenMode {
    Metrics,
    Logs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogFocus {
    Hosts,
    Logs,
    Tail,
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

#[derive(Clone, Debug)]
struct TargetStateSnapshot {
    selected: Option<TargetFilter>,
    cursor: Option<TargetFilter>,
}

#[derive(Debug, Default)]
struct TargetStateRestore {
    selected_missing: bool,
    cursor_missing: bool,
}

impl App {
    #[cfg(test)]
    pub fn new(prometheus: PrometheusConfig, display: DisplayConfig) -> Self {
        Self::with_loki(prometheus, LokiConfig::default(), display)
    }

    pub fn with_loki(
        prometheus: PrometheusConfig,
        loki: LokiConfig,
        display: DisplayConfig,
    ) -> Self {
        let source = build_source(prometheus);
        let loki_poll_secs = loki.poll_secs.max(1);
        let loki = LokiClient::new(loki);
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
            screen: ScreenMode::Metrics,
            log_focus: LogFocus::Hosts,
            log_hosts: Vec::new(),
            log_host_selected: 0,
            log_names: Vec::new(),
            log_name_selected: 0,
            log_entries: Vec::new(),
            log_filter_query: String::new(),
            log_filter_input_open: false,
            source,
            loki,
            loki_poll_secs,
            log_streams: BTreeMap::new(),
            log_tail_offset: 0,
            last_log_timestamp_ns: None,
            display,
            all_metrics: Vec::new(),
            metric_history: BTreeMap::new(),
            selected_metrics: BTreeSet::new(),
            target_notice: None,
        };
        info!(refresh_secs, "app initialized");
        app.reload();
        app
    }

    pub fn next(&mut self) {
        let _ = advance_wrapped_index(&mut self.cursor, self.metrics.len(), 1);
    }

    pub fn previous(&mut self) {
        let _ = advance_wrapped_index(&mut self.cursor, self.metrics.len(), -1);
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
        if advance_wrapped_index(&mut self.target_selected, self.target_options.len(), 1) {
            self.target_cursor = self.target_selected;
            info!(target = %self.selected_target(), "selected next target");
            self.apply_target_selection();
        }
    }

    pub fn previous_target(&mut self) {
        if advance_wrapped_index(&mut self.target_selected, self.target_options.len(), -1) {
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

    pub fn log_poll_secs(&self) -> u64 {
        self.loki_poll_secs
    }

    pub fn is_logs_screen(&self) -> bool {
        self.screen == ScreenMode::Logs
    }

    pub fn selected_log_host(&self) -> Option<&str> {
        self.log_hosts
            .get(self.log_host_selected)
            .map(String::as_str)
    }

    pub fn selected_log_name(&self) -> Option<&str> {
        self.log_names
            .get(self.log_name_selected)
            .map(String::as_str)
    }

    pub fn visible_log_entries(&self) -> Vec<&LogEntry> {
        let needle = self.log_filter_query.to_lowercase();
        self.log_entries
            .iter()
            .filter(|entry| self.matches_log_filter_with(&needle, entry))
            .collect()
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
        let _ = advance_wrapped_index(&mut self.target_cursor, self.target_options.len(), 1);
    }

    pub fn picker_previous(&mut self) {
        let _ = advance_wrapped_index(&mut self.target_cursor, self.target_options.len(), -1);
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

    pub fn open_logs(&mut self) {
        if self.loki.is_none() {
            self.status = String::from("loki is not configured");
            return;
        }
        self.filter_input_open = false;
        self.log_filter_input_open = false;
        self.target_picker_open = false;
        self.screen = ScreenMode::Logs;
        self.status = String::from("loading loki labels");
        self.refresh_log_options();
        self.restore_current_log_stream();
        self.refresh_logs();
    }

    pub fn close_logs(&mut self) {
        self.log_filter_input_open = false;
        self.screen = ScreenMode::Metrics;
        self.update_loaded_status();
    }

    pub fn next_log_focus(&mut self) {
        self.log_focus = match self.log_focus {
            LogFocus::Hosts => LogFocus::Logs,
            LogFocus::Logs => LogFocus::Tail,
            LogFocus::Tail => LogFocus::Hosts,
        };
        self.status = format!("log pane focus: {}", self.log_focus.label());
    }

    pub fn previous_log_focus(&mut self) {
        self.log_focus = match self.log_focus {
            LogFocus::Hosts => LogFocus::Tail,
            LogFocus::Logs => LogFocus::Hosts,
            LogFocus::Tail => LogFocus::Logs,
        };
        self.status = format!("log pane focus: {}", self.log_focus.label());
    }

    pub fn next_log_option(&mut self) {
        self.advance_log_option(1);
    }

    pub fn previous_log_option(&mut self) {
        self.advance_log_option(-1);
    }

    pub fn open_log_filter_input(&mut self) {
        self.log_filter_input_open = true;
        self.status = format!("filter logs: {}", self.log_filter_query);
        info!(filter = %self.log_filter_query, "opened log filter input");
    }

    pub fn close_log_filter_input(&mut self) {
        self.log_filter_input_open = false;
        self.update_log_status();
        info!(filter = %self.log_filter_query, visible_logs = self.visible_log_entries().len(), "closed log filter input");
    }

    pub fn push_log_filter_char(&mut self, ch: char) {
        self.log_filter_query.push(ch);
        self.clamp_log_tail_offset();
        self.store_current_log_stream();
        self.status = format!("filter logs: {}", self.log_filter_query);
        info!(filter = %self.log_filter_query, added = %ch, "updated log filter query");
    }

    pub fn pop_log_filter_char(&mut self) {
        self.log_filter_query.pop();
        self.clamp_log_tail_offset();
        self.store_current_log_stream();
        self.status = format!("filter logs: {}", self.log_filter_query);
        info!(filter = %self.log_filter_query, "deleted log filter character");
    }

    pub fn clear_log_filter(&mut self) {
        self.log_filter_query.clear();
        self.clamp_log_tail_offset();
        self.store_current_log_stream();
        self.status = String::from("log filter cleared");
        info!("cleared log filter query");
    }

    pub fn log_tail_scroll_offset(&self) -> usize {
        self.log_tail_offset
    }

    pub fn scroll_log_tail_up(&mut self, step: usize) {
        self.log_tail_offset = self.log_tail_offset.saturating_add(step);
        self.clamp_log_tail_offset();
        self.store_current_log_stream();
    }

    pub fn scroll_log_tail_down(&mut self, step: usize) {
        self.log_tail_offset = self.log_tail_offset.saturating_sub(step);
        self.store_current_log_stream();
    }

    pub fn scroll_log_tail_to_oldest(&mut self) {
        self.log_tail_offset = self.max_log_tail_offset();
        self.store_current_log_stream();
    }

    pub fn scroll_log_tail_to_latest(&mut self) {
        self.log_tail_offset = 0;
        self.store_current_log_stream();
    }

    pub fn refresh_logs(&mut self) {
        let Some(loki) = &self.loki else {
            self.status = String::from("loki is not configured");
            return;
        };
        let host = self.selected_log_host().map(str::to_owned);
        let log_name = self.selected_log_name().map(str::to_owned);
        let (Some(host), Some(log_name)) = (host, log_name) else {
            self.status = String::from("no loki host/log labels available");
            return;
        };

        match loki.poll_logs(&host, &log_name, self.last_log_timestamp_ns) {
            Ok(entries) => self.append_log_entries(entries),
            Err(err) => {
                self.status = format!("loki log fetch error: {err}");
            }
        }
    }

    pub fn reload_logs_screen(&mut self) {
        self.refresh_log_options();
        self.restore_current_log_stream();
        self.refresh_logs();
    }

    fn advance_log_option(&mut self, step: isize) {
        match self.log_focus {
            LogFocus::Hosts => {
                if advance_wrapped_index(&mut self.log_host_selected, self.log_hosts.len(), step) {
                    self.restore_current_log_stream();
                    self.refresh_logs();
                }
            }
            LogFocus::Logs => {
                if advance_wrapped_index(&mut self.log_name_selected, self.log_names.len(), step) {
                    self.restore_current_log_stream();
                    self.refresh_logs();
                }
            }
            LogFocus::Tail => {}
        }
    }

    fn clear_current_log_stream(&mut self) {
        self.last_log_timestamp_ns = None;
        self.log_entries.clear();
        self.log_tail_offset = 0;
    }

    fn current_log_stream_key(&self) -> Option<LogStreamKey> {
        Some((
            self.selected_log_host()?.to_owned(),
            self.selected_log_name()?.to_owned(),
        ))
    }

    fn restore_current_log_stream(&mut self) {
        let Some(key) = self.current_log_stream_key() else {
            self.clear_current_log_stream();
            return;
        };

        if let Some(state) = self.log_streams.get(&key) {
            self.last_log_timestamp_ns = state.last_timestamp_ns;
            self.log_entries = state.entries.clone();
            self.log_tail_offset = state.tail_offset;
            self.clamp_log_tail_offset();
        } else {
            self.clear_current_log_stream();
        }
    }

    fn store_current_log_stream(&mut self) {
        let Some(key) = self.current_log_stream_key() else {
            return;
        };

        self.log_streams.insert(
            key,
            LogStreamState {
                entries: self.log_entries.clone(),
                last_timestamp_ns: self.last_log_timestamp_ns,
                tail_offset: self.log_tail_offset,
            },
        );
    }

    fn append_log_entries(&mut self, entries: Vec<LogEntry>) {
        let needle = self.log_filter_query.to_lowercase();
        let appended_visible = if self.log_tail_offset == 0 {
            0
        } else {
            entries
                .iter()
                .filter(|entry| self.matches_log_filter_with(&needle, entry))
                .count()
        };

        if let Some(last) = entries.last() {
            self.last_log_timestamp_ns = Some(last.timestamp_ns);
        }

        self.log_entries.extend(entries);
        if self.log_entries.len() > LOG_ENTRY_LIMIT {
            let drop_len = self.log_entries.len() - LOG_ENTRY_LIMIT;
            self.log_entries.drain(0..drop_len);
        }

        if appended_visible > 0 {
            self.log_tail_offset = self.log_tail_offset.saturating_add(appended_visible);
        }

        self.clamp_log_tail_offset();
        self.store_current_log_stream();
        self.update_log_status();
    }

    fn clamp_log_tail_offset(&mut self) {
        self.log_tail_offset = self.log_tail_offset.min(self.max_log_tail_offset());
    }

    fn max_log_tail_offset(&self) -> usize {
        self.visible_log_entries().len().saturating_sub(1)
    }

    fn refresh_log_options(&mut self) {
        let Some(loki) = &self.loki else {
            return;
        };

        match loki.fetch_hosts() {
            Ok(hosts) => {
                self.log_hosts = hosts;
                if self.log_host_selected >= self.log_hosts.len() {
                    self.log_host_selected = 0;
                }
            }
            Err(err) => {
                self.status = format!("loki host labels error: {err}");
                return;
            }
        }

        match loki.fetch_logs() {
            Ok(log_names) => {
                self.log_names = log_names;
                if self.log_name_selected >= self.log_names.len() {
                    self.log_name_selected = 0;
                }
            }
            Err(err) => {
                self.status = format!("loki log labels error: {err}");
            }
        }
    }

    fn matches_log_filter_with(&self, needle: &str, entry: &LogEntry) -> bool {
        if needle.is_empty() {
            return true;
        }

        entry.line.to_lowercase().contains(needle)
    }

    fn update_log_status(&mut self) {
        let host = self.selected_log_host().unwrap_or("-");
        let log_name = self.selected_log_name().unwrap_or("-");
        let visible_logs = self.visible_log_entries().len();

        self.status = format!("streaming logs for {host} / {log_name} ({visible_logs} shown)");
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
        let previous = self.capture_target_state();
        info!(%base_url, "fetching targets from prometheus");
        let targets = fetch_targets(base_url, client)?;
        info!(%base_url, targets = targets.len(), "fetched targets from prometheus");
        self.target_options = targets;
        let restore = self.restore_target_state(previous);
        self.apply_target_restore(restore);
        self.load_selected_target_metrics(base_url, client)
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
        self.target_notice = None;
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
        let previous = self.capture_target_state();
        let filters = self.all_metrics.iter().filter_map(|metric| {
            let job = label_value(metric, "job")?.to_owned();
            let target = label_value(metric, "instance")
                .or_else(|| label_value(metric, "target"))
                .map(str::to_owned)
                .unwrap_or_else(|| String::from("-"));
            Some(TargetFilter { job, target })
        });
        self.target_options = build_target_options(filters);
        let restore = self.restore_target_state(previous);
        self.apply_target_restore(restore);
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
        let mut status = format!(
            "loaded {} metrics for {}",
            self.metrics.len(),
            self.selected_target()
        );
        if let Some(notice) = self.target_notice.take() {
            status.push_str(" (");
            status.push_str(&notice);
            status.push(')');
        }
        self.status = status;
    }

    fn capture_target_state(&self) -> TargetStateSnapshot {
        TargetStateSnapshot {
            selected: self.target_options.get(self.target_selected).cloned(),
            cursor: self
                .target_picker_open
                .then(|| self.target_options.get(self.target_cursor).cloned())
                .flatten(),
        }
    }

    fn restore_target_state(&mut self, previous: TargetStateSnapshot) -> TargetStateRestore {
        let mut result = TargetStateRestore::default();

        self.target_selected = match previous.selected {
            Some(current) => match self.target_options.iter().position(|item| item == &current) {
                Some(index) => index,
                None => {
                    result.selected_missing = true;
                    0
                }
            },
            None => 0,
        };

        self.target_cursor = match previous.cursor {
            Some(current) => match self.target_options.iter().position(|item| item == &current) {
                Some(index) => index,
                None => {
                    result.cursor_missing = true;
                    self.target_selected
                }
            },
            None => self.target_selected,
        };

        result
    }

    fn apply_target_restore(&mut self, restore: TargetStateRestore) {
        self.target_notice = if restore.selected_missing && restore.cursor_missing {
            Some(String::from(
                "previous target selection and picker cursor are no longer available",
            ))
        } else if restore.selected_missing {
            Some(String::from(
                "previous target selection is no longer available",
            ))
        } else if restore.cursor_missing {
            Some(String::from(
                "previous picker cursor target is no longer available",
            ))
        } else {
            None
        };
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

impl LogFocus {
    pub fn label(self) -> &'static str {
        match self {
            LogFocus::Hosts => "hosts",
            LogFocus::Logs => "logs",
            LogFocus::Tail => "tail",
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

fn advance_wrapped_index(index: &mut usize, len: usize, step: isize) -> bool {
    if len == 0 {
        return false;
    }

    let len = len as isize;
    let next = ((*index as isize) + step).rem_euclid(len) as usize;
    *index = next;
    true
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
    use super::{App, HistoryView, LogFocus, TargetFilter, HISTORY_LIMIT};
    use crate::config::{DisplayConfig, PrometheusConfig};
    use crate::loki::LogEntry;

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
    fn keeps_target_picker_open_and_cursor_position_after_reload() {
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

        app.reload_from_str(
            "up{job=\"api\",instance=\"a:9090\"} 2\nup{job=\"api\",instance=\"b:9090\"} 2\n",
        );

        assert!(app.target_picker_open);
        assert_eq!(app.selected_target(), &TargetFilter::wildcard());
        assert_eq!(app.target_options[app.target_cursor].target, "b:9090");
    }

    #[test]
    fn falls_back_when_picker_cursor_target_disappears_after_reload() {
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

        app.reload_from_str("up{job=\"api\",instance=\"a:9090\"} 2\n");

        assert!(app.target_picker_open);
        assert_eq!(app.selected_target(), &TargetFilter::wildcard());
        assert_eq!(app.target_cursor, app.target_selected);
        assert_eq!(app.status, "loaded 1 metrics for all targets (previous picker cursor target is no longer available)");
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
    fn filters_log_entries_by_text_query() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());
        app.log_entries = vec![
            LogEntry {
                timestamp_ns: 1,
                line: String::from("tailscaled: upload failed 429"),
            },
            LogEntry {
                timestamp_ns: 2,
                line: String::from("sshd: accepted publickey"),
            },
        ];

        app.push_log_filter_char('4');
        app.push_log_filter_char('2');
        app.push_log_filter_char('9');

        let visible = app.visible_log_entries();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].line, "tailscaled: upload failed 429");

        app.clear_log_filter();
        assert_eq!(app.visible_log_entries().len(), 2);
    }

    #[test]
    fn keeps_log_filter_applied_when_new_entries_arrive() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());
        app.log_entries = vec![LogEntry {
            timestamp_ns: 1,
            line: String::from("tailscaled: upload failed 429"),
        }];
        app.push_log_filter_char('t');
        app.push_log_filter_char('a');
        app.push_log_filter_char('i');
        app.push_log_filter_char('l');

        app.log_entries.push(LogEntry {
            timestamp_ns: 2,
            line: String::from("kernel: link is up"),
        });

        let visible = app.visible_log_entries();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].line, "tailscaled: upload failed 429");
    }

    #[test]
    fn cycles_log_focus_across_tail_pane() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());

        assert_eq!(app.log_focus, LogFocus::Hosts);

        app.next_log_focus();
        assert_eq!(app.log_focus, LogFocus::Logs);

        app.next_log_focus();
        assert_eq!(app.log_focus, LogFocus::Tail);

        app.previous_log_focus();
        assert_eq!(app.log_focus, LogFocus::Logs);
    }

    #[test]
    fn scrolls_tail_offset_and_clamps_to_visible_logs() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());
        app.log_entries = vec![
            LogEntry {
                timestamp_ns: 1,
                line: String::from("line 1"),
            },
            LogEntry {
                timestamp_ns: 2,
                line: String::from("line 2"),
            },
            LogEntry {
                timestamp_ns: 3,
                line: String::from("line 3"),
            },
        ];

        app.scroll_log_tail_up(1);
        assert_eq!(app.log_tail_scroll_offset(), 1);

        app.scroll_log_tail_up(10);
        assert_eq!(app.log_tail_scroll_offset(), 2);

        app.scroll_log_tail_down(1);
        assert_eq!(app.log_tail_scroll_offset(), 1);

        app.push_log_filter_char('3');
        assert_eq!(app.log_tail_scroll_offset(), 0);

        app.clear_log_filter();
        app.scroll_log_tail_to_oldest();
        assert_eq!(app.log_tail_scroll_offset(), 2);

        app.scroll_log_tail_to_latest();
        assert_eq!(app.log_tail_scroll_offset(), 0);
    }

    #[test]
    fn preserves_tail_scroll_position_when_new_matching_entries_arrive() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());
        app.log_entries = vec![
            LogEntry {
                timestamp_ns: 1,
                line: String::from("match 1"),
            },
            LogEntry {
                timestamp_ns: 2,
                line: String::from("match 2"),
            },
            LogEntry {
                timestamp_ns: 3,
                line: String::from("match 3"),
            },
        ];

        app.scroll_log_tail_up(1);
        app.append_log_entries(vec![LogEntry {
            timestamp_ns: 4,
            line: String::from("match 4"),
        }]);
        assert_eq!(app.log_tail_scroll_offset(), 2);

        app.push_log_filter_char('1');
        app.scroll_log_tail_to_latest();
        app.append_log_entries(vec![LogEntry {
            timestamp_ns: 5,
            line: String::from("other"),
        }]);
        assert_eq!(app.log_tail_scroll_offset(), 0);
    }

    #[test]
    fn preserves_loaded_logs_per_host_and_log_selection() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());
        app.log_hosts = vec![String::from("host-a"), String::from("host-b")];
        app.log_names = vec![String::from("kernel"), String::from("tailscaled")];

        app.log_entries = vec![
            LogEntry {
                timestamp_ns: 1,
                line: String::from("host-a kernel line 1"),
            },
            LogEntry {
                timestamp_ns: 2,
                line: String::from("host-a kernel line 2"),
            },
        ];
        app.last_log_timestamp_ns = Some(2);
        app.log_tail_offset = 1;
        app.store_current_log_stream();

        app.log_focus = LogFocus::Logs;
        app.next_log_option();
        assert!(app.log_entries.is_empty());
        assert_eq!(app.last_log_timestamp_ns, None);
        assert_eq!(app.log_tail_scroll_offset(), 0);

        app.log_entries = vec![LogEntry {
            timestamp_ns: 3,
            line: String::from("host-a tailscaled line"),
        }];
        app.last_log_timestamp_ns = Some(3);
        app.log_tail_offset = 0;
        app.store_current_log_stream();

        app.log_focus = LogFocus::Hosts;
        app.next_log_option();
        assert!(app.log_entries.is_empty());
        assert_eq!(app.last_log_timestamp_ns, None);
        assert_eq!(app.log_tail_scroll_offset(), 0);

        app.log_entries = vec![
            LogEntry {
                timestamp_ns: 4,
                line: String::from("host-b tailscaled line 1"),
            },
            LogEntry {
                timestamp_ns: 5,
                line: String::from("host-b tailscaled line 2"),
            },
            LogEntry {
                timestamp_ns: 6,
                line: String::from("host-b tailscaled line 3"),
            },
        ];
        app.last_log_timestamp_ns = Some(6);
        app.log_tail_offset = 2;
        app.store_current_log_stream();

        app.previous_log_option();
        assert_eq!(app.selected_log_host(), Some("host-a"));
        assert_eq!(app.selected_log_name(), Some("tailscaled"));
        assert_eq!(
            app.log_entries,
            vec![LogEntry {
                timestamp_ns: 3,
                line: String::from("host-a tailscaled line"),
            }]
        );
        assert_eq!(app.last_log_timestamp_ns, Some(3));
        assert_eq!(app.log_tail_scroll_offset(), 0);

        app.log_focus = LogFocus::Logs;
        app.previous_log_option();
        assert_eq!(app.selected_log_host(), Some("host-a"));
        assert_eq!(app.selected_log_name(), Some("kernel"));
        assert_eq!(
            app.log_entries,
            vec![
                LogEntry {
                    timestamp_ns: 1,
                    line: String::from("host-a kernel line 1"),
                },
                LogEntry {
                    timestamp_ns: 2,
                    line: String::from("host-a kernel line 2"),
                },
            ]
        );
        assert_eq!(app.last_log_timestamp_ns, Some(2));
        assert_eq!(app.log_tail_scroll_offset(), 1);
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
