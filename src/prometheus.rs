use serde::Deserialize;
use std::collections::{BTreeMap, VecDeque};
use tracing::error;

#[derive(Clone, Debug)]
pub struct MetricSample {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

pub type MetricKey = (String, Vec<(String, String)>);

#[derive(Clone, Debug)]
pub struct PrometheusTarget {
    pub job: String,
    pub target: String,
}

pub fn metric_key(metric: &MetricSample) -> MetricKey {
    (metric.name.clone(), metric.labels.clone())
}

pub fn metric_selector(metric: &MetricSample) -> String {
    if metric.labels.is_empty() {
        return metric.name.clone();
    }
    let labels = metric
        .labels
        .iter()
        .map(|(key, value)| format!(r#"{key}="{}""#, escape_matcher(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{}{{{labels}}}", metric.name)
}

pub fn fetch_targets(
    base_url: &str,
    client: &reqwest::blocking::Client,
) -> Result<Vec<PrometheusTarget>, String> {
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

    Ok(payload
        .data
        .into_iter()
        .filter_map(|series| {
            Some(PrometheusTarget {
                job: series.get("job")?.clone(),
                target: series.get("instance")?.clone(),
            })
        })
        .collect())
}

pub fn fetch_target_metrics(
    base_url: &str,
    target: Option<(&str, &str)>,
    client: &reqwest::blocking::Client,
) -> Result<Vec<MetricSample>, String> {
    let expression = target_selector(target);
    fetch_instant_query(base_url, &expression, client)
}

fn target_selector(target: Option<(&str, &str)>) -> String {
    match target {
        Some((job, target)) => format!(
            "{{job=\"{}\",instance=\"{}\"}}",
            escape_matcher(job),
            escape_matcher(target)
        ),
        None => String::from("{job!=\"\"}"),
    }
}

pub fn fetch_instant_query(
    base_url: &str,
    expression: &str,
    client: &reqwest::blocking::Client,
) -> Result<Vec<MetricSample>, String> {
    let response = client
        .get(format!("{}/api/v1/query", base_url.trim_end_matches('/')))
        .query(&[("query", expression)])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .map_err(|err| {
            error!(%base_url, query = %expression, error = %err, "prometheus metric request failed");
            err.to_string()
        })?;
    let response = response.error_for_status().map_err(|err| {
        error!(%base_url, query = %expression, error = %err, "prometheus metric response returned error status");
        err.to_string()
    })?;
    let payload: QueryResponse = response.json().map_err(|err| {
        error!(%base_url, error = %err, "failed to decode prometheus metric response");
        err.to_string()
    })?;

    if payload.status != "success" {
        return Err(format!("query status {}", payload.status));
    }
    let data = payload
        .data
        .ok_or_else(|| String::from("missing response data"))?;
    if data.result_type != "vector" {
        return Err(format!("unsupported result type {}", data.result_type));
    }
    data.result
        .into_iter()
        .map(QuerySample::into_metric_sample)
        .collect()
}

pub fn fetch_query_range(
    base_url: &str,
    query: &str,
    range_secs: u64,
    step_secs: u64,
    client: &reqwest::blocking::Client,
) -> Result<BTreeMap<MetricKey, VecDeque<f64>>, String> {
    let end = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    let start = end.saturating_sub(range_secs);
    let response = client
        .get(format!(
            "{}/api/v1/query_range",
            base_url.trim_end_matches('/')
        ))
        .query(&[
            ("query", query.to_owned()),
            ("start", start.to_string()),
            ("end", end.to_string()),
            ("step", step_secs.to_string()),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?;
    let payload: RangeQueryResponse = response.json().map_err(|err| err.to_string())?;
    if payload.status != "success" {
        return Err(format!("query_range status {}", payload.status));
    }
    let data = payload
        .data
        .ok_or_else(|| String::from("missing query_range data"))?;
    if data.result_type != "matrix" {
        return Err(format!(
            "unsupported range result type {}",
            data.result_type
        ));
    }

    data.result
        .into_iter()
        .map(|series| {
            let name = series
                .metric
                .get("__name__")
                .cloned()
                .unwrap_or_else(|| query.to_owned());
            let labels = series
                .metric
                .into_iter()
                .filter(|(key, _)| key != "__name__")
                .collect();
            let values = series
                .values
                .into_iter()
                .map(|(_, value)| {
                    value
                        .parse::<f64>()
                        .map_err(|_| String::from("invalid range sample value"))
                })
                .collect::<Result<VecDeque<_>, _>>()?;
            Ok(((name, labels), values))
        })
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
        let labels = self
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
struct RangeQueryResponse {
    status: String,
    data: Option<RangeQueryData>,
}

#[derive(Debug, Deserialize)]
struct RangeQueryData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<RangeQuerySeries>,
}

#[derive(Debug, Deserialize)]
struct RangeQuerySeries {
    metric: BTreeMap<String, String>,
    values: Vec<(f64, String)>,
}

#[derive(Debug, Deserialize)]
struct SeriesResponse {
    status: String,
    data: Vec<BTreeMap<String, String>>,
}

fn escape_matcher(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn parse_metrics(input: &str) -> Result<Vec<MetricSample>, String> {
    let mut samples = Vec::new();

    for (index, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.rsplitn(2, char::is_whitespace);
        let value_str = parts
            .next()
            .ok_or_else(|| format!("missing value at line {}", index + 1))?;
        let head = parts
            .next()
            .ok_or_else(|| format!("missing metric name at line {}", index + 1))?
            .trim();

        let value = value_str
            .parse::<f64>()
            .map_err(|_| format!("invalid value at line {}", index + 1))?;

        let (name, labels) =
            parse_head(head).map_err(|err| format!("{err} at line {}", index + 1))?;
        samples.push(MetricSample {
            name,
            labels,
            value,
        });
    }

    Ok(samples)
}

fn parse_head(head: &str) -> Result<(String, Vec<(String, String)>), String> {
    match head.split_once('{') {
        Some((name, rest)) => {
            let label_str = rest
                .strip_suffix('}')
                .ok_or_else(|| String::from("missing closing label brace"))?;
            let labels = parse_labels(label_str)?;
            Ok((name.to_string(), labels))
        }
        None => Ok((head.to_string(), Vec::new())),
    }
}

fn parse_labels(input: &str) -> Result<Vec<(String, String)>, String> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut labels = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut cursor = 0;

    while cursor < chars.len() {
        skip_delimiters(&chars, &mut cursor);
        if cursor >= chars.len() {
            break;
        }

        let key_start = cursor;
        while cursor < chars.len() && chars[cursor] != '=' {
            cursor += 1;
        }
        if cursor >= chars.len() {
            return Err(String::from("invalid label entry"));
        }

        let key: String = chars[key_start..cursor].iter().collect();
        cursor += 1;

        if cursor >= chars.len() || chars[cursor] != '"' {
            return Err(String::from("label value must be quoted"));
        }
        cursor += 1;

        let mut value = String::new();
        while cursor < chars.len() {
            match chars[cursor] {
                '\\' => {
                    cursor += 1;
                    if cursor >= chars.len() {
                        return Err(String::from("unfinished escape sequence"));
                    }
                    let escaped = match chars[cursor] {
                        'n' => '\n',
                        '\\' => '\\',
                        '"' => '"',
                        other => other,
                    };
                    value.push(escaped);
                    cursor += 1;
                }
                '"' => {
                    cursor += 1;
                    break;
                }
                ch => {
                    value.push(ch);
                    cursor += 1;
                }
            }
        }

        labels.push((key.trim().to_string(), value));
        skip_delimiters(&chars, &mut cursor);
    }

    Ok(labels)
}

fn skip_delimiters(chars: &[char], cursor: &mut usize) {
    while *cursor < chars.len() && (chars[*cursor] == ',' || chars[*cursor].is_whitespace()) {
        *cursor += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        metric_selector, parse_metrics, target_selector, MetricSample, RangeQueryResponse,
    };

    #[test]
    fn builds_target_selectors() {
        assert_eq!(target_selector(None), r#"{job!=""}"#);
        assert_eq!(
            target_selector(Some(("api", "node\"1"))),
            r#"{job="api",instance="node\"1"}"#
        );
    }

    #[test]
    fn builds_exact_promql_selector_for_metric() {
        let metric = MetricSample {
            name: String::from("up"),
            labels: vec![
                (String::from("job"), String::from("api")),
                (String::from("instance"), String::from("a\"b")),
            ],
            value: 1.0,
        };

        assert_eq!(metric_selector(&metric), r#"up{job="api",instance="a\"b"}"#);
    }

    #[test]
    fn parses_prometheus_matrix_response() {
        let payload: RangeQueryResponse = serde_json::from_str(
            r#"{
                "status":"success",
                "data":{"resultType":"matrix","result":[{
                    "metric":{"__name__":"up","job":"api"},
                    "values":[[1710000000.0,"1"],[1710000060.0,"0"]]
                }]}
            }"#,
        )
        .expect("matrix response should parse");

        let data = payload.data.expect("response data should exist");
        assert_eq!(data.result_type, "matrix");
        assert_eq!(data.result[0].values.len(), 2);
    }

    #[test]
    fn parses_exposition_lines() {
        let input = r#"
        # TYPE up gauge
        up{job="node",instance="localhost:9100"} 1
        process_cpu_seconds_total 2.5
        "#;

        let metrics = parse_metrics(input).expect("expected valid metrics");
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "up");
        assert_eq!(metrics[0].labels.len(), 2);
        assert_eq!(metrics[1].value, 2.5);
    }

    #[test]
    fn parses_quoted_label_values_with_commas() {
        let input = r#"http_requests_total{handler="/api/v1,internal",message="say \"hello\""} 3"#;

        let metrics = parse_metrics(input).expect("expected valid metrics");
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].labels[0].1, "/api/v1,internal");
        assert_eq!(metrics[0].labels[1].1, "say \"hello\"");
    }

    #[test]
    fn reports_line_numbers_for_invalid_values() {
        let err = parse_metrics("up 1\nrequests_total abc\n").expect_err("metric should fail");

        assert_eq!(err, "invalid value at line 2");
    }

    #[test]
    fn rejects_unclosed_label_blocks() {
        let err = parse_metrics("up{job=\"api\",instance=\"a:9090\" 1\n")
            .expect_err("metric should fail");

        assert_eq!(err, "missing closing label brace at line 1");
    }
}
