# Cranberry

## Purpose

This project develops a TUI dashboard for displaying Prometheus-format metrics.

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

- `cranberry.toml` can define the Prometheus HTTP endpoint.
- `cranberry.toml` can define initial display settings.
- `cranberry.toml` can define the automatic refresh interval in seconds.
