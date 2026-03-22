mod app;
mod config;
mod prometheus;
mod ui;

use std::env;
use std::io;
use std::time::Instant;
use std::time::Duration;

use app::App;
use config::Config;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config(env::args().skip(1).collect())?;
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
) -> Result<(), Box<dyn std::error::Error>> {
    let refresh_interval = config
        .display
        .refresh_secs
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(15));
    let mut app = App::new(config.prometheus.url, config.display);
    let mut last_reload = Instant::now();

    loop {
        if last_reload.elapsed() >= refresh_interval {
            app.reload();
            last_reload = Instant::now();
        }

        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Char('r') => {
                        app.reload();
                        last_reload = Instant::now();
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn load_config(args: Vec<String>) -> Result<Config, Box<dyn std::error::Error>> {
    let mut config_path = String::from("cranberry.toml");
    let mut source_url_override = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let path = iter
                .next()
                .ok_or_else(|| String::from("missing path after --config"))?;
            config_path = path;
        } else {
            source_url_override = Some(arg);
        }
    }

    let mut config = if std::path::Path::new(&config_path).exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };

    if source_url_override.is_some() {
        config.prometheus.url = source_url_override;
    }

    Ok(config)
}
