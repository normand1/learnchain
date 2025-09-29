# Repository Guidelines

Use this playbook to keep LearnChain's GenAI TUI predictable, debuggable, and easy to extend.

## Project Structure & Module Organization
The entry point lives in `src/main.rs`, which wires app state, views, and async workers; related helpers sit beside it (`src/config.rs`, `src/ai_manager.rs`, `src/markdown_rules.rs`). Persisted user settings are stored in `config/app_config.toml`; treat the config module as the single interface for load/save logic and default fallbacks. Keep integration fixtures under `tests/` and `tests/fixtures/`, runtime artefacts in `output/`, and reusable CLI helpers in `scripts/`. Reference runtime assets from `assets/` with workspace-relative paths so `cargo run` succeeds from the repo root.

## Build, Test, and Development Commands
- `cargo build` — compile the TUI and verify that the dependency graph is coherent.
- `cargo run` — launch the terminal UI to exercise menu flows, the “Configure defaults” view, and AI generation loops.
- `cargo test -- --nocapture` — execute unit and integration suites while preserving tracing output.
- `cargo fmt` / `cargo clippy --all-targets --all-features` — enforce formatting and lint rules prior to review.

## Coding Style & Naming Conventions
Use four-space indentation. Prefer expressive naming with Rust `snake_case` for modules and functions and `PascalCase` for types or traits. Keep behaviour in `App` methods or focused helpers; whenever you add key bindings, also refresh the status hints surfaced in menu, config, and learning views. Reserve comments for clarifying tricky control flow or invariants.

## Testing Guidelines
Add unit tests beside their modules with `#[cfg(test)]` and make assertions on concrete outcomes such as config normalization or markdown filtering. Reach for integration tests in `tests/` when validating session persistence, menu transitions, or config save/load loops. Run `cargo test -- --nocapture` after touching async tasks or persistence paths and note any timing nuances inline.

## Commit & Pull Request Guidelines
Write present-tense commit messages (for example, “Add config persistence”) and link related issues with `#123`. PR descriptions should flag UI impacts, list manual verification commands, and attach screenshots or ASCII casts when layouts shift. Before requesting review, confirm formatting, linting, tests, and config migrations in the checklist, and highlight risky assumptions early.

## Security & Configuration Tips
Keep OpenAI keys and other secrets out of version control; load them via environment variables or untracked files. Let the app manage `config/app_config.toml`, avoiding manual edits while the TUI is running. Always prefer workspace-relative paths to maintain portability across environments.
