use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Clear, Dataset, GraphType, List, ListItem, Paragraph,
        Row, Table,
    },
    Frame,
};

use crate::{
    app::{App, HistoryView, LogFocus, ScreenMode},
    prometheus::MetricSample,
};

pub fn render(frame: &mut Frame, app: &App) {
    if app.screen == ScreenMode::Logs {
        render_logs_screen(frame, app);
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Cranberry",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  source: "),
            Span::styled(app.source_label.as_str(), Style::default().fg(Color::Green)),
            Span::raw("  target: "),
            Span::styled(
                app.selected_target().to_string(),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  refresh: "),
            Span::styled(
                format!("{}s", app.refresh_secs()),
                Style::default().fg(Color::Blue),
            ),
            Span::raw("  state: "),
            Span::styled(
                app.refresh_state_label(),
                Style::default().fg(Color::LightGreen),
            ),
        ]),
        Line::from(vec![
            Span::raw("filter: "),
            Span::styled(
                if app.filter_query.is_empty() {
                    "*"
                } else {
                    app.filter_query.as_str()
                },
                Style::default().fg(Color::Magenta),
            ),
            Span::raw("  metrics: "),
            Span::styled(
                app.metrics.len().to_string(),
                Style::default().fg(Color::LightRed),
            ),
            Span::raw("  selected: "),
            Span::styled(
                app.selected_metrics_len().to_string(),
                Style::default().fg(Color::LightGreen),
            ),
            Span::raw("  range: "),
            Span::styled(app.history_range_label(), Style::default().fg(Color::Blue)),
            Span::raw("  dashboard: "),
            Span::styled(app.dashboard_label(), Style::default().fg(Color::LightCyan)),
        ]),
    ])
    .block(Block::default().borders(Borders::ALL).title("Overview"));

    frame.render_widget(header, vertical[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(vertical[1]);

    let metric_height = body[0].height.saturating_sub(2) as usize;
    let metric_start = window_start(app.cursor, app.metrics.len(), metric_height);
    let items = visible_list_items(
        &app.metrics,
        metric_start,
        metric_height,
        app.cursor,
        |metric| {
            let marker = if app.is_metric_selected(metric) {
                "[x]"
            } else {
                "[ ]"
            };
            format!("{marker} {} = {:.3}", metric.name, metric.value)
        },
    );

    let metric_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Metrics"))
        .highlight_style(Style::default().fg(Color::Yellow));

    frame.render_widget(metric_list, body[0]);

    let detail_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(body[1]);

    let detail_text = app
        .selected_metric()
        .map(format_metric_details)
        .unwrap_or_else(|| String::from("no metrics loaded"));

    let detail =
        Paragraph::new(detail_text).block(Block::default().borders(Borders::ALL).title("Details"));
    frame.render_widget(detail, detail_layout[0]);

    render_history(frame, app, detail_layout[1]);

    let footer = Paragraph::new(
        "q quit | j/k move | Space select | h graph/table | 1/2/3/4 live/1h/6h/24h | p PromQL | d preset | l logs | r reload",
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Help")
            .title_bottom(Line::from(app.status.as_str()).alignment(Alignment::Right)),
    );
    frame.render_widget(footer, vertical[2]);

    if app.target_picker_open {
        render_target_picker(frame, app);
    } else if app.filter_input_open {
        render_filter_input(frame, app);
    } else if app.promql_input_open {
        render_promql_input(frame, app);
    }
}

fn render_logs_screen(frame: &mut Frame, app: &App) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "Cranberry Logs",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  host label: "),
        Span::styled(
            app.selected_log_host().unwrap_or("-"),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  log label: "),
        Span::styled(
            app.selected_log_name().unwrap_or("-"),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  filter: "),
        Span::styled(
            if app.log_filter_query.is_empty() {
                "*"
            } else {
                app.log_filter_query.as_str()
            },
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("  poll: "),
        Span::styled(
            format!("{}s", app.log_poll_secs()),
            Style::default().fg(Color::Blue),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Logs"));
    frame.render_widget(header, vertical[0]);

    let pickers = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical[1]);
    render_string_list(
        frame,
        pickers[0],
        "Hosts",
        &app.log_hosts,
        app.log_host_selected,
        app.log_focus == LogFocus::Hosts,
    );
    render_string_list(
        frame,
        pickers[1],
        "Logs",
        &app.log_names,
        app.log_name_selected,
        app.log_focus == LogFocus::Logs,
    );

    let filtered_entries = app.visible_log_entries();
    let visible = vertical[2].height.saturating_sub(2) as usize;
    let start = window_start(
        app.log_tail_cursor(),
        filtered_entries.len(),
        visible.max(1),
    );
    let lines: Vec<Line> = filtered_entries
        .iter()
        .skip(start)
        .take(visible.max(1))
        .enumerate()
        .map(|(offset, entry)| {
            let timestamp = format_log_timestamp(entry.timestamp_ns);
            let is_cursor = start + offset == app.log_tail_cursor();
            let marker = if app.is_log_entry_selected(entry) {
                "[x]"
            } else {
                "[ ]"
            };
            let text_style = if is_cursor {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(
                    if is_cursor { "> " } else { "  " },
                    text_style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    marker,
                    if app.is_log_entry_selected(entry) {
                        text_style.fg(Color::LightGreen)
                    } else {
                        text_style
                    },
                ),
                Span::raw(" "),
                Span::styled(timestamp, text_style.fg(Color::Blue)),
                Span::raw(" "),
                Span::styled(
                    sanitize_log_line_for_display(entry.line.as_str()),
                    text_style,
                ),
            ])
        })
        .collect();
    let log_body = if lines.is_empty() {
        Paragraph::new(if app.log_entries.is_empty() {
            "no log lines yet"
        } else {
            "no matching log lines"
        })
    } else {
        Paragraph::new(lines)
    }
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if app.log_focus == LogFocus::Tail {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            })
            .title("Tail"),
    );
    frame.render_widget(log_body, vertical[2]);

    let footer = Paragraph::new(
        "Esc back | Tab/Shift-Tab or h/l switch pane | j/k move cursor | Space mark | c clear marks | PgUp/PgDn/Home/End tail | / filter | r reload labels/logs | q quit",
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Help")
            .title_bottom(Line::from(app.status.as_str()).alignment(Alignment::Right)),
    );
    frame.render_widget(footer, vertical[3]);

    if app.log_filter_input_open {
        render_log_filter_input(frame, app);
    }
}

fn render_target_picker(frame: &mut Frame, app: &App) {
    let area = centered_rect(frame.area(), 60, 60);
    frame.render_widget(Clear, area);

    let visible = area.height.saturating_sub(2) as usize;
    let start = window_start(app.target_cursor, app.target_options.len(), visible);
    let items = visible_list_items(
        &app.target_options,
        start,
        visible,
        app.target_cursor,
        |target| target.to_string(),
    );

    let picker = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Target Picker")
            .title_bottom(
                Line::from("Enter apply | Esc close | j/k move").alignment(Alignment::Center),
            ),
    );

    frame.render_widget(picker, area);
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical: [Rect; 1] = Layout::vertical([Constraint::Percentage(height_percent)])
        .flex(Flex::Center)
        .areas(area);
    let horizontal: [Rect; 1] = Layout::horizontal([Constraint::Percentage(width_percent)])
        .flex(Flex::Center)
        .areas(vertical[0]);
    horizontal[0]
}

fn render_filter_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(frame.area(), 60, 20);
    frame.render_widget(Clear, area);

    let prompt = Paragraph::new(format!(
        "Filter metrics\n\n{}\n\nEnter apply | Esc close | Backspace delete | Ctrl-U clear",
        app.filter_query
    ))
    .block(Block::default().borders(Borders::ALL).title("Filter"));

    frame.render_widget(prompt, area);
}

fn render_log_filter_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(frame.area(), 60, 20);
    frame.render_widget(Clear, area);

    let prompt = Paragraph::new(format!(
        "Filter logs\n\n{}\n\nEnter apply | Esc close | Backspace delete | Ctrl-U clear",
        app.log_filter_query
    ))
    .block(Block::default().borders(Borders::ALL).title("Filter"));

    frame.render_widget(prompt, area);
}

fn render_promql_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(frame.area(), 80, 25);
    frame.render_widget(Clear, area);
    let prompt = Paragraph::new(format!(
        "PromQL\n\n{}\n\nEnter execute | Esc cancel | Backspace delete | Ctrl-U clear",
        app.promql_query
    ))
    .block(Block::default().borders(Borders::ALL).title("PromQL Query"));
    frame.render_widget(prompt, area);
}

fn render_history(frame: &mut Frame, app: &App, area: Rect) {
    match app.history_view {
        HistoryView::Graph => render_history_chart(frame, app, area),
        HistoryView::Table => render_history_table(frame, app, area),
    }
}

fn render_history_chart(frame: &mut Frame, app: &App, area: Rect) {
    let series = app.displayed_metric_histories();
    if series.is_empty() || series.iter().all(|(_, history)| history.is_empty()) {
        render_empty_history(
            frame,
            area,
            &history_title("History (Graph)", selected_metric_unit(app)),
        );
        return;
    }
    let unit = common_metric_unit(&series);

    let min = series
        .iter()
        .flat_map(|(_, history)| history.iter().copied())
        .fold(f64::INFINITY, f64::min);
    let max = series
        .iter()
        .flat_map(|(_, history)| history.iter().copied())
        .fold(f64::NEG_INFINITY, f64::max);
    let (min, max) = if (max - min).abs() < f64::EPSILON {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    };

    let point_sets: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|(_, history)| {
            history
                .iter()
                .enumerate()
                .map(|(index, value)| (index as f64, *value))
                .collect()
        })
        .collect();
    let x_max = point_sets
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(1)
        .saturating_sub(1) as f64;
    let colors = [
        Color::Cyan,
        Color::Yellow,
        Color::LightGreen,
        Color::Magenta,
        Color::LightBlue,
        Color::LightRed,
    ];
    let datasets: Vec<Dataset> = series
        .iter()
        .zip(point_sets.iter())
        .enumerate()
        .map(|(index, ((metric, _), points))| {
            Dataset::default()
                .name(compact_series_label(metric))
                .graph_type(GraphType::Line)
                .style(Style::default().fg(colors[index % colors.len()]))
                .data(points)
        })
        .collect();

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(history_title("History (Graph)", unit)),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max.max(1.0)])
                .labels(vec![Line::from("old"), Line::from("now")]),
        )
        .y_axis(Axis::default().bounds([min, max]).labels(vec![
            Line::from(format_history_value(min, unit)),
            Line::from(format_history_value(max, unit)),
        ]));

    frame.render_widget(chart, area);
}

fn render_history_table(frame: &mut Frame, app: &App, area: Rect) {
    let series = app.displayed_metric_histories();
    if series.len() > 1 {
        render_history_summary_table(frame, area, &series);
        return;
    }
    let history = app.selected_metric_history();
    let unit = selected_metric_unit(app);
    if history.is_empty() {
        render_empty_history(frame, area, &history_title("History (Table)", unit));
        return;
    }

    let visible_rows = area.height.saturating_sub(3) as usize;
    let rows: Vec<Row> = history_table_rows(&history, visible_rows, unit)
        .into_iter()
        .map(|(point, value, delta)| {
            Row::new(vec![
                Cell::from(point),
                Cell::from(value),
                Cell::from(delta),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Min(12),
            Constraint::Min(12),
        ],
    )
    .header(
        Row::new(vec![
            String::from("Sample"),
            history_column_title("Value", unit),
            history_column_title("Delta", unit),
        ])
        .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(history_title("History (Table)", unit)),
    )
    .column_spacing(1);

    frame.render_widget(table, area);
}

fn render_history_summary_table(
    frame: &mut Frame,
    area: Rect,
    series: &[(MetricSample, Vec<f64>)],
) {
    let rows = series.iter().map(|(metric, history)| {
        let latest = history.last().copied();
        let delta = history.iter().rev().take(2).copied().collect::<Vec<_>>();
        let min = history.iter().copied().reduce(f64::min);
        let max = history.iter().copied().reduce(f64::max);
        Row::new(vec![
            Cell::from(metric_series_label(metric)),
            Cell::from(format_optional_value(latest)),
            Cell::from(if delta.len() == 2 {
                format!("{:+.3}", delta[0] - delta[1])
            } else {
                String::from("n/a")
            }),
            Cell::from(format_optional_value(min)),
            Cell::from(format_optional_value(max)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ],
    )
    .header(
        Row::new(["Series", "Latest", "Delta", "Min", "Max"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("History (Comparison)"),
    );
    frame.render_widget(table, area);
}

fn format_optional_value(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| String::from("n/a"))
}

fn metric_series_label(metric: &MetricSample) -> String {
    let identity = metric
        .labels
        .iter()
        .find(|(key, _)| matches!(key.as_str(), "instance" | "target" | "job"))
        .map(|(_, value)| value.as_str());
    match identity {
        Some(identity) => format!("{} [{}]", metric.name, identity),
        None => metric.name.clone(),
    }
}

fn compact_series_label(metric: &MetricSample) -> String {
    let label = metric_series_label(metric);
    const MAX_CHARS: usize = 28;
    if label.chars().count() <= MAX_CHARS {
        return label;
    }
    let mut compact: String = label.chars().take(MAX_CHARS - 1).collect();
    compact.push('…');
    compact
}

fn common_metric_unit(series: &[(MetricSample, Vec<f64>)]) -> Option<&str> {
    let first = series
        .first()
        .and_then(|(metric, _)| metric_unit_from_name(&metric.name));
    series
        .iter()
        .all(|(metric, _)| metric_unit_from_name(&metric.name) == first)
        .then_some(first)
        .flatten()
}

fn history_table_rows(
    history: &[f64],
    visible_rows: usize,
    unit: Option<&str>,
) -> Vec<(String, String, String)> {
    history
        .iter()
        .enumerate()
        .rev()
        .take(visible_rows.max(1))
        .map(|(index, value)| {
            let age = history.len().saturating_sub(index + 1);
            let point = if age == 0 {
                String::from("now")
            } else {
                format!("-{age}")
            };
            let delta = if index == 0 {
                String::from("n/a")
            } else {
                format_delta(history[index] - history[index - 1], unit)
            };
            (point, format_history_value(*value, unit), delta)
        })
        .collect()
}

fn selected_metric_unit(app: &App) -> Option<&str> {
    app.selected_metric()
        .and_then(|metric| metric_unit_from_name(&metric.name))
}

fn metric_unit_from_name(name: &str) -> Option<&str> {
    let mut parts: Vec<&str> = name.split('_').filter(|part| !part.is_empty()).collect();
    while matches!(
        parts.last().copied(),
        Some("total" | "sum" | "count" | "bucket" | "info" | "created")
    ) {
        parts.pop();
    }

    if parts.len() < 2 {
        None
    } else {
        match parts.last().copied() {
            Some(
                unit @ ("seconds" | "bytes" | "meters" | "celsius" | "volts" | "amperes" | "joules"
                | "grams" | "ratio"),
            ) => Some(unit),
            _ => None,
        }
    }
}

fn history_title(base: &str, unit: Option<&str>) -> String {
    match unit {
        Some(unit) => format!("{base} [{unit}]"),
        None => base.to_string(),
    }
}

fn history_column_title(base: &str, unit: Option<&str>) -> String {
    match unit {
        Some(unit) => format!("{base} ({unit})"),
        None => base.to_string(),
    }
}

fn format_history_value(value: f64, unit: Option<&str>) -> String {
    match unit {
        Some(unit) => format!("{value:.3} {unit}"),
        None => format!("{value:.3}"),
    }
}

fn format_delta(delta: f64, unit: Option<&str>) -> String {
    match unit {
        Some(unit) => format!("{delta:+.3} {unit}"),
        None => format!("{delta:+.3}"),
    }
}

fn window_start(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        return 0;
    }

    let max_start = len.saturating_sub(visible);
    selected
        .saturating_sub(visible.saturating_sub(1))
        .min(max_start)
}

fn visible_list_items<T, F>(
    items: &[T],
    start: usize,
    visible: usize,
    selected: usize,
    render_item: F,
) -> Vec<ListItem<'_>>
where
    F: Fn(&T) -> String,
{
    items
        .iter()
        .skip(start)
        .take(visible)
        .enumerate()
        .map(|(offset, item)| {
            let prefix = if start + offset == selected {
                "> "
            } else {
                "  "
            };
            ListItem::new(format!("{prefix}{}", render_item(item)))
        })
        .collect()
}

fn render_string_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: &[String],
    selected: usize,
    focused: bool,
) {
    let visible = area.height.saturating_sub(2) as usize;
    let start = window_start(selected, items.len(), visible);
    let rendered = visible_list_items(items, start, visible, selected, |item| item.clone());
    let border_style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let list = List::new(rendered).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title),
    );
    frame.render_widget(list, area);
}

fn format_log_timestamp(timestamp_ns: i64) -> String {
    Local
        .timestamp_nanos(timestamp_ns)
        .format("%H:%M:%S%.3f")
        .to_string()
}

fn sanitize_log_line_for_display(line: &str) -> String {
    let segment = line
        .split('\r')
        .rev()
        .find(|segment| !segment.is_empty())
        .unwrap_or("");
    strip_ansi_and_control_chars(segment)
}

fn strip_ansi_and_control_chars(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.next_if_eq(&'[').is_some() {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }

        match ch {
            '\n' | '\t' => sanitized.push(' '),
            ch if ch.is_control() => {}
            _ => sanitized.push(ch),
        }
    }

    sanitized
}

fn format_metric_details(metric: &MetricSample) -> String {
    format!(
        "name: {}\nvalue: {}\n\nlabels:\n{}",
        metric.name,
        metric.value,
        format_metric_labels(metric)
    )
}

fn format_metric_labels(metric: &MetricSample) -> String {
    if metric.labels.is_empty() {
        String::from("none")
    } else {
        metric
            .labels
            .iter()
            .map(|(key, value)| format!("{key}=\"{value}\""))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn render_empty_history(frame: &mut Frame, area: Rect, title: &str) {
    let empty =
        Paragraph::new("no history yet").block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(empty, area);
}

#[cfg(test)]
mod tests {
    use super::{history_table_rows, metric_unit_from_name, render, sanitize_log_line_for_display};
    use crate::{
        app::App,
        config::{DisplayConfig, PrometheusConfig},
    };
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_selected_metric_series_in_comparison_chart() {
        let mut app = App::new(PrometheusConfig::default(), DisplayConfig::default());
        app.cursor = 0;
        app.toggle_metric_selection();
        app.cursor = 1;
        app.toggle_metric_selection();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal should be created");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("dashboard should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("selected: 2"));
        assert!(rendered.contains("History (Graph)"));
    }

    #[test]
    fn history_table_rows_show_latest_first() {
        let rows = history_table_rows(&[70.0, 71.0, 69.0], 10, None);

        assert_eq!(
            rows,
            vec![
                (
                    String::from("now"),
                    String::from("69.000"),
                    String::from("-2.000"),
                ),
                (
                    String::from("-1"),
                    String::from("71.000"),
                    String::from("+1.000"),
                ),
                (
                    String::from("-2"),
                    String::from("70.000"),
                    String::from("n/a"),
                ),
            ]
        );
    }

    #[test]
    fn history_table_rows_respect_visible_limit() {
        let rows = history_table_rows(&[70.0, 71.0, 69.0], 2, None);

        assert_eq!(
            rows,
            vec![
                (
                    String::from("now"),
                    String::from("69.000"),
                    String::from("-2.000"),
                ),
                (
                    String::from("-1"),
                    String::from("71.000"),
                    String::from("+1.000"),
                ),
            ]
        );
    }

    #[test]
    fn sanitize_log_line_keeps_latest_carriage_return_segment() {
        let line = "  0%|          | 0/20 [00:00<?, ?it/s]\r 35%|███▌      | 7/20 [00:37<01:27,  6.75s/it]";

        assert_eq!(
            sanitize_log_line_for_display(line),
            " 35%|███▌      | 7/20 [00:37<01:27,  6.75s/it]"
        );
    }

    #[test]
    fn sanitize_log_line_strips_ansi_and_control_characters() {
        let line = "\u{1b}[32mready\u{1b}[0m\tstatus\nok\u{7}";

        assert_eq!(sanitize_log_line_for_display(line), "ready status ok");
    }

    #[test]
    fn extracts_metric_units_from_prometheus_names() {
        assert_eq!(
            metric_unit_from_name("process_cpu_seconds_total"),
            Some("seconds")
        );
        assert_eq!(
            metric_unit_from_name("process_resident_memory_bytes"),
            Some("bytes")
        );
        assert_eq!(
            metric_unit_from_name("http_request_duration_seconds_bucket"),
            Some("seconds")
        );
        assert_eq!(metric_unit_from_name("http_requests_total"), None);
        assert_eq!(metric_unit_from_name("up"), None);
    }

    #[test]
    fn history_table_rows_include_units_in_values_and_deltas() {
        let rows = history_table_rows(&[10.0, 12.5], 10, Some("seconds"));

        assert_eq!(
            rows,
            vec![
                (
                    String::from("now"),
                    String::from("12.500 seconds"),
                    String::from("+2.500 seconds"),
                ),
                (
                    String::from("-1"),
                    String::from("10.000 seconds"),
                    String::from("n/a"),
                ),
            ]
        );
    }
}
