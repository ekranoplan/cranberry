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
    app::{App, HistoryView},
    prometheus::MetricSample,
};

pub fn render(frame: &mut Frame, app: &App) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "Cranberry",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  source: "),
        Span::styled(app.source_label.as_str(), Style::default().fg(Color::Green)),
        Span::raw("  target: "),
        Span::styled(app.selected_target().to_string(), Style::default().fg(Color::Yellow)),
        Span::raw("  refresh: "),
        Span::styled(
            format!("{}s", app.refresh_secs()),
            Style::default().fg(Color::Blue),
        ),
        Span::raw("  filter: "),
        Span::styled(
            if app.filter_query.is_empty() {
                "*"
            } else {
                app.filter_query.as_str()
            },
            Style::default().fg(Color::Magenta),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Overview"));

    frame.render_widget(header, vertical[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(vertical[1]);

    let metric_height = body[0].height.saturating_sub(2) as usize;
    let metric_start = window_start(app.selected, app.metrics.len(), metric_height);
    let items = visible_list_items(
        &app.metrics,
        metric_start,
        metric_height,
        app.selected,
        |metric| format!("{} = {:.3}", metric.name, metric.value),
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

    let footer = Paragraph::new(format!(
        "{} | q quit | j/k move | h history view | t target picker | / filter | r reload now",
        app.status
    ))
    .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(footer, vertical[2]);

    if app.target_picker_open {
        render_target_picker(frame, app);
    } else if app.filter_input_open {
        render_filter_input(frame, app);
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

fn render_history(frame: &mut Frame, app: &App, area: Rect) {
    match app.history_view {
        HistoryView::Graph => render_history_chart(frame, app, area),
        HistoryView::Table => render_history_table(frame, app, area),
    }
}

fn render_history_chart(frame: &mut Frame, app: &App, area: Rect) {
    let history = app.selected_metric_history();
    if history.is_empty() {
        render_empty_history(frame, area, "History (Graph)");
        return;
    }

    let min = history.iter().copied().fold(f64::INFINITY, f64::min);
    let max = history.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (min, max) = if (max - min).abs() < f64::EPSILON {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    };

    let points: Vec<(f64, f64)> = history
        .iter()
        .enumerate()
        .map(|(index, value)| (index as f64, *value))
        .collect();
    let x_max = points.len().saturating_sub(1) as f64;
    let datasets = vec![Dataset::default()
        .name("value")
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&points)];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("History (Graph)"),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max.max(1.0)])
                .labels(vec![Line::from("old"), Line::from("now")]),
        )
        .y_axis(Axis::default().bounds([min, max]).labels(vec![
            Line::from(format!("{min:.3}")),
            Line::from(format!("{max:.3}")),
        ]));

    frame.render_widget(chart, area);
}

fn render_history_table(frame: &mut Frame, app: &App, area: Rect) {
    let history = app.selected_metric_history();
    if history.is_empty() {
        render_empty_history(frame, area, "History (Table)");
        return;
    }

    let visible_rows = area.height.saturating_sub(3) as usize;
    let start = history.len().saturating_sub(visible_rows.max(1));
    let rows: Vec<Row> = history
        .iter()
        .enumerate()
        .skip(start)
        .map(|(index, value)| {
            let age = history.len().saturating_sub(index + 1);
            let point = if age == 0 {
                String::from("now")
            } else {
                format!("-{age}")
            };
            Row::new(vec![Cell::from(point), Cell::from(format!("{value:.3}"))])
        })
        .collect();

    let table = Table::new(rows, [Constraint::Length(8), Constraint::Min(12)])
        .header(
            Row::new(vec!["Sample", "Value"]).style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("History (Table)"),
        )
        .column_spacing(1);

    frame.render_widget(table, area);
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
            let prefix = if start + offset == selected { "> " } else { "  " };
            ListItem::new(format!("{prefix}{}", render_item(item)))
        })
        .collect()
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
    let empty = Paragraph::new("no history yet")
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(empty, area);
}
