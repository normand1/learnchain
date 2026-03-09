# Changelog

All notable changes to this project will be documented in this file.

## [0.3.0] - 2026-03-09

### Added

- Added saved deep-dive generation for current or selected Codex sessions, including optional export to configured document repositories.
- Added library browsing and export flows for generated deep dives and quiz artifacts.
- Added bundled Codex deep-dive skill installation and a printable custom-command template for Codex users.
- Added richer Codex and Claude session ingestion plus deterministic session analytics and dashboard views.

### Changed

- Replaced the previous AI manager with Rig-backed LLM workflows for quiz and deep-dive generation.
- Expanded configuration and terminal UI flows for provider setup, document repositories, session selection, and learning history review.
- Updated the public documentation to cover deep dives, analytics, document exports, and Codex integration.

### Fixed

- Fixed the npm publish workflow so releases derive the expected package version and avoid publish-time 404 failures.
- Cleaned up release packaging, deep-dive defaults, document UI behavior, and configuration panel hierarchy.
