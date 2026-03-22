# Cranberry

## Purpose

This project develops a TUI dashboard for displaying metrics collected by Prometheus through the Prometheus HTTP API.

## Stack

- Language: Rust
- TUI framework: Ratatui

## Testing

- Tests are run with `cargo test`.
- The current test coverage is minimal.
- A parser unit test exists for Prometheus exposition-format input.
- A display-config unit test exists for initial selection and metric count limiting.
- TUI rendering and input handling tests are not yet implemented.

## Configuration

- `cranberry.toml` can define the Prometheus base URL.
- `cranberry.toml` can define initial display settings.
- `cranberry.toml` can define the automatic refresh interval in seconds.
- Targets are discovered dynamically from Prometheus.
- Metrics for the selected target are loaded dynamically from Prometheus.
