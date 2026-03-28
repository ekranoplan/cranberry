# Cranberry

## Purpose

This project develops a TUI dashboard for displaying metrics collected by Prometheus through the Prometheus HTTP API and logs collected from Loki.

## Stack

- Language: Rust
- TUI framework: Ratatui

## Testing

- Tests are run with `cargo test`.
- After code changes, run `cargo fmt`.
- After code changes, run `cargo clippy --all-targets --all-features`.
- The current test coverage is minimal.
- A parser unit test exists for Prometheus exposition-format input.
- A display-config unit test exists for initial selection and metric count limiting.
- TUI rendering and input handling tests are not yet implemented.

## Development

- Do not change the public API, CLI arguments, or external I/O behavior unless explicitly requested.
- Keep behavior-preserving changes small and focused unless a broader refactor is explicitly requested.
- Treat the Prometheus and Loki integrations as separate concerns when making changes.

## Configuration

- `cranberry.toml` can define the Prometheus base URL.
- `cranberry.toml` can define the Loki base URL and label settings.
- `cranberry.toml` can define initial display settings.
- `cranberry.toml` can define the automatic refresh interval in seconds.
- Targets are discovered dynamically from Prometheus.
- Metrics for the selected target are loaded dynamically from Prometheus.
- Loki hosts and log labels are discovered dynamically from Loki.
- If Loki is not configured, the logs screen should remain unavailable while the metrics screen continues to work normally.
- When changing Loki-related state transitions or refresh behavior, verify the logs screen behavior and the app state transitions with the existing tests.
