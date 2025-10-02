# LearnChain

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

https://github.com/normand1/learnchain/assets/1002439/example-movie.mp4

**LearnChain** is a terminal-based learning tool that helps you learn to code while using generative AI to build projects. It analyzes your Codex or Claude session logs and surfaces interactive quizzes in a beautiful terminal UI, reinforcing concepts as you work.

## Features

- **Session Log Analysis**: Parse and learn from your AI-assisted coding sessions
- **Quiz Generation**: AI-powered quiz creation based on your actual coding patterns
- **Configuration Management**: Persistent settings stored in `config/app_config.toml`
- **Multi-platform Support**: Distributed via npm for easy installation across platforms
- **Interactive TUI**: Built with [Ratatui](https://ratatui.rs) for a polished terminal experience

## Installation

### Via npm (Recommended)

```bash
npm install -g learnchain
npx learnchain --help
```

### From Source

```bash
git clone https://github.com/yourusername/learnchain.git
cd learnchain
cargo build --release
./target/release/learnchain
```

## Quick Start

1. **Configure your OpenAI API key** (required for quiz generation):
   ```bash
   npx learnchain --set-openai-key sk-...
   ```

   Or configure it directly in the TUI: Config view → "OpenAI API key" → press Enter to edit

2. **Launch the terminal UI**:
   ```bash
   cargo run
   # or if installed via npm:
   npx learnchain
   ```

3. Navigate the menu to load session logs, configure defaults, and start learning!

## Development

### Prerequisites

- Rust (edition 2024)
- Node.js >= 16 (for npm distribution)
- Cargo

### Build Commands

```bash
# Compile the TUI
cargo build

# Run the application
cargo run

# Run tests with output
cargo test -- --nocapture

# Format and lint
cargo fmt
cargo clippy --all-targets --all-features

# Build release binary for npm distribution
npm run build
```

### Project Structure

```
learnchain/
├── src/
│   ├── main.rs              # Entry point and app state
│   ├── ai_manager.rs        # AI/LLM integration
│   ├── config.rs            # Configuration management
│   ├── session_manager.rs   # AI coding tool (Claude/Codex) log processing
│   ├── ui_renderer.rs       # Terminal UI rendering
│   └── view_managers/       # View-specific logic
├── config/                  # Runtime configuration
├── test_fixtures/           # Test fixtures
├── scripts/                 # Build and install helpers
└── dist/                    # npm distribution files
```

See [AGENTS.md](AGENTS.md) for detailed development guidelines.

## Configuration

LearnChain stores configuration in `config/app_config.toml`. Key settings include:

- OpenAI API key (required for quiz generation)
- Default session log paths
- UI preferences

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

See [AGENTS.md](AGENTS.md) for coding standards and testing guidelines.

## License

Copyright (c) Dave Norman <david.norman.w@gmail.com>

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Built with [Ratatui](https://ratatui.rs) - Rust TUI framework
