# Cranberry

Cranberry is a Rust TUI dashboard for browsing metrics from Prometheus through the Prometheus HTTP API.

![Cranberry app screenshot](docs/ss.png)

Japanese README: [README-ja.md](README-ja.md)

## Run

```bash
cargo run
```

If `cranberry.toml` exists, it is loaded automatically.

You can also override the Prometheus base URL on the command line:

```bash
cargo run -- http://127.0.0.1:9090
```

Or choose a different config file:

```bash
cargo run -- --config /path/to/cranberry.toml
```

## Configuration

Example `cranberry.toml.sample`:

```toml
[prometheus]
base_url = "http://127.0.0.1:9090"

[display]
max_metrics = 20
initial_metric = "up"
refresh_secs = 15

[logging]
path = "cranberry.log"
level = "info"
```

Supported options:

- `prometheus.base_url`: Base URL for the Prometheus server, for example `http://127.0.0.1:9090`
- `display.max_metrics`: Optional cap for the metric list after target and text filtering
- `display.initial_metric`: Optional metric name to select initially
- `display.refresh_secs`: Automatic refresh interval in seconds
- `logging.path`: Log file path. Defaults to `cranberry.log`
- `logging.level`: Log verbosity. One of `trace`, `debug`, `info`, `warn`, `error`. Defaults to `info`

If `prometheus.base_url` is omitted, Cranberry starts with built-in sample metrics.

## Controls

- `q`: Quit
- `j` / `k`: Move selection
- `[` / `]`: Switch target
- `t`: Open target picker
- `/`: Open metric filter input
- `r`: Reload immediately
- `Esc`: Close target picker or filter input
- `Enter`: Apply target picker selection or close filter input
- `Backspace`: Delete one character in filter input
- `Ctrl-U`: Clear filter input
