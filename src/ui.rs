use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::App;

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
        Span::raw("  Prometheus TUI Dashboard"),
        Span::raw("  source: "),
        Span::styled(
            app.source_label.as_str(),
            Style::default().fg(Color::Green),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Overview"));

    frame.render_widget(header, vertical[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(vertical[1]);

    let items: Vec<ListItem> = app
        .metrics
        .iter()
        .enumerate()
        .map(|(index, metric)| {
            let prefix = if index == app.selected { "> " } else { "  " };
            ListItem::new(format!("{prefix}{} = {:.3}", metric.name, metric.value))
        })
        .collect();

    let metric_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Metrics"))
        .highlight_style(Style::default().fg(Color::Yellow));

    frame.render_widget(metric_list, body[0]);

    let detail_text = match app.selected_metric() {
        Some(metric) => {
            let labels = if metric.labels.is_empty() {
                String::from("none")
            } else {
                metric
                    .labels
                    .iter()
                    .map(|(key, value)| format!("{key}=\"{value}\""))
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            format!(
                "name: {}\nvalue: {}\n\nlabels:\n{}",
                metric.name, metric.value, labels
            )
        }
        None => String::from("no metrics loaded"),
    };

    let detail = Paragraph::new(detail_text)
        .block(Block::default().borders(Borders::ALL).title("Details"));
    frame.render_widget(detail, body[1]);

    let footer = Paragraph::new(format!(
        "{} | q quit | j/k move | r reload now",
        app.status
    ))
    .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(footer, vertical[2]);
}
