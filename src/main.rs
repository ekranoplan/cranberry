mod app;
mod config;
mod logging;
mod loki;
mod prometheus;
mod ui;

use std::env;
use std::error::Error;
use std::io;
use std::time::Duration;
use std::time::Instant;

use app::App;
use config::Config;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing::info;

fn main() -> Result<(), Box<dyn Error>> {
    let config = load_config(env::args().skip(1).collect())?;
    if let Err(err) = logging::init(&config.logging) {
        eprintln!("logging initialization failed: {err}");
    } else {
        info!(
            log_path = %config.logging.path,
            log_level = %config.logging.level,
            refresh_secs = config.display.refresh_secs.unwrap_or(15),
            prometheus_base_url = config.prometheus.base_url.as_deref().unwrap_or("sample"),
            "logging initialized"
        );
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_app(&mut terminal, config);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
) -> Result<(), Box<dyn Error>> {
    let refresh_interval = config
        .display
        .refresh_secs
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(15));
    info!(
        refresh_secs = refresh_interval.as_secs(),
        "starting application"
    );
    let mut app = App::with_loki(config.prometheus, config.loki, config.display);
    let mut last_reload = Instant::now();
    let mut last_log_poll = Instant::now();

    loop {
        if last_reload.elapsed() >= refresh_interval {
            info!(
                refresh_secs = refresh_interval.as_secs(),
                "automatic reload triggered"
            );
            app.reload();
            last_reload = Instant::now();
        }

        if app.is_logs_screen()
            && last_log_poll.elapsed() >= Duration::from_secs(app.log_poll_secs())
        {
            app.refresh_logs();
            last_log_poll = Instant::now();
        }

        terminal.draw(|frame| ui::render(frame, &app))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        if key.kind != KeyEventKind::Press {
            continue;
        }

        let action = if app.is_logs_screen() && app.log_filter_input_open {
            handle_log_filter_key(&mut app, key)
        } else if app.is_logs_screen() {
            handle_log_key(&mut app, key)
        } else if app.target_picker_open {
            handle_target_picker_key(&mut app, key)
        } else if app.filter_input_open {
            handle_filter_key(&mut app, key)
        } else {
            handle_main_key(&mut app, key)
        };

        match action {
            AppAction::Continue => {}
            AppAction::Reloaded => last_reload = Instant::now(),
            AppAction::LogsReloaded => last_log_poll = Instant::now(),
            AppAction::Quit => break,
        }
    }

    Ok(())
}

fn load_config(args: Vec<String>) -> Result<Config, Box<dyn Error>> {
    let mut config_path = String::from("cranberry.toml");
    let mut base_url_override = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let path = iter
                .next()
                .ok_or_else(|| String::from("missing path after --config"))?;
            config_path = path;
        } else {
            base_url_override = Some(arg);
        }
    }

    let mut config = if std::path::Path::new(&config_path).exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };

    if base_url_override.is_some() {
        config.prometheus.base_url = base_url_override;
    }

    if let Some(base_url) = &config.prometheus.base_url {
        info!(%base_url, "prometheus base url configured");
    } else {
        info!("using built-in sample metrics");
    }

    Ok(config)
}

enum AppAction {
    Continue,
    Reloaded,
    LogsReloaded,
    Quit,
}

fn handle_log_key(app: &mut App, key: KeyEvent) -> AppAction {
    match key.code {
        KeyCode::Esc => app.close_logs(),
        KeyCode::Char('/') => app.open_log_filter_input(),
        KeyCode::Tab | KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
            app.toggle_log_focus()
        }
        KeyCode::Down | KeyCode::Char('j') => app.next_log_option(),
        KeyCode::Up | KeyCode::Char('k') => app.previous_log_option(),
        KeyCode::Char('r') => {
            app.reload_logs_screen();
            return AppAction::LogsReloaded;
        }
        KeyCode::Char('q') => return AppAction::Quit,
        _ => {}
    }

    AppAction::Continue
}

fn handle_target_picker_key(app: &mut App, key: KeyEvent) -> AppAction {
    match key.code {
        KeyCode::Esc => app.close_target_picker(),
        KeyCode::Enter => app.picker_apply(),
        KeyCode::Down | KeyCode::Char('j') => app.picker_next(),
        KeyCode::Up | KeyCode::Char('k') => app.picker_previous(),
        KeyCode::Char('q') => return AppAction::Quit,
        _ => {}
    }

    AppAction::Continue
}

fn handle_filter_key(app: &mut App, key: KeyEvent) -> AppAction {
    match key.code {
        KeyCode::Esc | KeyCode::Enter => app.close_filter_input(),
        KeyCode::Backspace => app.pop_filter_char(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.clear_filter(),
        KeyCode::Char(ch) => app.push_filter_char(ch),
        _ => {}
    }

    AppAction::Continue
}

fn handle_log_filter_key(app: &mut App, key: KeyEvent) -> AppAction {
    match key.code {
        KeyCode::Esc | KeyCode::Enter => app.close_log_filter_input(),
        KeyCode::Backspace => app.pop_log_filter_char(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_log_filter()
        }
        KeyCode::Char(ch) => app.push_log_filter_char(ch),
        _ => {}
    }

    AppAction::Continue
}

fn handle_main_key(app: &mut App, key: KeyEvent) -> AppAction {
    match key.code {
        KeyCode::Char('q') => return AppAction::Quit,
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Up | KeyCode::Char('k') => app.previous(),
        KeyCode::Char(' ') => app.toggle_metric_selection(),
        KeyCode::Char('c') => app.clear_metric_selection(),
        KeyCode::Char('h') => app.toggle_history_view(),
        KeyCode::Char('[') => app.previous_target(),
        KeyCode::Char(']') => app.next_target(),
        KeyCode::Char('t') => app.open_target_picker(),
        KeyCode::Char('l') => {
            app.open_logs();
            return AppAction::LogsReloaded;
        }
        KeyCode::Char('/') => app.open_filter_input(),
        KeyCode::Char('r') => {
            info!("manual reload triggered");
            app.reload();
            return AppAction::Reloaded;
        }
        _ => {}
    }

    AppAction::Continue
}
