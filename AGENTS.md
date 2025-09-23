# Repository Guidelines

## Project Structure & Module Organization
The repository root contains `Cargo.toml` and `README.md`; Rust sources live in `src/`, with the entry point at `src/main.rs` defining the `App` TUI loop. Keep additional modules in `src/` using Rust's module system (`mod foo;`) and co-locate integration data or assets under a sibling `assets/` directory when needed. Integration tests belong in `tests/`, while snapshot fixtures should live in `tests/fixtures/` to avoid bundling them with application code.

## Build, Test, and Development Commands
- `cargo build` – compile the application and surface compiler warnings.
- `cargo run` – launch the Ratatui interface locally for manual testing.
- `cargo test` – execute unit and integration tests; add `-- --nocapture` to print logs.
- `cargo fmt` – apply `rustfmt` formatting; run before committing.
- `cargo clippy --all-targets --all-features` – lint with Clippy and receive actionable diagnostics.

## Coding Style & Naming Conventions
Use four-space indentation and Rust's standard snake_case for functions, modules, and files; types and enums should use PascalCase. Prefer expressive names that reflect the UI widget or state they manage (e.g., `status_panel`, `input_mode`). Keep functions small, favoring methods on `App` or domain structs. Always run `cargo fmt` and `cargo clippy` before opening a pull request to align with the Rust ecosystem's expectations.

## Testing Guidelines
Unit tests should accompany the module they exercise inside `#[cfg(test)]` blocks. For behavior that spans modules or renders the TUI, add integration tests in `tests/` using descriptive names like `app_handles_quit.rs`. Maintain meaningful assertions rather than relying solely on snapshot output, and target coverage for new branches or state transitions you introduce. Run `cargo test` locally before pushing.

## Commit & Pull Request Guidelines
There is no established history yet, so adopt short, present-tense commit messages (e.g., "Add app state transitions") and reference related issues with `#123` when applicable. Pull requests should summarize the change, call out UI impacts, and list manual test commands (`cargo run`, `cargo test`). Include screenshots or ASCII recordings if the TUI presentation changes. Ensure CI-critical commands (`cargo fmt`, `cargo clippy`, `cargo test`) succeed prior to requesting review.

## TUI-Specific Notes
When expanding the interface, centralize layout logic inside `App::render` and keep event handling in `App::on_key_event`. Document new key bindings in the on-screen help text to aid contributors verifying behavior via `cargo run`.
