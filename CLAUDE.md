# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
cargo build                                    # Compile the TUI
cargo run                                      # Launch the terminal UI
cargo test -- --nocapture                      # Run tests with output
cargo fmt                                      # Format code
cargo clippy --all-targets --all-features     # Lint
npm run build                                  # Build release binary for npm distribution
```

CLI flags: `--set-openai-key <key>`, `--clear-openai-key`, `--version`, `--help`

## Architecture Overview

LearnChain is a terminal-based learning tool that parses AI coding session logs (from Codex CLI or Claude Code) and generates interactive quizzes using OpenAI's API.

### Core Components

- **App** (`main.rs`): Central state machine managing views, events, and the main event loop. Views are defined in `AppView` enum: Menu, Events, Learning, Config, Analytics.

- **Session Sources** (`session_sources/`): Modular session parsing with extensible architecture:
  - `mod.rs`: Defines `SessionSource` trait, `SessionEvent`, and `SessionLoad` types
  - `codex.rs`: `CodexCliSource` implementation for Codex CLI (`~/.codex/sessions/`)
  - `claude.rs`: `ClaudeCodeSource` implementation for Claude Code (`~/.claude/projects/`)

- **SessionManager** (`session_manager.rs`): Orchestrates session loading across multiple sources. Uses the `SessionSource` trait implementations from `session_sources/` module.

- **AiManager** (`ai_manager.rs`): Handles OpenAI API integration for quiz generation. Runs requests in a background thread with `mpsc` channels for async communication. Returns `StructuredLearningResponse` containing knowledge groups with quizzes.

- **View Managers** (`view_managers/`): Stateless managers that borrow `&mut App` to handle input and rendering for each view. Pattern: `XxxManager::new(&mut app).handle_key(key)`.

- **KnowledgeStore** (`knowledge_store.rs`): SQLite-based persistence for quiz attempts and analytics.

- **UiRenderer** (`ui_renderer.rs`): Ratatui-based terminal rendering, dispatches to view-specific render methods.

### Data Flow

1. SessionManager loads session logs on startup
2. OutputManager generates markdown summary from events
3. User triggers quiz generation via menu
4. AiManager sends summary to OpenAI, receives structured quiz data
5. LearningManager presents interactive quiz in TUI
6. KnowledgeStore records first attempts for analytics

### Configuration

User settings stored in `config/app_config.toml`, managed by `config.rs`. Includes OpenAI API key, session source selection, model preference, and artifact output toggle.

## Testing

Tests use `#[cfg(test)]` alongside modules. Test fixtures live in `test_fixtures/`. Use `tempfile` crate for tests requiring filesystem operations.

### Interactive TUI Testing

To validate TUI changes in a non-interactive environment, use tmux:

```bash
# Start app in detached tmux session
tmux new-session -d -s test 'cargo run'

# Send keystrokes (e.g., select menu option 3)
tmux send-keys -t test '3'

# Capture current screen output
tmux capture-pane -t test -p

# Send navigation keys
tmux send-keys -t test Down
tmux send-keys -t test Enter
tmux send-keys -t test BSpace

# Kill session when done
tmux kill-session -t test
```

This allows automated verification of TUI behavior without a real terminal.
