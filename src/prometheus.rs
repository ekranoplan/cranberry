#[derive(Clone, Debug)]
pub struct MetricSample {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
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

        let (name, labels) = parse_head(head).map_err(|err| format!("{err} at line {}", index + 1))?;
        samples.push(MetricSample { name, labels, value });
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
    use super::parse_metrics;

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
}
