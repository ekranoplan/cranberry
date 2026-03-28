use crate::config::LokiConfig;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::error;

const REQUEST_TIMEOUT_SECS: u64 = 5;
const QUERY_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp_ns: i64,
    pub line: String,
}

#[derive(Clone, Debug)]
pub struct LokiClient {
    base_url: String,
    host_label: String,
    log_label: String,
    lookback_secs: u64,
    client: reqwest::blocking::Client,
}

impl LokiClient {
    pub fn new(config: LokiConfig) -> Option<Self> {
        let base_url = config.base_url?;
        Some(Self {
            base_url,
            host_label: config.host_label,
            log_label: config.log_label,
            lookback_secs: config.lookback_secs,
            client: reqwest::blocking::Client::new(),
        })
    }

    pub fn fetch_hosts(&self) -> Result<Vec<String>, String> {
        self.fetch_label_values(&self.host_label)
    }

    pub fn fetch_logs(&self) -> Result<Vec<String>, String> {
        self.fetch_label_values(&self.log_label)
    }

    pub fn poll_logs(
        &self,
        host: &str,
        log_name: &str,
        since_ns: Option<i64>,
    ) -> Result<Vec<LogEntry>, String> {
        let end_ns = now_unix_nanos()?;
        let start_ns = since_ns
            .map(|value| value.saturating_add(1))
            .unwrap_or_else(|| end_ns.saturating_sub((self.lookback_secs as i64) * 1_000_000_000));
        let query = format!(
            "{{{}=\"{}\",{}=\"{}\"}}",
            self.host_label,
            escape_label_value(host),
            self.log_label,
            escape_label_value(log_name)
        );
        let response = self
            .client
            .get(format!(
                "{}/loki/api/v1/query_range",
                self.base_url.trim_end_matches('/')
            ))
            .query(&[
                ("query", query.as_str()),
                ("start", &start_ns.to_string()),
                ("end", &end_ns.to_string()),
                ("limit", &QUERY_LIMIT.to_string()),
                ("direction", "forward"),
            ])
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .send()
            .map_err(|err| {
                error!(base_url = %self.base_url, error = %err, "loki log request failed");
                err.to_string()
            })?;
        let response = response.error_for_status().map_err(|err| {
            error!(base_url = %self.base_url, error = %err, "loki log response returned error status");
            err.to_string()
        })?;
        let payload: QueryRangeResponse = response.json().map_err(|err| {
            error!(base_url = %self.base_url, error = %err, "failed to decode loki log response");
            err.to_string()
        })?;

        if payload.status != "success" {
            return Err(format!("query_range status {}", payload.status));
        }

        let data = payload
            .data
            .ok_or_else(|| String::from("missing response data"))?;
        if data.result_type != "streams" {
            return Err(format!("unsupported result type {}", data.result_type));
        }

        let mut entries = Vec::new();
        for stream in data.result {
            let _labels = stream.stream;
            for value in stream.values {
                let timestamp_ns = value
                    .0
                    .parse::<i64>()
                    .map_err(|_| String::from("invalid log timestamp"))?;
                entries.push(LogEntry {
                    timestamp_ns,
                    line: value.1,
                });
            }
        }
        entries.sort_by(|left, right| left.timestamp_ns.cmp(&right.timestamp_ns));
        Ok(entries)
    }

    fn fetch_label_values(&self, label: &str) -> Result<Vec<String>, String> {
        let end_ns = now_unix_nanos()?;
        let start_ns = end_ns.saturating_sub((self.lookback_secs as i64) * 1_000_000_000);
        let response = self
            .client
            .get(format!(
                "{}/loki/api/v1/label/{label}/values",
                self.base_url.trim_end_matches('/')
            ))
            .query(&[("start", start_ns.to_string()), ("end", end_ns.to_string())])
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .send()
            .map_err(|err| {
                error!(base_url = %self.base_url, label, error = %err, "loki label request failed");
                err.to_string()
            })?;
        let response = response.error_for_status().map_err(|err| {
            error!(base_url = %self.base_url, label, error = %err, "loki label response returned error status");
            err.to_string()
        })?;
        let payload: LabelValuesResponse = response.json().map_err(|err| {
            error!(base_url = %self.base_url, label, error = %err, "failed to decode loki label response");
            err.to_string()
        })?;

        if payload.status != "success" {
            return Err(format!("label status {}", payload.status));
        }

        let mut values = payload.data;
        values.retain(|value| !value.is_empty());
        values.sort();
        values.dedup();
        Ok(values)
    }
}

fn now_unix_nanos() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?;
    i64::try_from(duration.as_nanos()).map_err(|_| String::from("system clock overflow"))
}

fn escape_label_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug, Deserialize)]
struct LabelValuesResponse {
    status: String,
    data: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct QueryRangeResponse {
    status: String,
    data: Option<QueryRangeData>,
}

#[derive(Debug, Deserialize)]
struct QueryRangeData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<LogStream>,
}

#[derive(Debug, Deserialize)]
struct LogStream {
    stream: BTreeMap<String, String>,
    values: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::escape_label_value;

    #[test]
    fn escapes_quotes_and_backslashes_in_label_values() {
        assert_eq!(escape_label_value(r#"api\"test"#), r#"api\\\"test"#);
    }
}
